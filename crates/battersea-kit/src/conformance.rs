//! Copyright (c) Scott A Dixon
//!
//! The differential conformance harness of `contracts/kits.md`. A kit plays each case on
//! its reference host and on the application's host, both on a [`ManualClock`] the suite
//! owns, and reports the first divergence in outcome or execution events.
use crate::Kit;
use battersea_flow::{catalog::Catalog, dsl::parse_flow_dsl, registry::Registry};
use battersea_model::ManualClock;
use battersea_runtime::{
    ActivationHost, ExecutionError, ExecutionEvent, ExecutionHost, SchedulerState,
};
use futures_util::future::{select, Either};
use serde_json::Value;
use std::{
    collections::HashMap,
    pin::pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// The clock reading every case starts from, in milliseconds since the Unix epoch.
pub const EPOCH_MS: u64 = 1_700_000_000_000;

/// One case: a flow in text source, the activation, the clock steps taken during the run
/// and the port operations performed before it.
#[derive(Debug, Clone)]
pub struct Case<Op = ()> {
    /// Names the case in a report and is the run id on both hosts.
    pub name: String,
    pub flow: String,
    /// The activated node.
    pub node: String,
    pub values: HashMap<String, Value>,
    /// Each step advances the clock once the activation waits on a timer.
    pub steps: Vec<Duration>,
    pub before: Vec<Op>,
}

/// What the suite composes on every host: the kit, the manifests of its catalogue, its
/// token types and its cases.
pub struct Suite<Op = ()> {
    pub kit: &'static Kit,
    /// The manifests of the catalogue, the kit's first.
    pub manifests: Vec<(&'static str, &'static str)>,
    pub register_token_types: fn(&mut Registry) -> Result<(), String>,
    pub cases: Vec<Case<Op>>,
}

/// How the suite reaches a host.
pub trait Fixture {
    type Host: ActivationHost;
    /// A host with no state, running on `clock`.
    fn host(&self, clock: Arc<ManualClock>) -> Self::Host;
    /// The host's run state around `scheduler`, with no kit state.
    fn state(
        &self,
        host: &Self::Host,
        scheduler: SchedulerState,
    ) -> <Self::Host as ExecutionHost>::State;
    /// The execution events the host recorded for `run_id`.
    fn events(&self, host: &Self::Host, run_id: &str) -> Vec<ExecutionEvent>;
}

/// How an activation ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Failed {
        code: String,
        reason: Option<String>,
        message: String,
    },
}
impl Outcome {
    fn from_result<E: ExecutionError>(result: Result<(), E>) -> Self {
        match result {
            Ok(()) => Self::Completed,
            Err(error) => Self::Failed {
                code: error.code().into(),
                reason: error.reason_code().map(Into::into),
                message: error.message().into(),
            },
        }
    }
    /// Equal outcomes; a failure's message is the host's own wording and is not compared.
    pub fn agrees_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Completed, Self::Completed) => true,
            (
                Self::Failed { code, reason, .. },
                Self::Failed {
                    code: other_code,
                    reason: other_reason,
                    ..
                },
            ) => code == other_code && reason == other_reason,
            _ => false,
        }
    }
}

/// One case played on one host.
#[derive(Debug, Clone)]
pub struct Played {
    pub outcome: Outcome,
    /// The host's execution events for the run, as JSON.
    pub events: Vec<Value>,
    /// Clock steps the activation never waited for.
    pub unused_steps: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Reference,
    Candidate,
}
impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Reference => "reference host",
            Self::Candidate => "candidate host",
        })
    }
}

#[derive(Debug, Clone)]
pub enum Divergence {
    /// The case could not be set up on `side`.
    Setup { side: Side, message: String },
    /// `side` finished with clock steps left: the case expects a wait the host never made.
    UnusedClockSteps { side: Side, remaining: usize },
    Outcome {
        reference: Outcome,
        candidate: Outcome,
    },
    /// The first event that differs, or the end of one host's events.
    Event {
        index: usize,
        reference: Option<Value>,
        candidate: Option<Value>,
    },
}

/// The first divergence, naming its case.
#[derive(Debug, Clone)]
pub struct Failure {
    pub case: String,
    pub divergence: Box<Divergence>,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "case {:?}: ", self.case)?;
        match &*self.divergence {
            Divergence::Setup { side, message } => write!(f, "{side} setup failed: {message}"),
            Divergence::UnusedClockSteps { side, remaining } => {
                write!(f, "{side} left {remaining} clock step(s) unused")
            }
            Divergence::Outcome {
                reference,
                candidate,
            } => write!(f, "outcome {candidate:?} differs from {reference:?}"),
            Divergence::Event {
                index,
                reference,
                candidate,
            } => write!(
                f,
                "event {index} {} differs from {}",
                candidate
                    .as_ref()
                    .map_or("missing".to_string(), Value::to_string),
                reference
                    .as_ref()
                    .map_or("missing".to_string(), Value::to_string)
            ),
        }
    }
}
impl std::error::Error for Failure {}

