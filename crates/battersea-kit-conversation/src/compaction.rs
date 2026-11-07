//! Copyright (c) Scott A Dixon
//!
//! Compaction: the policy that bounds a transcript and the evaluation that decides whether
//! to compact it.
use crate::{JobId, Memory, Port, TranscriptId};
use battersea_model::Clock;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};
use tokio::{sync::Notify, time::Instant};

/// The thresholds that bound a transcript. Each is optional; a flow overrides any of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Policy {
    /// Compact after a run once more turns than this follow the summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u64>,
    /// Compact after a run once the turns after the summary measure more tokens than this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Compact once the transcript has had no run for this long.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle: Option<Duration>,
}
impl Policy {
    /// This policy with each threshold `overrides` sets replaced.
    pub fn with_overrides(&self, overrides: &Policy) -> Policy {
        Policy {
            max_turns: overrides.max_turns.or(self.max_turns),
            max_tokens: overrides.max_tokens.or(self.max_tokens),
            idle: overrides.idle.or(self.idle),
        }
    }
}

/// What prompted an evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// A run on the transcript completed.
    AfterRun,
    /// The transcript's idle timer elapsed.
    Idle,
}

/// What an evaluation decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Compact,
    Hold,
}

/// The turns after the summary and their measured tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Measured {
    pub turns: u64,
    pub tokens: u64,
}

/// One evaluation of a policy against a transcript's memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolicyEvaluation {
    pub transcript: TranscriptId,
    pub trigger: Trigger,
    pub decision: Decision,
    pub measured: Measured,
    pub policy: Policy,
}

/// Decides whether to compact `memory`'s transcript. After a run, the decision is `compact`
/// when the turns after the summary exceed `max_turns` or their `tokens` exceed
/// `max_tokens`. On idle, it is `compact` when any turn follows the summary. Otherwise it
/// is `hold`.
pub fn evaluate<X>(
    policy: &Policy,
    trigger: Trigger,
    memory: &Memory<X>,
    tokens: u64,
) -> PolicyEvaluation {
    let turns = memory.turns.len() as u64;
    let compact = match trigger {
        Trigger::AfterRun => {
            policy.max_turns.is_some_and(|max| turns > max)
                || policy.max_tokens.is_some_and(|max| tokens > max)
        }
        Trigger::Idle => turns > 0,
    };
    PolicyEvaluation {
        transcript: memory.transcript.clone(),
        trigger,
        decision: if compact {
            Decision::Compact
        } else {
            Decision::Hold
        },
        measured: Measured { turns, tokens },
        policy: policy.clone(),
    }
}

/// Drives compaction outside flows: evaluates after a run, arms a transcript's idle timer on
/// the port's clock and evaluates once it elapses. It requests at most one job per transcript
/// until the application reports that job ended; it never activates a flow.
pub struct Runner<H: Port> {
    host: Arc<H>,
    clock: Arc<dyn Clock>,
    inner: Mutex<Inner>,
    changed: Notify,
}
#[derive(Default)]
struct Inner {
    /// The job requested for each transcript that has one open.
    requested: BTreeMap<TranscriptId, JobId>,
    /// When each armed transcript's idle timer elapses.
    deadlines: BTreeMap<TranscriptId, Instant>,
}
impl<H: Port> Runner<H> {
    /// The node name the runner measures and requests under.
    pub const NODE: &'static str = "conversation.compaction.runner";

    /// A runner over `host`, on the host's clock.
    pub fn new(host: Arc<H>) -> Self {
        let clock = host.clock();
        Self {
            host,
            clock,
            inner: Mutex::default(),
            changed: Notify::new(),
        }
    }
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// The job the runner has open for `transcript`.
    pub fn requested(&self, transcript: &TranscriptId) -> Option<JobId> {
        self.inner().requested.get(transcript).cloned()
    }
    /// Forgets `job`, so the runner may request another for its transcript. The application
    /// calls this when its port transitions the job to `completed` or `failed`.
    pub fn job_ended(&self, job: &JobId) {
        self.inner().requested.retain(|_, held| held != job);
    }
    /// Evaluates `transcript` with the `after_run` trigger, requests a job on `compact` unless
    /// one is open, and arms the idle timer where the policy sets `idle`.
    pub async fn after_run(
        &self,
        state: &mut H::State,
        transcript: &TranscriptId,
    ) -> Result<Option<JobId>, H::Error> {
        let policy = self.host.policy(state, transcript)?;
        if let Some(idle) = policy.idle {
            let deadline = self.clock.instant() + idle;
            self.inner().deadlines.insert(transcript.clone(), deadline);
            self.changed.notify_waiters();
        }
        self.evaluate(state, transcript, &policy, Trigger::AfterRun)
            .await
    }
    /// Completes when a transcript's idle timer elapses, naming the transcript; pending while
    /// none is armed.
    pub async fn idle_elapsed(&self) -> TranscriptId {
        loop {
            let notified = self.changed.notified();
            let next = self
                .inner()
                .deadlines
                .iter()
                .min_by_key(|(_, deadline)| **deadline)
                .map(|(transcript, deadline)| (transcript.clone(), *deadline));
            let Some((transcript, deadline)) = next else {
                notified.await;
                continue;
            };
            tokio::select! {
                () = self.clock.sleep_until(deadline) => {
                    let mut inner = self.inner();
                    if inner.deadlines.get(&transcript) == Some(&deadline) {
                        inner.deadlines.remove(&transcript);
                        return transcript;
                    }
                }
                () = notified => {}
            }
        }
    }
    /// Evaluates `transcript` with the `idle` trigger and requests a job on `compact` unless one
    /// is open.
    pub async fn idle(
        &self,
        state: &mut H::State,
        transcript: &TranscriptId,
    ) -> Result<Option<JobId>, H::Error> {
        let policy = self.host.policy(state, transcript)?;
        self.evaluate(state, transcript, &policy, Trigger::Idle)
            .await
    }
    async fn evaluate(
        &self,
        state: &mut H::State,
        transcript: &TranscriptId,
        policy: &Policy,
        trigger: Trigger,
    ) -> Result<Option<JobId>, H::Error> {
        if self.inner().requested.contains_key(transcript) {
            return Ok(None);
        }
        let memory = self.host.memory(state, transcript).await?;
        let engine = self.host.engine(state, Self::NODE)?;
        let tokens = crate::nodes::count(engine.as_ref(), crate::nodes::messages(&memory.turns))
            .await
            .map_err(|error| crate::nodes::provider_failed::<H::Error>(&error))?;
        let evaluation = evaluate(policy, trigger, &memory, tokens);
        if evaluation.decision != Decision::Compact {
            return Ok(None);
        }
        let job = self.host.request_compaction(state, evaluation).await?;
        self.inner()
            .requested
            .insert(transcript.clone(), job.clone());
        Ok(Some(job))
    }
}
