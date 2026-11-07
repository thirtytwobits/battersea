//! Copyright (c) Scott A Dixon
//!
//! The kit's conformance suite: plays its cases on the reference host and on an
//! application's host, reached through [`Port`] and [`Probe`] alone.
use crate::{
    reference, register_token_types, CallRecord, CompactionJob, Options, Port, Reservation,
    Summary, TranscriptId, Turn, KIT, MANIFEST,
};
use async_trait::async_trait;
use battersea_flow::registry::Registry;
pub use battersea_kit::conformance::{Divergence, Failure, Fixture, Outcome, Played, Side};
use battersea_kit::{
    conformance::{Case, PortAccess, Suite},
    ManualClock, ReferenceHost, ReferenceState,
};
use battersea_runtime::{ExecutionError, ExecutionEvent, RegistryBuilder, SchedulerState};
pub use reference::ScriptedResponse;
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

/// What the suite needs of a host beside its port: seeding, scripting and observation.
#[async_trait]
pub trait Probe: Port {
    /// The extension the suite's seeded records carry.
    fn extension(&self) -> Self::Extension;
    /// Creates `transcript` holding `turns` committed and `summary` as its memory. A turn
    /// without an id is assigned one.
    async fn seed(
        &self,
        state: &mut Self::State,
        transcript: TranscriptId,
        turns: Vec<Turn<Self::Extension>>,
        summary: Option<Summary<Self::Extension>>,
    ) -> Result<(), Self::Error>;
    /// Installs the responses the engine returns, in order.
    async fn script(
        &self,
        state: &mut Self::State,
        responses: Vec<ScriptedResponse>,
    ) -> Result<(), Self::Error>;
    /// The observable state.
    async fn snapshot(&self, state: &Self::State)
        -> Result<Snapshot<Self::Extension>, Self::Error>;
}

/// A transcript as a host holds it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptSnapshot<X> {
    pub turns: Vec<Turn<X>>,
    pub summary: Option<Summary<X>>,
}

/// A host's observable state: its transcripts, the run's staged turns, its jobs and the
/// run's call records, open reservations and deliveries.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot<X> {
    pub transcripts: BTreeMap<TranscriptId, TranscriptSnapshot<X>>,
    pub staged: Vec<Turn<X>>,
    pub jobs: Vec<CompactionJob<X>>,
    pub calls: Vec<CallRecord>,
    pub reservations: Vec<Reservation>,
    pub deliveries: Vec<String>,
}
impl<X: Serialize> Snapshot<X> {
    /// The snapshot as the suite compares it: extensions removed, and every turn, job and
    /// reservation id replaced by its order of appearance, since each host names them.
    pub fn comparable(&self) -> Result<Value, String> {
        let mut aliases = BTreeMap::new();
        let mut alias = |kind: &str, id: &str| {
            let next = aliases.len();
            aliases
                .entry(id.to_string())
                .or_insert_with(|| format!("{kind}#{next}"));
        };
        for turn in self
            .transcripts
            .values()
            .flat_map(|transcript| &transcript.turns)
            .chain(&self.staged)
        {
            if let Some(id) = &turn.id {
                alias("turn", id.as_str());
            }
        }
        for job in &self.jobs {
            alias("job", job.id.as_str());
        }
        for reservation in self.reservations.iter().chain(
            self.calls
                .iter()
                .filter_map(|call| call.reservation.as_ref()),
        ) {
            alias("reservation", &reservation.id);
        }
        let mut value = serde_json::to_value(self).map_err(|error| error.to_string())?;
        rewrite(&mut value, &aliases);
        Ok(value)
    }
}

/// Replaces aliased ids and removes `extension` keys throughout `value`.
fn rewrite(value: &mut Value, aliases: &BTreeMap<String, String>) {
    match value {
        Value::Object(map) => {
            map.remove("extension");
            for value in map.values_mut() {
                rewrite(value, aliases);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite(item, aliases);
            }
        }
        Value::String(text) => {
            if let Some(alias) = aliases.get(text) {
                *text = alias.clone();
            }
        }
        _ => {}
    }
}

