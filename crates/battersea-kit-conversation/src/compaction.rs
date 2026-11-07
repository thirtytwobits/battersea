//! Copyright (c) Scott A Dixon
//!
//! Compaction: the policy that bounds a transcript and the evaluation that decides whether
//! to compact it.
use crate::{Memory, TranscriptId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;

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
