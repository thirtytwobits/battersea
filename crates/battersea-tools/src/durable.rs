//! Copyright (c) Scott A Dixon
//! Revision-fenced approval decisions, dispatch intents and retained tool results.
use crate::{registry::PreparedCall, Tool, ToolRegistry};
use async_trait::async_trait;
use battersea_model::{
    configuration_digest, EngineAdapterRequestError, EngineLocalToolCall,
    EngineLocalToolDefinition, EngineLocalToolExecutor, EngineLocalToolResult, ErrorKind,
    ToolCallEvidence, ToolCallOutcome, ToolCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::Write,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::{timeout, timeout_at, Instant};

pub const TOOL_APPROVAL_JOURNAL_VERSION: u32 = 1;
const MAX_CALLS: usize = 4096;
const MAX_BYTES: usize = battersea_model::payload::PROVIDER_PAYLOAD_BYTES;

/// Durable identity and absolute expiry supplied by the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableToolApprovalScope {
    pub run_id: String,
    pub binding_revision: String,
    pub expires_at_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolApprovalJournalLimits {
    pub max_calls: usize,
    /// Complete encoded journal, including definitions, arguments, results and metadata.
    pub max_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalAction {
    Allow,
    Deny,
    Cancel,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredToolDecision {
    pub request_revision: String,
    pub action: ToolApprovalAction,
    /// The host authenticates this identity.
    pub actor: String,
    pub at_unix_ms: u64,
}

/// A dispatch intent remains unresolved until publication or explicit reconciliation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoredToolCallState {
    Pending,
    Approved,
    Denied,
    Cancelled,
    Intent,
    Completed { result: EngineLocalToolResult },
    Failed { error: EngineAdapterRequestError },
    Abandoned,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolApprovalReconciliation {
    NotApplied,
    Completed { result: EngineLocalToolResult },
    Abandon,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolApprovalResolution {
    pub actor: String,
    pub evidence: String,
    pub revision: u64,
    pub at_unix_ms: u64,
    pub resolution: ToolApprovalReconciliation,
}

/// Host-only durable state. Debug formatting omits definitions, arguments and result values.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredToolCall {
    pub tool: Tool,
    pub call: EngineLocalToolCall,
    pub request_revision: String,
    pub decision: Option<StoredToolDecision>,
    pub state: StoredToolCallState,
    pub resolutions: Vec<ToolApprovalResolution>,
}
impl std::fmt::Debug for StoredToolCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredToolCall")
            .field("request_revision", &self.request_revision)
            .field("resolutions", &self.resolutions.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolApprovalJournal {
    pub version: u32,
    pub revision: u64,
    pub scope: DurableToolApprovalScope,
    pub limits: ToolApprovalJournalLimits,
    pub updated_at_unix_ms: u64,
    pub calls: BTreeMap<String, StoredToolCall>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolApprovalStoreError {
    Conflict,
    Unavailable,
}

/// Host-owned storage with atomic, durable compare-and-exchange.
///
/// Loads are read-only and must bound input before decoding it. Successful writes are
/// durable; errors or cancelled writes may have committed. The store must fence revisions
/// atomically across all handles and processes. The host fences execution ownership before
/// reconciling an intent, so its original executor can no longer perform an effect.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait ToolApprovalStore: Send + Sync {
    async fn load(
        &self,
        run_id: &str,
        max_bytes: usize,
    ) -> Result<Option<ToolApprovalJournal>, ToolApprovalStoreError>;
    async fn compare_exchange(
        &self,
        run_id: &str,
        expected_revision: Option<u64>,
        next: &ToolApprovalJournal,
    ) -> Result<(), ToolApprovalStoreError>;
}

fn failure(code: ToolCode, message: &str) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new("tools", message, ErrorKind::InvalidRequest).with_tool_code(code)
}
fn invalid() -> EngineAdapterRequestError {
    failure(
        ToolCode::InvalidToolApproval,
        "Invalid or incompatible tool approval journal.",
    )
}
fn conflict() -> EngineAdapterRequestError {
    failure(
        ToolCode::ToolApprovalConflict,
        "Tool approval revision changed.",
    )
}
fn now_ms() -> Result<u64, EngineAdapterRequestError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid())?
        .as_millis()
        .try_into()
        .map_err(|_| invalid())
}
fn measure(value: &impl Serialize, limit: usize) -> Result<(), EngineAdapterRequestError> {
    struct Counter(usize);
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_sub(bytes.len())
                .ok_or_else(|| std::io::Error::other("capacity"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(limit), value).map_err(|_| {
        failure(
            ToolCode::ToolApprovalCapacity,
            "Tool approval journal capacity exceeded.",
        )
    })
}
fn request_revision(
    scope: &DurableToolApprovalScope,
    tool: &Tool,
    call: &EngineLocalToolCall,
) -> String {
    configuration_digest(&json!({"scope":scope, "tool":tool, "call":call}))
}
fn evidence(call: &EngineLocalToolCall, outcome: ToolCallOutcome) -> ToolCallEvidence {
    ToolCallEvidence {
        call_id: call.id.clone(),
        tool_name: call.name.clone(),
        outcome,
        request_id: None,
    }
}
fn checked_limits(limits: ToolApprovalJournalLimits) -> Result<(), EngineAdapterRequestError> {
    if !(1..=MAX_CALLS).contains(&limits.max_calls) || !(1..=MAX_BYTES).contains(&limits.max_bytes)
    {
        return Err(invalid());
    }
    Ok(())
}
impl ToolApprovalJournal {
    /// Whether every call is denied, cancelled, completed, failed or abandoned: no decision,
    /// dispatch or reconciliation waits on the journal. The run it serves still reads its
    /// results; see `contracts/recovery.md` (Retention).
    pub fn is_settled(&self) -> bool {
        self.calls.values().all(|entry| {
            matches!(
                entry.state,
                StoredToolCallState::Denied
                    | StoredToolCallState::Cancelled
                    | StoredToolCallState::Completed { .. }
                    | StoredToolCallState::Failed { .. }
                    | StoredToolCallState::Abandoned
            )
        })
    }

    /// Validate format, identities, state consistency and encoded retention bounds.
    pub fn validate(&self) -> Result<(), EngineAdapterRequestError> {
        checked_limits(self.limits)?;
        measure(self, self.limits.max_bytes)?;
        if self.version != TOOL_APPROVAL_JOURNAL_VERSION
            || self.revision == 0
            || self.scope.run_id.trim().is_empty()
            || self.scope.binding_revision.trim().is_empty()
            || self.scope.expires_at_unix_ms == 0
            || self.calls.len() > self.limits.max_calls
        {
            return Err(invalid());
        }
        let mut intents = 0;
        for (id, entry) in &self.calls {
            if id.trim().is_empty()
                || id != &entry.call.id
                || entry.tool.name != entry.call.name
                || entry.request_revision != request_revision(&self.scope, &entry.tool, &entry.call)
            {
                return Err(invalid());
            }
            let expected = match entry.state {
                StoredToolCallState::Pending => None,
                StoredToolCallState::Denied => Some(ToolApprovalAction::Deny),
                StoredToolCallState::Cancelled => Some(ToolApprovalAction::Cancel),
                _ => Some(ToolApprovalAction::Allow),
            };
            if entry.decision.as_ref().map(|d| d.action) != expected {
                return Err(invalid());
            }
            if let Some(decision) = &entry.decision {
                if decision.request_revision != entry.request_revision
                    || decision.actor.trim().is_empty()
                    || decision.at_unix_ms >= self.scope.expires_at_unix_ms
                    || decision.at_unix_ms > self.updated_at_unix_ms
                {
                    return Err(invalid());
                }
            }
            if let StoredToolCallState::Failed { error } = &entry.state {
                if !error.tool_call.as_ref().is_some_and(|e| {
                    e.call_id == *id
                        && e.tool_name == entry.call.name
                        && e.outcome != ToolCallOutcome::Uncertain
                }) {
                    return Err(invalid());
                }
            }
            if matches!(entry.state, StoredToolCallState::Intent) {
                intents += 1;
            }
            if matches!(
                entry.state,
                StoredToolCallState::Pending | StoredToolCallState::Denied
            ) && !entry.resolutions.is_empty()
            {
                return Err(invalid());
            }
            if matches!(entry.state, StoredToolCallState::Abandoned)
                && !entry
                    .resolutions
                    .last()
                    .is_some_and(|r| matches!(r.resolution, ToolApprovalReconciliation::Abandon))
            {
                return Err(invalid());
            }
            let mut previous = 0;
            let mut previous_time = 0;
            for (index, resolution) in entry.resolutions.iter().enumerate() {
                let last = index + 1 == entry.resolutions.len();
                let consistent = match &resolution.resolution {
                    ToolApprovalReconciliation::NotApplied => true,
                    ToolApprovalReconciliation::Completed { result } => {
                        last && matches!(&entry.state, StoredToolCallState::Completed { result: stored } if stored == result)
                    }
                    ToolApprovalReconciliation::Abandon => {
                        last && matches!(entry.state, StoredToolCallState::Abandoned)
                    }
                };
                if !consistent
                    || resolution.at_unix_ms < previous_time
                    || resolution.actor.trim().is_empty()
                    || resolution.evidence.trim().is_empty()
                    || resolution.revision <= previous
                    || resolution.revision > self.revision
                    || resolution.at_unix_ms > self.updated_at_unix_ms
                {
                    return Err(invalid());
                }
                previous = resolution.revision;
                previous_time = resolution.at_unix_ms;
            }
        }
        if intents > 1 {
            return Err(invalid());
        }
        Ok(())
    }
}

/// Explicit durable tool-call resumption using a host-owned journal.
///
/// `call` records a pending request and stops the provider loop. The host reads it, records
/// a decision with `decide`, then invokes `resume`. A resumed result is returned only after
/// durable publication. Restoring a provider transcript is owned by the host's continuation.
#[derive(Clone)]
pub struct DurableToolExecutor {
    store: Arc<dyn ToolApprovalStore>,
    registry: ToolRegistry,
    scope: DurableToolApprovalScope,
    max_bytes: usize,
    store_timeout: Duration,
    deadline: Instant,
}
impl DurableToolExecutor {
    fn build(
        store: Arc<dyn ToolApprovalStore>,
        registry: ToolRegistry,
        scope: DurableToolApprovalScope,
        max_bytes: usize,
        store_timeout: Duration,
    ) -> Result<Self, EngineAdapterRequestError> {
        if scope.run_id.trim().is_empty()
            || scope.binding_revision.trim().is_empty()
            || scope.expires_at_unix_ms == 0
            || !(1..=MAX_BYTES).contains(&max_bytes)
            || store_timeout.is_zero()
            || store_timeout > Duration::from_secs(60)
        {
            return Err(invalid());
        }
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(
                scope.expires_at_unix_ms.saturating_sub(now_ms()?),
            ))
            .ok_or_else(invalid)?;
        Ok(Self {
            store,
            registry,
            scope,
            max_bytes,
            store_timeout,
            deadline,
        })
    }

    /// Create a journal exactly once. Store operations have a timeout in (0, 60s].
    pub async fn create(
        store: Arc<dyn ToolApprovalStore>,
        registry: ToolRegistry,
        scope: DurableToolApprovalScope,
        limits: ToolApprovalJournalLimits,
        store_timeout: Duration,
    ) -> Result<Self, EngineAdapterRequestError> {
        checked_limits(limits)?;
        measure(&scope, limits.max_bytes)?;
        let executor = Self::build(
            store,
            registry,
            scope.clone(),
            limits.max_bytes,
            store_timeout,
        )?;
        let record = ToolApprovalJournal {
            version: TOOL_APPROVAL_JOURNAL_VERSION,
            revision: 1,
            scope,
            limits,
            updated_at_unix_ms: now_ms()?,
            calls: BTreeMap::new(),
        };
        executor.live(&record)?;
        executor.write(None, &record).await?;
        Ok(executor)
    }

    /// Open without writing or extending expiry. `max_bytes` bounds host decoding and admission.
    pub async fn open(
        store: Arc<dyn ToolApprovalStore>,
        registry: ToolRegistry,
        scope: DurableToolApprovalScope,
        max_bytes: usize,
        store_timeout: Duration,
    ) -> Result<Self, EngineAdapterRequestError> {
        let executor = Self::build(store, registry, scope, max_bytes, store_timeout)?;
        executor.inspect().await?;
        Ok(executor)
    }

    pub fn definitions(&self) -> Vec<EngineLocalToolDefinition> {
        self.registry.definitions()
    }

    /// Inspect durable state, including expired calls and unresolved intents, without mutation.
    pub async fn inspect(&self) -> Result<ToolApprovalJournal, EngineAdapterRequestError> {
        let record = timeout(
            self.store_timeout,
            self.store.load(&self.scope.run_id, self.max_bytes),
        )
        .await
        .map_err(|_| Self::storage_error(ToolApprovalStoreError::Unavailable))?
        .map_err(Self::storage_error)?
        .ok_or_else(invalid)?;
        measure(&record, self.max_bytes)?;
        record.validate()?;
        if record.scope != self.scope || record.limits.max_bytes > self.max_bytes {
            return Err(invalid());
        }
        Ok(record)
    }
    fn storage_error(error: ToolApprovalStoreError) -> EngineAdapterRequestError {
        match error {
            ToolApprovalStoreError::Conflict => conflict(),
            ToolApprovalStoreError::Unavailable => EngineAdapterRequestError::new(
                "tools",
                "Approval storage outcome requires inspection.",
                ErrorKind::Transport,
            )
            .with_tool_code(ToolCode::ToolApprovalStorage),
        }
    }
    async fn write(
        &self,
        expected: Option<u64>,
        next: &ToolApprovalJournal,
    ) -> Result<(), EngineAdapterRequestError> {
        next.validate()?;
        measure(next, self.max_bytes)?;
        timeout(
            self.store_timeout,
            self.store
                .compare_exchange(&self.scope.run_id, expected, next),
        )
        .await
        .map_err(|_| Self::storage_error(ToolApprovalStoreError::Unavailable))?
        .map_err(Self::storage_error)
    }
    fn live(&self, record: &ToolApprovalJournal) -> Result<(), EngineAdapterRequestError> {
        let now = now_ms()?;
        if now < record.updated_at_unix_ms {
            return Err(invalid());
        }
        if now >= self.scope.expires_at_unix_ms || Instant::now() >= self.deadline {
            return Err(EngineAdapterRequestError::new(
                "tools",
                "Tool approval expired.",
                ErrorKind::Expired,
            )
            .with_tool_code(ToolCode::ToolApprovalExpired));
        }
        Ok(())
    }
    fn idle(record: &ToolApprovalJournal) -> Result<(), EngineAdapterRequestError> {
        if let Some(entry) = record
            .calls
            .values()
            .find(|e| matches!(e.state, StoredToolCallState::Intent))
        {
            return Err(failure(
                ToolCode::ToolApprovalUnresolved,
                "Dispatch intent requires host reconciliation.",
            )
            .with_tool_call(evidence(&entry.call, ToolCallOutcome::Uncertain)));
        }
        Ok(())
    }
    async fn expected(
        &self,
        revision: u64,
    ) -> Result<ToolApprovalJournal, EngineAdapterRequestError> {
        let record = self.inspect().await?;
        if record.revision != revision {
            return Err(conflict());
        }
        Ok(record)
    }
    fn advance(record: &mut ToolApprovalJournal) -> Result<(), EngineAdapterRequestError> {
        let now = now_ms()?;
        if now < record.updated_at_unix_ms {
            return Err(invalid());
        }
        record.updated_at_unix_ms = now;
        record.revision = record.revision.checked_add(1).ok_or_else(invalid)?;
        Ok(())
    }

    /// Persist one validated pending call. Existing call identities are never replaced.
    pub async fn submit(
        &self,
        call: EngineLocalToolCall,
        expected_revision: u64,
    ) -> Result<ToolApprovalJournal, EngineAdapterRequestError> {
        let prepared = self.registry.prepare(call)?;
        let refusal = evidence(&prepared.call, ToolCallOutcome::NotDispatched);
        let work = async {
            let mut record = self.expected(expected_revision).await?;
            Self::idle(&record)?;
            self.live(&record)?;
            if prepared.call.id.trim().is_empty() || record.calls.contains_key(&prepared.call.id) {
                return Err(failure(
                    ToolCode::InvalidToolCallId,
                    "Tool call identity is blank or already recorded.",
                ));
            }
            if record.calls.len() == record.limits.max_calls {
                return Err(failure(
                    ToolCode::ToolApprovalCapacity,
                    "Tool approval call capacity exhausted.",
                ));
            }
            measure(&(prepared.tool(), &prepared.call), record.limits.max_bytes)?;
            let entry = StoredToolCall {
                tool: prepared.tool().clone(),
                request_revision: request_revision(&self.scope, prepared.tool(), &prepared.call),
                call: prepared.call,
                decision: None,
                state: StoredToolCallState::Pending,
                resolutions: Vec::new(),
            };
            record.calls.insert(entry.call.id.clone(), entry);
            Self::advance(&mut record)?;
            self.write(Some(expected_revision), &record).await?;
            Ok(record)
        };
        work.await.map_err(|e: EngineAdapterRequestError| {
            if e.tool_call.is_some() {
                e
            } else {
                e.with_tool_call(refusal)
            }
        })
    }

    /// Persist a host decision for the exact stored request. This method does not execute it.
    pub async fn decide(
        &self,
        call_id: &str,
        request_revision: &str,
        expected_revision: u64,
        action: ToolApprovalAction,
        actor: &str,
    ) -> Result<ToolApprovalJournal, EngineAdapterRequestError> {
        let mut record = self.expected(expected_revision).await?;
        Self::idle(&record)?;
        self.live(&record)?;
        if actor.trim().is_empty() {
            return Err(invalid());
        }
        measure(&actor, record.limits.max_bytes)?;
        let entry = record.calls.get_mut(call_id).ok_or_else(invalid)?;
        if entry.request_revision != request_revision {
            return Err(conflict());
        }
        if entry
            .decision
            .as_ref()
            .is_some_and(|d| d.action == action && d.actor == actor)
        {
            return Ok(record);
        }
        if !(matches!(entry.state, StoredToolCallState::Pending)
            || action == ToolApprovalAction::Cancel
                && matches!(entry.state, StoredToolCallState::Approved))
        {
            return Err(invalid());
        }
        entry.decision = Some(StoredToolDecision {
            request_revision: request_revision.into(),
            action,
            actor: actor.into(),
            at_unix_ms: now_ms()?,
        });
        entry.state = match action {
            ToolApprovalAction::Allow => StoredToolCallState::Approved,
            ToolApprovalAction::Deny => StoredToolCallState::Denied,
            ToolApprovalAction::Cancel => StoredToolCallState::Cancelled,
        };
        Self::advance(&mut record)?;
        self.write(Some(expected_revision), &record).await?;
        Ok(record)
    }

    fn prepare(&self, entry: &StoredToolCall) -> Result<PreparedCall, EngineAdapterRequestError> {
        let prepared = self.registry.prepare(entry.call.clone())?;
        if prepared.tool() != &entry.tool
            || request_revision(&self.scope, prepared.tool(), &prepared.call)
                != entry.request_revision
        {
            return Err(failure(
                ToolCode::InvalidToolApproval,
                "Stored tool definition or arguments changed.",
            )
            .with_tool_call(evidence(&entry.call, ToolCallOutcome::NotDispatched)));
        }
        Ok(prepared)
    }

    /// Resume an approved call, or retrieve its published result, using the journal revision.
    /// An existing intent is never dispatched again by this method.
    pub async fn resume(
        &self,
        call_id: &str,
        expected_revision: u64,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        let mut record = self.expected(expected_revision).await?;
        let entry = record.calls.get(call_id).ok_or_else(invalid)?;
        let refused = |code, message| {
            failure(code, message)
                .with_tool_call(evidence(&entry.call, ToolCallOutcome::NotDispatched))
        };
        match &entry.state {
            StoredToolCallState::Completed { result } => return Ok(result.clone()),
            StoredToolCallState::Failed { error } => return Err(error.clone()),
            StoredToolCallState::Pending => {
                return Err(refused(
                    ToolCode::ToolApprovalPending,
                    "Tool call awaits a host decision.",
                ))
            }
            StoredToolCallState::Denied | StoredToolCallState::Cancelled => {
                return Err(refused(
                    ToolCode::ToolApprovalDenied,
                    "Tool call is terminal.",
                ))
            }
            StoredToolCallState::Abandoned => {
                return Err(
                    failure(ToolCode::ToolApprovalDenied, "Tool call was abandoned.")
                        .with_tool_call(evidence(&entry.call, ToolCallOutcome::Uncertain)),
                )
            }
            StoredToolCallState::Intent => {
                return Err(failure(
                    ToolCode::ToolApprovalUnresolved,
                    "Dispatch intent requires host reconciliation.",
                )
                .with_tool_call(evidence(&entry.call, ToolCallOutcome::Uncertain)))
            }
            StoredToolCallState::Approved => (),
        }
        Self::idle(&record)?;
        self.live(&record)
            .map_err(|e| e.with_tool_call(evidence(&entry.call, ToolCallOutcome::NotDispatched)))?;
        let prepared = self.prepare(entry)?;
        let call = prepared.call.clone();
        record.calls.get_mut(call_id).unwrap().state = StoredToolCallState::Intent;
        Self::advance(&mut record)?;
        self.write(Some(expected_revision), &record)
            .await
            .map_err(|e| e.with_tool_call(evidence(&call, ToolCallOutcome::NotDispatched)))?;
        // Expiry after intent publication still forbids dispatch. The intent requires inspection.
        self.live(&record)
            .map_err(|e| e.with_tool_call(evidence(&call, ToolCallOutcome::NotDispatched)))?;
        let result = timeout_at(self.deadline, prepared.execute())
            .await
            .unwrap_or_else(|_| {
                Err(EngineAdapterRequestError::new(
                    "tools",
                    "Tool execution deadline expired.",
                    ErrorKind::Timeout,
                )
                .with_tool_call(evidence(&call, ToolCallOutcome::Uncertain)))
            });
        let observed = match &result {
            Ok(_) => evidence(&call, ToolCallOutcome::Completed),
            Err(e) => e
                .tool_call
                .as_deref()
                .cloned()
                .unwrap_or_else(|| evidence(&call, ToolCallOutcome::Uncertain)),
        };
        // Uncertain effects retain their intent for explicit reconciliation.
        if observed.outcome == ToolCallOutcome::Uncertain {
            return result;
        }
        measure(&result, record.limits.max_bytes)
            .map_err(|e| e.with_tool_call(observed.clone()))?;
        let intent_revision = record.revision;
        record.calls.get_mut(call_id).unwrap().state = match &result {
            Ok(result) => StoredToolCallState::Completed {
                result: result.clone(),
            },
            Err(error) => StoredToolCallState::Failed {
                error: error.clone(),
            },
        };
        Self::advance(&mut record).map_err(|e| e.with_tool_call(observed.clone()))?;
        self.write(Some(intent_revision), &record)
            .await
            .map_err(|e| e.with_tool_call(observed))?;
        result
    }

    /// Resolve an intent after fencing its original executor and inspecting its effect.
    /// Reconciliation may publish a retained result after expiry; expiry still forbids dispatch.
    pub async fn reconcile(
        &self,
        call_id: &str,
        expected_revision: u64,
        resolution: ToolApprovalReconciliation,
        actor: &str,
        proof: &str,
    ) -> Result<ToolApprovalJournal, EngineAdapterRequestError> {
        let mut record = self.expected(expected_revision).await?;
        measure(&(&resolution, actor, proof), record.limits.max_bytes)?;
        if actor.trim().is_empty() || proof.trim().is_empty() {
            return Err(invalid());
        }
        let entry = record.calls.get_mut(call_id).ok_or_else(invalid)?;
        if !matches!(entry.state, StoredToolCallState::Intent) {
            return Err(invalid());
        }
        if let ToolApprovalReconciliation::Completed { result } = &resolution {
            self.prepare(entry)?.validate_result(result)?;
        }
        entry.state = match &resolution {
            ToolApprovalReconciliation::NotApplied => StoredToolCallState::Approved,
            ToolApprovalReconciliation::Completed { result } => StoredToolCallState::Completed {
                result: result.clone(),
            },
            ToolApprovalReconciliation::Abandon => StoredToolCallState::Abandoned,
        };
        Self::advance(&mut record)?;
        record
            .calls
            .get_mut(call_id)
            .unwrap()
            .resolutions
            .push(ToolApprovalResolution {
                actor: actor.into(),
                evidence: proof.into(),
                revision: record.revision,
                at_unix_ms: record.updated_at_unix_ms,
                resolution,
            });
        self.write(Some(expected_revision), &record).await?;
        Ok(record)
    }
}

#[allow(clippy::double_must_use)]
#[async_trait]
impl EngineLocalToolExecutor for DurableToolExecutor {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        let revision = self
            .inspect()
            .await
            .map_err(|e| e.with_tool_call(evidence(&call, ToolCallOutcome::NotDispatched)))?
            .revision;
        let id = call.id.clone();
        let record = self.submit(call, revision).await?;
        Err(failure(
            ToolCode::ToolApprovalPending,
            "Tool call awaits a durable host decision.",
        )
        .with_tool_call(evidence(
            &record.calls[&id].call,
            ToolCallOutcome::NotDispatched,
        )))
    }
}
