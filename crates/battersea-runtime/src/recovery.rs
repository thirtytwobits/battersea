//! Versioned checkpoints and revision-fenced effect/host-commit publication.
use crate::RunIdentity;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

pub const JOURNAL_VERSION: u32 = 1;
pub const MAX_JOURNAL_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_EFFECTS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionedState {
    pub version: u32,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryPhase {
    Executing,
    ExecutionComplete,
    HostCommit { intent: Value },
    Committed { result: Value },
    Abandoned { actor: String, evidence: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectState {
    Intent,
    Completed { result: Value },
    NotApplied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectRecord {
    pub node: String,
    pub operation: String,
    pub input: Value,
    pub state: EffectState,
}
impl EffectRecord {
    pub fn result(&self) -> Option<&Value> {
        match &self.state {
            EffectState::Completed { result } => Some(result),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionRecord {
    pub effect_id: String,
    pub actor: String,
    pub evidence: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRecord {
    pub version: u32,
    pub revision: u64,
    pub identity: RunIdentity,
    pub checkpoint: VersionedState,
    pub accepted_checkpoint: VersionedState,
    pub phase: RecoveryPhase,
    pub effects: BTreeMap<String, EffectRecord>,
    pub resolutions: Vec<ResolutionRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    Missing,
    Conflict,
    Incompatible(String),
    Invalid(String),
    Storage(String),
    UnresolvedEffects(Vec<String>),
    CommitPending,
    Terminal,
    /// A failed write may have committed. Reopen storage before taking another action.
    Poisoned,
}
impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("Execution journal is unavailable."),
            Self::Conflict => f.write_str("Execution journal revision changed."),
            Self::Incompatible(message) => write!(f, "Incompatible execution journal: {message}"),
            Self::Invalid(message) => write!(f, "Invalid recovery operation: {message}"),
            Self::Storage(message) => write!(f, "Execution journal storage failed: {message}"),
            Self::UnresolvedEffects(ids) => {
                write!(f, "Effects require explicit resolution: {ids:?}")
            }
            Self::CommitPending => f.write_str("Host commit requires reconciliation."),
            Self::Terminal => f.write_str("Execution already has a terminal outcome."),
            Self::Poisoned => f.write_str("Reopen the journal after its failed write."),
        }
    }
}
impl std::error::Error for RecoveryError {}

/// The host implements atomic, durable compare-and-exchange and exclusive execution
/// ownership. Loading is read-only, including for unsupported records. A write error
/// may follow a durable write; the caller must inspect storage before continuing.
pub trait JournalStore: Send + Sync {
    fn load(&self, run_id: &str) -> Result<Option<JournalRecord>, RecoveryError>;
    fn compare_exchange(
        &self,
        run_id: &str,
        expected_revision: Option<u64>,
        next: &JournalRecord,
    ) -> Result<(), RecoveryError>;
}

pub struct Journal {
    store: Arc<dyn JournalStore>,
    record: JournalRecord,
    poisoned: bool,
}
impl std::fmt::Debug for Journal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Journal")
            .field("identity", &self.record.identity)
            .field("revision", &self.record.revision)
            .field("phase", &self.record.phase)
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}
impl JournalRecord {
    pub fn validate(&self) -> Result<(), RecoveryError> {
        if self.version != JOURNAL_VERSION {
            return Err(RecoveryError::Incompatible(format!(
                "version {}",
                self.version
            )));
        }
        if self.revision == 0
            || self.checkpoint.version == 0
            || self.accepted_checkpoint.version == 0
        {
            return Err(RecoveryError::Invalid(
                "Zero revision or state version.".into(),
            ));
        }
        for value in [
            &self.identity.run_id,
            &self.identity.flow_key,
            &self.identity.graph_revision,
            &self.identity.catalogue_revision,
            &self.identity.configuration_revision,
        ] {
            nonempty(value)?;
        }
        if self.effects.len() > MAX_EFFECTS || self.resolutions.len() > MAX_EFFECTS {
            return Err(RecoveryError::Invalid(
                "Effect journal limit exceeded.".into(),
            ));
        }
        if matches!(
            self.phase,
            RecoveryPhase::ExecutionComplete
                | RecoveryPhase::HostCommit { .. }
                | RecoveryPhase::Committed { .. }
        ) && self
            .effects
            .values()
            .any(|effect| matches!(effect.state, EffectState::Intent))
        {
            return Err(RecoveryError::Invalid(
                "Completion contains unresolved effects.".into(),
            ));
        }
        crate::pump::measure(self, MAX_JOURNAL_BYTES)
            .map_err(|error| RecoveryError::Invalid(error.to_string()))?;
        Ok(())
    }
}

impl Journal {
    pub fn accept(
        store: Arc<dyn JournalStore>,
        identity: RunIdentity,
        checkpoint: VersionedState,
    ) -> Result<Self, RecoveryError> {
        let record = JournalRecord {
            version: JOURNAL_VERSION,
            revision: 1,
            identity,
            accepted_checkpoint: checkpoint.clone(),
            checkpoint,
            phase: RecoveryPhase::Executing,
            effects: BTreeMap::new(),
            resolutions: Vec::new(),
        };
        record.validate()?;
        store.compare_exchange(&record.identity.run_id, None, &record)?;
        Ok(Self {
            store,
            record,
            poisoned: false,
        })
    }

    pub fn open(store: Arc<dyn JournalStore>, run_id: &str) -> Result<Self, RecoveryError> {
        let record = store.load(run_id)?.ok_or(RecoveryError::Missing)?;
        record.validate()?;
        if record.identity.run_id != run_id {
            return Err(RecoveryError::Invalid("Run identity mismatch.".into()));
        }
        Ok(Self {
            store,
            record,
            poisoned: false,
        })
    }

    pub fn record(&self) -> &JournalRecord {
        &self.record
    }

    pub fn validate_identity(&self, expected: &RunIdentity) -> Result<(), RecoveryError> {
        if self.record.identity == *expected {
            Ok(())
        } else {
            Err(RecoveryError::Incompatible(
                "Accepted identity or pinned revisions changed.".into(),
            ))
        }
    }

    pub fn require_resumable(&self) -> Result<(), RecoveryError> {
        self.require_executing()?;
        let unresolved = self
            .record
            .effects
            .iter()
            .filter(|(_, effect)| matches!(effect.state, EffectState::Intent))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        if unresolved.is_empty() {
            Ok(())
        } else {
            Err(RecoveryError::UnresolvedEffects(unresolved))
        }
    }

    /// Call only after acquiring the host's exclusive execution ownership.
    pub fn claim_resume(&mut self, expected_revision: u64) -> Result<(), RecoveryError> {
        self.require_resumable()?;
        if self.record.revision != expected_revision {
            return Err(RecoveryError::Conflict);
        }
        self.publish(self.record.clone())
    }

    pub fn begin_effect(
        &mut self,
        node: &str,
        operation: &str,
        input: Value,
    ) -> Result<String, RecoveryError> {
        self.require_executing()?;
        nonempty(node)?;
        nonempty(operation)?;
        if self.record.effects.len() >= MAX_EFFECTS {
            return Err(RecoveryError::Invalid(
                "Effect journal limit exceeded.".into(),
            ));
        }
        let mut next = self.record.clone();
        let id = format!("{}:{}", next.identity.run_id, next.revision);
        if next
            .effects
            .insert(
                id.clone(),
                EffectRecord {
                    node: node.into(),
                    operation: operation.into(),
                    input,
                    state: EffectState::Intent,
                },
            )
            .is_some()
        {
            return Err(RecoveryError::Invalid("Duplicate effect identity.".into()));
        }
        self.publish(next)?;
        Ok(id)
    }

    /// Publish state and the results that justify it in one storage transaction.
    pub fn checkpoint(
        &mut self,
        checkpoint: VersionedState,
        results: Vec<(String, Value)>,
    ) -> Result<(), RecoveryError> {
        self.require_executing()?;
        let mut next = self.record.clone();
        for (id, result) in results {
            let effect = next
                .effects
                .get_mut(&id)
                .ok_or_else(|| RecoveryError::Invalid("Unknown effect.".into()))?;
            if !matches!(effect.state, EffectState::Intent) {
                return Err(RecoveryError::Invalid("Effect already resolved.".into()));
            }
            effect.state = EffectState::Completed { result };
        }
        if next
            .effects
            .values()
            .any(|effect| matches!(effect.state, EffectState::Intent))
        {
            return Err(RecoveryError::Invalid(
                "Checkpoint has unresolved effects.".into(),
            ));
        }
        next.checkpoint = checkpoint;
        self.publish(next)
    }

    pub fn resolve_not_applied(
        &mut self,
        effect_id: &str,
        actor: &str,
        evidence: &str,
    ) -> Result<(), RecoveryError> {
        self.require_executing()?;
        nonempty(actor)?;
        nonempty(evidence)?;
        let mut next = self.record.clone();
        let effect = next
            .effects
            .get_mut(effect_id)
            .ok_or_else(|| RecoveryError::Invalid("Unknown effect.".into()))?;
        if !matches!(effect.state, EffectState::Intent) {
            return Err(RecoveryError::Invalid("Effect already resolved.".into()));
        }
        effect.state = EffectState::NotApplied;
        next.resolutions.push(ResolutionRecord {
            effect_id: effect_id.into(),
            actor: actor.into(),
            evidence: evidence.into(),
            revision: next
                .revision
                .checked_add(1)
                .ok_or(RecoveryError::Conflict)?,
        });
        self.publish(next)
    }

    pub fn execution_complete(&mut self, checkpoint: VersionedState) -> Result<(), RecoveryError> {
        self.require_resumable()?;
        let mut next = self.record.clone();
        next.checkpoint = checkpoint;
        next.phase = RecoveryPhase::ExecutionComplete;
        self.publish(next)
    }

    pub fn begin_host_commit(&mut self, intent: Value) -> Result<(), RecoveryError> {
        self.require_healthy()?;
        if !matches!(self.record.phase, RecoveryPhase::ExecutionComplete) {
            return Err(RecoveryError::Invalid(
                "Graph execution must complete before host commit.".into(),
            ));
        }
        let mut next = self.record.clone();
        next.phase = RecoveryPhase::HostCommit { intent };
        self.publish(next)
    }

    /// The host has verified its durable destination, including after restart.
    pub fn host_committed(&mut self, result: Value) -> Result<(), RecoveryError> {
        self.require_healthy()?;
        if !matches!(self.record.phase, RecoveryPhase::HostCommit { .. }) {
            return Err(RecoveryError::Invalid(
                "Host commit intent is required.".into(),
            ));
        }
        let mut next = self.record.clone();
        next.phase = RecoveryPhase::Committed { result };
        self.publish(next)
    }

    pub fn abandon(&mut self, actor: &str, evidence: &str) -> Result<(), RecoveryError> {
        self.require_healthy()?;
        match self.record.phase {
            RecoveryPhase::Executing | RecoveryPhase::ExecutionComplete => {}
            RecoveryPhase::HostCommit { .. } => return Err(RecoveryError::CommitPending),
            RecoveryPhase::Committed { .. } | RecoveryPhase::Abandoned { .. } => {
                return Err(RecoveryError::Terminal)
            }
        }
        nonempty(actor)?;
        nonempty(evidence)?;
        let mut next = self.record.clone();
        next.phase = RecoveryPhase::Abandoned {
            actor: actor.into(),
            evidence: evidence.into(),
        };
        self.publish(next)
    }

    fn require_healthy(&self) -> Result<(), RecoveryError> {
        if self.poisoned {
            Err(RecoveryError::Poisoned)
        } else {
            Ok(())
        }
    }
    fn require_executing(&self) -> Result<(), RecoveryError> {
        self.require_healthy()?;
        match self.record.phase {
            RecoveryPhase::Executing => Ok(()),
            RecoveryPhase::ExecutionComplete | RecoveryPhase::HostCommit { .. } => {
                Err(RecoveryError::CommitPending)
            }
            RecoveryPhase::Committed { .. } | RecoveryPhase::Abandoned { .. } => {
                Err(RecoveryError::Terminal)
            }
        }
    }
    fn publish(&mut self, mut next: JournalRecord) -> Result<(), RecoveryError> {
        self.require_healthy()?;
        next.revision = self
            .record
            .revision
            .checked_add(1)
            .ok_or(RecoveryError::Conflict)?;
        next.validate()?;
        match self
            .store
            .compare_exchange(&next.identity.run_id, Some(self.record.revision), &next)
        {
            Ok(()) => {
                self.record = next;
                Ok(())
            }
            Err(error) => {
                self.poisoned = true;
                Err(error)
            }
        }
    }
}

fn nonempty(value: &str) -> Result<(), RecoveryError> {
    if value.trim().is_empty() {
        Err(RecoveryError::Invalid(
            "An identity or evidence is empty.".into(),
        ))
    } else {
        Ok(())
    }
}