/// A port operation a case performs before its activation.
#[derive(Debug, Clone)]
pub enum Op {
    /// Creates a transcript with committed turns and a summary.
    Seed {
        transcript: TranscriptId,
        turns: Vec<Turn<()>>,
        summary: Option<Summary<()>>,
    },
    /// Installs the engine's responses.
    Script { responses: Vec<ScriptedResponse> },
}

/// The kit's port access: applies [`Op`]s through [`Probe`] and observes its snapshot.
pub struct Access;
impl<H: Probe> PortAccess<H, Op> for Access {
    async fn prepare(&self, host: &H, state: &mut H::State, operation: &Op) -> Result<(), String> {
        let message = |error: H::Error| error.message().to_string();
        match operation {
            Op::Seed {
                transcript,
                turns,
                summary,
            } => {
                let turns = turns
                    .iter()
                    .map(|turn| Turn {
                        id: turn.id.clone(),
                        role: turn.role,
                        content: turn.content.clone(),
                        created_at: turn.created_at,
                        revision: turn.revision,
                        extension: host.extension(),
                    })
                    .collect();
                let summary = summary.as_ref().map(|summary| Summary {
                    text: summary.text.clone(),
                    through: summary.through.clone(),
                    job: summary.job.clone(),
                    extension: host.extension(),
                });
                host.seed(state, transcript.clone(), turns, summary)
                    .await
                    .map_err(message)
            }
            Op::Script { responses } => {
                host.script(state, responses.clone()).await.map_err(message)
            }
        }
    }
    async fn observe(&self, host: &H, state: &H::State) -> Result<Value, String> {
        host.snapshot(state)
            .await
            .map_err(|error| error.message().to_string())?
            .comparable()
    }
}

/// The transcript every case's run is on.
pub const TRANSCRIPT: &str = "conformance";

/// Registers the kit's token types with the extension schema `true`, and the shared
/// vocabulary the kit references, as the generic kit registers it.
pub fn register_for_suite(registry: &mut Registry) -> Result<(), String> {
    for name in battersea_kit::SHARED_TOKEN_TYPES {
        registry
            .register_token_type(name, json!(true))
            .map_err(|error| error.to_string())?;
    }
    register_token_types(
        registry,
        &Options {
            extension_schema: json!(true),
        },
    )
}

/// The kit's cases.
pub fn cases() -> Vec<Case<Op>> {
    vec![]
}

/// The suite: the kit, its manifest and its cases.
pub fn suite() -> Suite<Op> {
    Suite {
        kit: &KIT,
        manifests: vec![(crate::NAME, MANIFEST)],
        register_token_types: register_for_suite,
        cases: cases(),
    }
}

/// The reference host as the suite's oracle.
pub struct ReferenceFixture;
impl Fixture for ReferenceFixture {
    type Host = ReferenceHost<reference::State>;
    fn host(&self, clock: Arc<ManualClock>) -> Self::Host {
        reference::host(RegistryBuilder::default().build(), clock)
    }
    fn state(
        &self,
        host: &Self::Host,
        scheduler: SchedulerState,
    ) -> ReferenceState<reference::State> {
        host.state(
            scheduler,
            reference::State::new(TRANSCRIPT, host.clock().clone()),
        )
    }
    fn events(&self, host: &Self::Host, run_id: &str) -> Vec<ExecutionEvent> {
        host.run_events(run_id)
    }
}

/// Plays every case on the reference host and on `fixture`'s host and reports the first
/// divergence.
pub async fn run<F: Fixture>(fixture: &F) -> Result<(), Failure>
where
    F::Host: Probe,
{
    battersea_kit::conformance::run(&suite(), &ReferenceFixture, fixture, &Access).await
}
