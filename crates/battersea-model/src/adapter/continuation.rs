//! Copyright (c) Scott A Dixon
//! Durable provider turns and host-controlled continuation of tool conversations.
use super::*;
use crate::{configuration_digest, ErrorKind, ProviderContentOrigin, ToolCode};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::{timeout, timeout_at, Instant};

pub const CONVERSATION_CHECKPOINT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationRequest {
    pub shared: EngineAdapterRequest,
    pub tools: Vec<EngineLocalToolDefinition>,
    /// Maximum persisted provider dispatch intents, including explicitly reconciled attempts.
    pub max_turns: u32,
}
impl ConversationRequest {
    pub fn to_request(&self) -> EngineTextStreamRequest {
        EngineTextStreamRequest {
            shared: self.shared.clone(),
            local_tools: self.tools.clone(),
            max_tool_rounds: self.max_turns as usize,
            local_tool_executor: None,
            mock_response: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationBinding {
    pub origin: ProviderContentOrigin,
    /// Digest of safe backend configuration, including model, credentials scope and limits.
    pub configuration_revision: String,
    pub timeout_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationScope {
    pub run_id: String,
    pub tool_binding_revision: String,
    pub expires_at_unix_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationToolResult {
    pub call: EngineLocalToolCall,
    pub result: EngineLocalToolResult,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationTurn {
    /// Versioned adapter-owned transcript after the completed provider turn.
    pub state: Value,
    pub calls: Vec<EngineLocalToolCall>,
    /// Buffered observations become available only with the durable turn publication.
    pub events: Vec<EngineTextStreamEvent>,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredConversationTurn {
    pub attempt: u32,
    pub response: ConversationTurn,
    pub results: BTreeMap<String, ConversationToolResult>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationPhase {
    Ready,
    ProviderIntent,
    AwaitingTools,
    Completed,
    Abandoned,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationResolution {
    NotDispatched,
    Completed { turn: ConversationTurn },
    Abandon,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredConversationResolution {
    pub attempt: u32,
    pub actor: String,
    pub evidence: String,
    pub resolution: ConversationResolution,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationCheckpoint {
    pub version: u32,
    pub revision: u64,
    pub scope: ConversationScope,
    pub binding: ConversationBinding,
    pub request: ConversationRequest,
    pub max_bytes: usize,
    pub updated_at_unix_ms: u64,
    pub attempts: u32,
    pub phase: ConversationPhase,
    pub turns: Vec<StoredConversationTurn>,
    pub resolutions: Vec<StoredConversationResolution>,
    /// Integrity check for accidental modification; storage and authority remain host-owned.
    pub checksum: String,
}
impl std::fmt::Debug for ConversationCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationCheckpoint")
            .field("revision", &self.revision)
            .field("phase", &self.phase)
            .field("attempts", &self.attempts)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationStoreError {
    Conflict,
    Unavailable,
}
/// Successful compare-and-exchange writes are durable and atomic across processes.
/// Failed or cancelled writes may have committed. Loads bound bytes before decoding and never upgrade.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait ConversationStore: Send + Sync {
    async fn load(
        &self,
        run_id: &str,
        max_bytes: usize,
    ) -> Result<Option<ConversationCheckpoint>, ConversationStoreError>;
    async fn compare_exchange(
        &self,
        run_id: &str,
        expected_revision: Option<u64>,
        next: &ConversationCheckpoint,
    ) -> Result<(), ConversationStoreError>;
}
fn error(code: ToolCode, message: &str) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new("conversation", message, ErrorKind::InvalidRequest)
        .with_tool_code(code)
}
fn invalid() -> EngineAdapterRequestError {
    error(
        ToolCode::InvalidToolContinuation,
        "Invalid or incompatible conversation checkpoint.",
    )
}
pub(crate) fn unsupported() -> EngineAdapterRequestError {
    error(
        ToolCode::UnsupportedToolContinuation,
        "Adapter does not support durable tool conversations.",
    )
}
fn clock() -> Result<u64, EngineAdapterRequestError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid())?
        .as_millis()
        .try_into()
        .map_err(|_| invalid())
}
fn bounded(value: &impl Serialize, max: usize) -> Result<(), EngineAdapterRequestError> {
    let bytes = payload::check_payload(value, "conversation")
        .map_err(|e| e.with_tool_code(ToolCode::ToolTranscriptTooLarge))?;
    if bytes > max {
        return Err(error(
            ToolCode::ToolTranscriptTooLarge,
            "Conversation checkpoint capacity exceeded.",
        ));
    }
    Ok(())
}
fn same_call(a: &EngineLocalToolCall, b: &EngineLocalToolCall) -> bool {
    a.id == b.id
        && local_tool_name(&a.name) == local_tool_name(&b.name)
        && a.arguments == b.arguments
}
fn validate_turn_events(
    request: &ConversationRequest,
    turn: &ConversationTurn,
) -> Result<(), EngineAdapterRequestError> {
    let reason = if turn.calls.is_empty() {
        StopReason::Complete
    } else {
        StopReason::ToolUse
    };
    if !matches!(
        turn.events.first(),
        Some(EngineTextStreamEvent::MessageStart { .. })
    ) || !matches!(turn.events.last(),Some(EngineTextStreamEvent::MessageStop { reason:r }) if *r==reason)
        || turn
            .events
            .iter()
            .filter(|e| matches!(e, EngineTextStreamEvent::MessageStart { .. }))
            .count()
            != 1
        || turn
            .events
            .iter()
            .filter(|e| matches!(e, EngineTextStreamEvent::MessageStop { .. }))
            .count()
            != 1
    {
        return Err(invalid());
    }
    let modalities = request
        .shared
        .chat
        .output_modalities
        .clone()
        .unwrap_or_else(|| vec![crate::Modality::Text]);
    let mut text = String::new();
    for event in &turn.events {
        let mode = match event {
            EngineTextStreamEvent::TextDelta { text: part } => {
                text.push_str(part);
                Some(crate::Modality::Text)
            }
            EngineTextStreamEvent::ContentBlock {
                block: crate::ContentBlock::Image { .. },
            } => Some(crate::Modality::Image),
            EngineTextStreamEvent::ContentBlock {
                block: crate::ContentBlock::Audio { .. },
            } => Some(crate::Modality::Audio),
            EngineTextStreamEvent::ContentBlock {
                block: crate::ContentBlock::Video { .. },
            } => Some(crate::Modality::Video),
            EngineTextStreamEvent::ContentBlock {
                block: crate::ContentBlock::Document { .. },
            } => Some(crate::Modality::Document),
            EngineTextStreamEvent::ToolCallStarted { .. }
            | EngineTextStreamEvent::ToolCallCompleted { .. } => return Err(invalid()),
            _ => None,
        };
        if mode.is_some_and(|m| !modalities.contains(&m)) {
            return Err(invalid());
        }
    }
    if turn.calls.is_empty() {
        if let Some(format) = &request.shared.chat.response_format {
            let value = format.parse("conversation", &text)?;
            for event in &turn.events {
                if let EngineTextStreamEvent::StructuredOutput { value: stored } = event {
                    if stored != &value {
                        return Err(invalid());
                    }
                }
            }
        }
    }
    Ok(())
}
impl ConversationCheckpoint {
    fn digest(&self) -> Result<String, EngineAdapterRequestError> {
        let mut value = serde_json::to_value(self).map_err(|_| invalid())?;
        value
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("checksum");
        Ok(configuration_digest(&value))
    }
    /// Check format, integrity, bounds and shared batch invariants without changing storage.
    pub fn validate(&self) -> Result<(), EngineAdapterRequestError> {
        if self.version != CONVERSATION_CHECKPOINT_VERSION
            || self.revision == 0
            || !(1..=payload::PROVIDER_PAYLOAD_BYTES).contains(&self.max_bytes)
            || self.scope.run_id.trim().is_empty()
            || self.scope.tool_binding_revision.trim().is_empty()
            || self.scope.expires_at_unix_ms == 0
            || !(1..=32).contains(&self.request.max_turns)
            || self.attempts > self.request.max_turns
            || self.turns.len() > self.attempts as usize
            || self.binding.timeout_ms == 0
            || self.binding.configuration_revision.is_empty()
        {
            return Err(invalid());
        }
        bounded(self, self.max_bytes)?;
        if self.checksum != self.digest()? {
            return Err(invalid());
        }
        let mut ids = HashSet::new();
        let mut previous = 0;
        for (index, turn) in self.turns.iter().enumerate() {
            if turn.attempt <= previous
                || turn.attempt > self.attempts
                || turn.response.calls.len() > 256
            {
                return Err(invalid());
            }
            previous = turn.attempt;
            validate_turn_events(&self.request, &turn.response)?;
            tool_loop::validate_tool_calls(
                "conversation",
                &self.request.to_request(),
                &turn.response.calls,
            )?;
            for call in &turn.response.calls {
                if !ids.insert(&call.id) {
                    return Err(invalid());
                }
            }
            for (id, result) in &turn.results {
                if id != &result.call.id
                    || !turn
                        .response
                        .calls
                        .iter()
                        .any(|call| same_call(call, &result.call))
                {
                    return Err(invalid());
                }
                result.result.provider_content()?;
            }
            if index + 1 < self.turns.len()
                && (turn.response.calls.is_empty()
                    || turn.results.len() != turn.response.calls.len())
            {
                return Err(invalid());
            }
        }
        let last = self.turns.last();
        let ready = last.is_none_or(|t| {
            !t.response.calls.is_empty() && t.results.len() == t.response.calls.len()
        });
        let consistent = match self.phase {
            ConversationPhase::Ready => ready,
            ConversationPhase::ProviderIntent => ready && self.attempts > previous,
            ConversationPhase::AwaitingTools => last.is_some_and(|t| !t.response.calls.is_empty()),
            ConversationPhase::Completed => last.is_some_and(|t| t.response.calls.is_empty()),
            ConversationPhase::Abandoned => true,
        };
        if !consistent {
            return Err(invalid());
        }
        for resolution in &self.resolutions {
            if resolution.attempt > self.attempts
                || resolution.actor.trim().is_empty()
                || resolution.evidence.trim().is_empty()
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
/// Host commands for a single durable conversation. Provider dispatch never invokes tools.
#[derive(Clone)]
pub struct DurableConversation {
    store: Arc<dyn ConversationStore>,
    adapter: Arc<dyn EngineAdapter>,
    scope: ConversationScope,
    binding: ConversationBinding,
    request: ConversationRequest,
    max_bytes: usize,
    store_timeout: Duration,
    deadline: Instant,
}
impl DurableConversation {
    fn build(
        store: Arc<dyn ConversationStore>,
        adapter: Arc<dyn EngineAdapter>,
        scope: ConversationScope,
        request: ConversationRequest,
        max_bytes: usize,
        store_timeout: Duration,
    ) -> Result<Self, EngineAdapterRequestError> {
        if scope.run_id.trim().is_empty()
            || scope.tool_binding_revision.trim().is_empty()
            || !(1..=32).contains(&request.max_turns)
            || !(1..=payload::PROVIDER_PAYLOAD_BYTES).contains(&max_bytes)
            || store_timeout.is_zero()
            || store_timeout > Duration::from_secs(60)
        {
            return Err(invalid());
        }
        bounded(&request, max_bytes)?;
        let binding = adapter.validate_conversation(&request, None)?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(
                scope.expires_at_unix_ms.saturating_sub(clock()?),
            ))
            .ok_or_else(invalid)?;
        Ok(Self {
            store,
            adapter,
            scope,
            binding,
            request,
            max_bytes,
            store_timeout,
            deadline,
        })
    }
    /// Persist acceptance before the first generation. Expiry must fit the backend operation timeout.
    pub async fn create(
        store: Arc<dyn ConversationStore>,
        adapter: Arc<dyn EngineAdapter>,
        scope: ConversationScope,
        request: ConversationRequest,
        max_bytes: usize,
        store_timeout: Duration,
    ) -> Result<Self, EngineAdapterRequestError> {
        let this = Self::build(
            store,
            adapter,
            scope.clone(),
            request.clone(),
            max_bytes,
            store_timeout,
        )?;
        let now = clock()?;
        if scope.expires_at_unix_ms.saturating_sub(now) > this.binding.timeout_ms {
            return Err(invalid());
        }
        let mut record = ConversationCheckpoint {
            version: CONVERSATION_CHECKPOINT_VERSION,
            revision: 1,
            scope,
            binding: this.binding.clone(),
            request,
            max_bytes,
            updated_at_unix_ms: now,
            attempts: 0,
            phase: ConversationPhase::Ready,
            turns: vec![],
            resolutions: vec![],
            checksum: String::new(),
        };
        this.live(&record)?;
        this.write(None, &mut record).await?;
        Ok(this)
    }
    /// Reopen with the original request, scope and configured adapter. Reads never renew expiry.
    pub async fn open(
        store: Arc<dyn ConversationStore>,
        adapter: Arc<dyn EngineAdapter>,
        scope: ConversationScope,
        request: ConversationRequest,
        max_bytes: usize,
        store_timeout: Duration,
    ) -> Result<Self, EngineAdapterRequestError> {
        let this = Self::build(store, adapter, scope, request, max_bytes, store_timeout)?;
        this.inspect().await?;
        Ok(this)
    }
    fn storage(e: ConversationStoreError) -> EngineAdapterRequestError {
        match e {
            ConversationStoreError::Conflict => error(
                ToolCode::ToolContinuationConflict,
                "Conversation revision changed.",
            ),
            ConversationStoreError::Unavailable => EngineAdapterRequestError::new(
                "conversation",
                "Conversation storage outcome requires inspection.",
                ErrorKind::Transport,
            )
            .with_tool_code(ToolCode::ToolContinuationStorage),
        }
    }
    pub async fn inspect(&self) -> Result<ConversationCheckpoint, EngineAdapterRequestError> {
        let record = timeout(
            self.store_timeout,
            self.store.load(&self.scope.run_id, self.max_bytes),
        )
        .await
        .map_err(|_| Self::storage(ConversationStoreError::Unavailable))?
        .map_err(Self::storage)?
        .ok_or_else(invalid)?;
        bounded(&record, self.max_bytes)?;
        record.validate()?;
        if record.scope != self.scope
            || record.request != self.request
            || record.binding != self.binding
            || record.max_bytes > self.max_bytes
        {
            return Err(invalid());
        }
        for turn in &record.turns {
            if self
                .adapter
                .validate_conversation_turn(&record.request, &turn.response)?
                != self.binding
            {
                return Err(invalid());
            }
        }
        Ok(record)
    }
    async fn expected(
        &self,
        revision: u64,
    ) -> Result<ConversationCheckpoint, EngineAdapterRequestError> {
        let r = self.inspect().await?;
        if r.revision != revision {
            return Err(Self::storage(ConversationStoreError::Conflict));
        }
        Ok(r)
    }
    fn live(&self, r: &ConversationCheckpoint) -> Result<(), EngineAdapterRequestError> {
        let now = clock()?;
        if now < r.updated_at_unix_ms {
            return Err(invalid());
        }
        if now >= self.scope.expires_at_unix_ms || Instant::now() >= self.deadline {
            return Err(error(
                ToolCode::ToolContinuationExpired,
                "Conversation expired.",
            ));
        }
        Ok(())
    }
    fn advance(r: &mut ConversationCheckpoint) -> Result<(), EngineAdapterRequestError> {
        let now = clock()?;
        if now < r.updated_at_unix_ms {
            return Err(invalid());
        }
        r.revision = r.revision.checked_add(1).ok_or_else(invalid)?;
        r.updated_at_unix_ms = now;
        Ok(())
    }
    async fn write(
        &self,
        expected: Option<u64>,
        r: &mut ConversationCheckpoint,
    ) -> Result<(), EngineAdapterRequestError> {
        bounded(r, self.max_bytes)?;
        r.checksum = r.digest()?;
        r.validate()?;
        bounded(r, self.max_bytes)?;
        timeout(
            self.store_timeout,
            self.store.compare_exchange(&self.scope.run_id, expected, r),
        )
        .await
        .map_err(|_| Self::storage(ConversationStoreError::Unavailable))?
        .map_err(Self::storage)
    }
    /// Copy an exact, completed tool result from host-owned durable execution state.
    pub async fn record_tool_result(
        &self,
        expected_revision: u64,
        result: ConversationToolResult,
    ) -> Result<ConversationCheckpoint, EngineAdapterRequestError> {
        let mut r = self.expected(expected_revision).await?;
        if r.phase != ConversationPhase::AwaitingTools {
            return Err(invalid());
        }
        bounded(&result, self.max_bytes)?;
        result.result.provider_content()?;
        let turn = r.turns.last_mut().ok_or_else(invalid)?;
        if !turn
            .response
            .calls
            .iter()
            .any(|call| same_call(call, &result.call))
        {
            return Err(invalid());
        }
        if let Some(previous) = turn.results.get(&result.call.id) {
            return if previous == &result {
                Ok(r)
            } else {
                Err(invalid())
            };
        }
        turn.results.insert(result.call.id.clone(), result);
        Self::advance(&mut r)?;
        self.write(Some(expected_revision), &mut r).await?;
        Ok(r)
    }
    fn retain(
        &self,
        r: &mut ConversationCheckpoint,
        mut turn: ConversationTurn,
    ) -> Result<(), EngineAdapterRequestError> {
        bounded(&turn, self.max_bytes)?;
        if self.adapter.validate_conversation_turn(&r.request, &turn)? != self.binding {
            return Err(invalid());
        }
        tool_loop::validate_tool_calls("conversation", &r.request.to_request(), &turn.calls)?;
        for event in &mut turn.events {
            match event {
                EngineTextStreamEvent::MessageStart { turn_index } => *turn_index = r.attempts - 1,
                EngineTextStreamEvent::TokenUsage { usage } => usage.turn_index = r.attempts - 1,
                EngineTextStreamEvent::ToolCallStarted { .. }
                | EngineTextStreamEvent::ToolCallCompleted { .. } => return Err(invalid()),
                _ => (),
            }
        }
        r.phase = if turn.calls.is_empty() {
            ConversationPhase::Completed
        } else {
            ConversationPhase::AwaitingTools
        };
        r.turns.push(StoredConversationTurn {
            attempt: r.attempts,
            response: turn,
            results: BTreeMap::new(),
        });
        Ok(())
    }
    /// Dispatch one next turn, or inspect a terminal result. Pending results stop before provider I/O.
    pub async fn resume(
        &self,
        expected_revision: u64,
    ) -> Result<ConversationCheckpoint, EngineAdapterRequestError> {
        let r = self.expected(expected_revision).await?;
        let evidence = r
            .turns
            .last()
            .and_then(|t| {
                t.response
                    .calls
                    .iter()
                    .rev()
                    .find_map(|c| t.results.get(&c.id))
            })
            .map(|r| crate::ToolCallEvidence {
                call_id: r.call.id.clone(),
                tool_name: r.call.name.clone(),
                outcome: crate::ToolCallOutcome::Completed,
                request_id: None,
            });
        self.resume_record(r, expected_revision)
            .await
            .map_err(|mut e| {
                if e.tool_call.is_none() {
                    e.tool_call = evidence.map(Box::new);
                }
                e
            })
    }
    async fn resume_record(
        &self,
        mut r: ConversationCheckpoint,
        expected_revision: u64,
    ) -> Result<ConversationCheckpoint, EngineAdapterRequestError> {
        if r.phase == ConversationPhase::Completed {
            return Ok(r);
        }
        if r.phase == ConversationPhase::ProviderIntent {
            return Err(error(
                ToolCode::ToolContinuationUnresolved,
                "Provider dispatch intent requires host reconciliation.",
            ));
        }
        if r.phase == ConversationPhase::Abandoned {
            return Err(invalid());
        }
        self.live(&r)?;
        let mut results = Vec::new();
        if let Some(turn) = r.turns.last() {
            if turn.results.len() != turn.response.calls.len() {
                return Err(error(
                    ToolCode::ToolApprovalPending,
                    "Conversation awaits durable tool results.",
                ));
            }
            results = turn
                .response
                .calls
                .iter()
                .map(|c| turn.results[&c.id].clone())
                .collect();
        }
        if r.attempts == r.request.max_turns {
            return Err(error(
                ToolCode::ToolRoundsExceeded,
                "Conversation exhausted its provider turn budget.",
            ));
        }
        let state = r.turns.last().map(|t| t.response.state.clone());
        self.adapter
            .validate_conversation(&r.request, state.as_ref())?;
        r.attempts += 1;
        r.phase = ConversationPhase::ProviderIntent;
        Self::advance(&mut r)?;
        self.write(Some(expected_revision), &mut r).await?;
        self.live(&r)?;
        let response = timeout_at(
            self.deadline,
            self.adapter
                .conversation_turn(r.request.clone(), state, results),
        )
        .await
        .map_err(|_| {
            EngineAdapterRequestError::new(
                "conversation",
                "Provider turn deadline expired; intent requires inspection.",
                ErrorKind::Timeout,
            )
        })??;
        self.retain(&mut r, response)
            .map_err(|e| e.with_dispatch(crate::DispatchState::Unknown))?;
        let intent_revision = r.revision;
        Self::advance(&mut r).map_err(|e| e.with_dispatch(crate::DispatchState::Unknown))?;
        self.write(Some(intent_revision), &mut r)
            .await
            .map_err(|e| e.with_dispatch(crate::DispatchState::Unknown))?;
        Ok(r)
    }
    /// Fence the original provider task before reconciling an intent or abandoning pending work.
    /// Recovered output must come from retained provider evidence. Attempts and expiry are preserved.
    pub async fn reconcile(
        &self,
        expected_revision: u64,
        resolution: ConversationResolution,
        actor: &str,
        evidence: &str,
    ) -> Result<ConversationCheckpoint, EngineAdapterRequestError> {
        let mut r = self.expected(expected_revision).await?;
        if actor.trim().is_empty()
            || evidence.trim().is_empty()
            || matches!(
                r.phase,
                ConversationPhase::Completed | ConversationPhase::Abandoned
            )
        {
            return Err(invalid());
        }
        if r.phase != ConversationPhase::ProviderIntent
            && !matches!(resolution, ConversationResolution::Abandon)
        {
            return Err(invalid());
        }
        bounded(&(&resolution, actor, evidence), self.max_bytes)?;
        match &resolution {
            ConversationResolution::NotDispatched => r.phase = ConversationPhase::Ready,
            ConversationResolution::Completed { turn } => self.retain(&mut r, turn.clone())?,
            ConversationResolution::Abandon => r.phase = ConversationPhase::Abandoned,
        }
        r.resolutions.push(StoredConversationResolution {
            attempt: r.attempts,
            actor: actor.into(),
            evidence: evidence.into(),
            resolution,
        });
        Self::advance(&mut r)?;
        self.write(Some(expected_revision), &mut r).await?;
        Ok(r)
    }
}