/// Plays `case` on the fixture's host. `prepare` performs each port operation of the case
/// before the activation.
pub async fn play<Op, F: Fixture>(
    suite: &Suite<Op>,
    fixture: &F,
    case: &Case<Op>,
    prepare: impl AsyncFn(&F::Host, &mut <F::Host as ExecutionHost>::State, &Op) -> Result<(), String>,
) -> Result<Played, String> {
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let host = fixture.host(clock.clone());
    let mut registry = Registry::default();
    (suite.register_token_types)(&mut registry)?;
    for id in host.handlers().ids() {
        registry
            .register_handler(id)
            .map_err(|error| error.to_string())?;
    }
    let catalog =
        Catalog::from_manifests(&suite.manifests, registry).map_err(|error| error.to_string())?;
    let catalog = host.handlers().bind_catalog(catalog)?;
    let flow = parse_flow_dsl(&case.flow).map_err(|error| error.to_string())?;
    let validation = catalog.validate(&flow);
    if !validation.valid {
        return Err(format!("flow is invalid: {:?}", validation.issues));
    }
    let scheduler = SchedulerState::new(case.name.clone(), &flow, catalog.definitions().clone())
        .map_err(|issues| format!("scheduler refused the flow: {issues:?}"))?;
    let mut state = fixture.state(&host, scheduler);
    for operation in &case.before {
        prepare(&host, &mut state, operation).await?;
    }
    let taken = AtomicUsize::new(0);
    let stepper = async {
        for step in &case.steps {
            clock.timer_within(Duration::MAX).await;
            clock.advance(*step);
            taken.fetch_add(1, Ordering::SeqCst);
        }
    };
    let activation = pin!(host.run_activation(
        &mut state,
        &case.node,
        case.values.clone(),
        CancellationToken::new(),
    ));
    let result = match select(activation, pin!(stepper)).await {
        Either::Left((result, _)) => result,
        Either::Right(((), activation)) => activation.await,
    };
    let events = fixture
        .events(&host, &case.name)
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(Played {
        outcome: Outcome::from_result(result),
        events,
        unused_steps: case.steps.len() - taken.load(Ordering::SeqCst),
    })
}

/// Plays every case on both hosts and reports the first divergence.
pub async fn run<Op, R: Fixture, C: Fixture>(
    suite: &Suite<Op>,
    reference: &R,
    candidate: &C,
    prepare_reference: impl AsyncFn(
        &R::Host,
        &mut <R::Host as ExecutionHost>::State,
        &Op,
    ) -> Result<(), String>,
    prepare_candidate: impl AsyncFn(
        &C::Host,
        &mut <C::Host as ExecutionHost>::State,
        &Op,
    ) -> Result<(), String>,
) -> Result<(), Failure> {
    for case in &suite.cases {
        let failure = |divergence| Failure {
            case: case.name.clone(),
            divergence: Box::new(divergence),
        };
        let expected = play(suite, reference, case, &prepare_reference)
            .await
            .map_err(|message| {
                failure(Divergence::Setup {
                    side: Side::Reference,
                    message,
                })
            })?;
        let actual = play(suite, candidate, case, &prepare_candidate)
            .await
            .map_err(|message| {
                failure(Divergence::Setup {
                    side: Side::Candidate,
                    message,
                })
            })?;
        for (side, played) in [(Side::Reference, &expected), (Side::Candidate, &actual)] {
            if played.unused_steps > 0 {
                return Err(failure(Divergence::UnusedClockSteps {
                    side,
                    remaining: played.unused_steps,
                }));
            }
        }
        if !expected.outcome.agrees_with(&actual.outcome) {
            return Err(failure(Divergence::Outcome {
                reference: expected.outcome,
                candidate: actual.outcome,
            }));
        }
        let length = expected.events.len().max(actual.events.len());
        for index in 0..length {
            let reference = expected.events.get(index);
            let candidate = actual.events.get(index);
            if reference != candidate {
                return Err(failure(Divergence::Event {
                    index,
                    reference: reference.cloned(),
                    candidate: candidate.cloned(),
                }));
            }
        }
    }
    Ok(())
}
