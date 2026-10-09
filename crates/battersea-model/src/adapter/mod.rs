//! Copyright (c) Scott A Dixon
//!
//! Defines the provider-neutral text-generation adapter contracts used by the
//! engine core. Concrete provider modules implement these traits while the core
//! consumes the shared configuration, request, response, and logging shapes here.

pub mod error;
pub mod tool_loop;

pub use crate::engine::EngineToolExecutionMode;
use crate::engine::{EngineBackendCapabilities, EngineChatParameters};
use async_trait::async_trait;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

use self::error::EngineAdapterRequestError;

/// Error for a chat-backend value that configuration must supply. The
/// adapters carry no hardcoded transport, auth, or endpoint defaults: if
/// configuration does not provide the value, the request cannot proceed.
pub fn missing_backend_option(provider: &str, field: &str) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new(
        provider,
        format!("Backend must configure {field}."),
        "invalid_request",
    )
}

/// Identifies the high-level engine operation a backend request is serving.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EngineOperation {
    FlowTextStream,
}

impl EngineOperation {
    /// Returns the stable operation label written into diagnostics and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FlowTextStream => "flow-text-stream",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MockToolCallSpec {
    /// Tool to call. Accepts the canonical rpc name (`example.lookup`) or
    /// the provider-facing name (`example_lookup`); the consumer
    /// normalises by replacing `.` with `_`.
    pub name: String,
    /// JSON arguments object passed to the tool. A missing value is
    /// treated as `{}`. Must satisfy the tool's input schema or the
    /// call errors (which a test can also assert on).
    #[serde(default)]
    pub arguments: Value,
    /// Tool-loop round this call belongs to (1-based). All round-N
    /// calls are issued and their results folded back before round
    /// N+1 runs, exercising multi-round tool use.
    #[serde(default = "default_mock_tool_round")]
    pub round: u32,
}

fn default_mock_tool_round() -> u32 {
    1
}

/// A refusal the mock backend returns in place of a response, as a provider
/// adapter reports one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MockFailureSpec {
    /// The adapter classification: `auth`, `rate_limit`, `server`, `request`,
    /// `transport` or `invalid_response`.
    pub classification: String,
    pub message: String,
    /// Emit this many response chunks before failing. Omitted means a
    /// pre-stream refusal.
    #[serde(default, rename = "afterChunks")]
    pub after_chunks: Option<u32>,
}

/// Stores optional backend-level tuning knobs shared across adapter implementations.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EngineBackendOptions {
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
    #[serde(rename = "maxRetries")]
    pub max_retries: Option<u32>,
    /// Maximum time to wait for the *next* streamed event before treating the
    /// provider stream as stalled and aborting it with an error. This is the
    /// watchdog that prevents a silent, unbounded hang (and the money burn
    /// that comes with a model reasoning against a dead stream). Idle-based:
    /// the clock resets on every event, so a slow-but-live stream keeps going;
    /// only a genuine stall trips it. Required for streaming chat backends —
    /// authored in `engine.yaml`, never hidden as a constant in the engine.
    #[serde(rename = "streamIdleTimeoutMs")]
    pub stream_idle_timeout_ms: Option<u64>,
    /// When true, run generations through OpenAI's background mode: the
    /// request is created with `background: true` and polled to
    /// completion rather than streamed. Required for the "pro" reasoning
    /// models, which reason silently for minutes — longer than any
    /// streaming idle window or the per-request HTTP timeout. The idle
    /// watchdog covers the complete request, including tool turns.
    pub background: Option<bool>,
    /// How a background response is consumed: `"stream"` (create with
    /// `stream: true` and consume the SSE feed, reconnecting from the
    /// `sequence_number` cursor across drops — live tokens) or `"poll"`
    /// (create then poll to completion — no live tokens). Only used when
    /// `background` is true.
    #[serde(rename = "backgroundMode")]
    pub background_mode: Option<String>,
    /// Idle gap on a background SSE stream that triggers a reconnect from
    /// the last cursor. Only used in `backgroundMode: "stream"`.
    #[serde(rename = "backgroundReconnectIdleMs")]
    pub background_reconnect_idle_ms: Option<u64>,
    /// Maximum reconnect attempts before aborting a background stream
    /// (money-safety). Only used in `backgroundMode: "stream"`.
    #[serde(rename = "backgroundMaxReconnects")]
    pub background_max_reconnects: Option<u32>,
    /// How often to poll a background response for completion. Only used
    /// when `background` is true and `backgroundMode` is `"poll"`.
    #[serde(rename = "backgroundPollIntervalMs")]
    pub background_poll_interval_ms: Option<u64>,
    /// Hard ceiling on total time spent polling a background response
    /// before giving up and aborting (money-safety bound). Only used
    /// when `background` is true.
    #[serde(rename = "backgroundMaxWaitMs")]
    pub background_max_wait_ms: Option<u64>,
    pub transport: Option<String>,
    #[serde(rename = "mockResponseTargetChars")]
    pub mock_response_target_chars: Option<u32>,
    #[serde(rename = "mockStreamDelayMultiplier")]
    pub mock_stream_delay_multiplier: Option<f64>,
    /// Test-only: when set on a `mock` backend, the inline mock streaming
    /// path emits these strings as `EngineTextStreamEvent::ReasoningDelta`
    /// events in order, before the visible response chunks. Provides a
    /// deterministic path to exercise the reasoning / thinking surface
    /// (graph port + controller output) end-to-end without relying on a
    /// live reasoning-capable provider.
    #[serde(rename = "mockStreamingReasoningChunks")]
    pub mock_streaming_reasoning_chunks: Option<Vec<String>>,
    /// Test-only: when set on a `mock` backend, the inline mock path
    /// replays these tool calls (in `round` order) through the real
    /// local-tool executor whenever the activated flow offers tools.
    /// Provides a deterministic path to exercise tool-using flows
    /// end-to-end without a live model.
    #[serde(rename = "mockToolCalls")]
    pub mock_tool_calls: Option<Vec<MockToolCallSpec>>,
    /// Test-only: when set on a `mock` backend, every request is refused with
    /// this failure before anything streams. Provides a deterministic path to
    /// exercise a failed send end-to-end without a live provider.
    #[serde(rename = "mockFailure")]
    pub mock_failure: Option<MockFailureSpec>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl EngineBackendOptions {
    /// Resolves the effective request timeout in milliseconds, falling back when unset.
    pub fn timeout_ms(&self, fallback: u64) -> u64 {
        self.timeout_ms.unwrap_or(fallback)
    }

    /// Resolves the effective retry budget for outbound requests, falling back when unset.
    pub fn max_retries(&self, fallback: u32) -> u32 {
        self.max_retries.unwrap_or(fallback)
    }

    /// Resolves the target character count used by mock streaming responses.
    ///
    /// Zero and missing values are treated as unusable and therefore fall back to `None`.
    pub fn mock_response_target_chars(&self) -> Option<usize> {
        self.mock_response_target_chars
            .filter(|value| *value > 0)
            .map(|value| value as usize)
    }

    /// Resolves the multiplier applied to mock per-chunk delays.
    ///
    /// Missing or non-finite values fall back to `1.0`, while negative finite values clamp to `0.0`.
    pub fn mock_stream_delay_multiplier(&self) -> f64 {
        self.mock_stream_delay_multiplier
            .filter(|value| value.is_finite())
            .map(|value| value.max(0.0))
            .unwrap_or(1.0)
    }

    /// Returns the configured mock reasoning chunks, filtering out empty
    /// strings so callers can treat a returned `Some` as "actually has
    /// reasoning to emit."
    pub fn mock_streaming_reasoning_chunks(&self) -> Option<Vec<String>> {
        let chunks = self.mock_streaming_reasoning_chunks.as_ref()?;
        let filtered: Vec<String> = chunks
            .iter()
            .filter(|chunk| !chunk.is_empty())
            .cloned()
            .collect();
        if filtered.is_empty() {
            None
        } else {
            Some(filtered)
        }
    }

    /// Returns the configured mock tool-call script, dropping entries
    /// with a blank name so callers can treat a returned `Some` as
    /// "actually has tool calls to replay."
    pub fn mock_tool_calls(&self) -> Option<Vec<MockToolCallSpec>> {
        let specs = self.mock_tool_calls.as_ref()?;
        let filtered: Vec<MockToolCallSpec> = specs
            .iter()
            .filter(|spec| !spec.name.trim().is_empty())
            .cloned()
            .collect();
        if filtered.is_empty() {
            None
        } else {
            Some(filtered)
        }
    }
}

/// Describes how a backend should obtain and present authentication credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineAuthConfig {
    pub auth_type: String,
    pub api_key_env: String,
    pub header: Option<String>,
    pub version_header: Option<String>,
    pub version: Option<String>,
    pub has_api_key: bool,
    #[serde(skip_serializing, default)]
    pub api_key: Option<String>,
}

/// Represents one configured text-generation backend from the runtime config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineBackendConfig {
    pub id: String,
    pub provider: String,
    pub label: String,
    pub enabled: bool,
    pub endpoint: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_order: Option<u32>,
    pub chat: EngineChatParameters,
    pub capabilities: EngineBackendCapabilities,
    pub context_window_tokens: u64,
    pub options: EngineBackendOptions,
    pub auth: EngineAuthConfig,
    /// Single-line summary, ≤100 characters, surfaced in compact UI
    /// affordances (model picker rows, status labels). Authored on
    /// the concrete backend; family bases do NOT contribute.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub short_description: String,
    /// Multi-line markdown description. The engine assembles this
    /// from every family/mixin in the merge chain plus the concrete
    /// backend's section, joined by blank lines, so common context
    /// (provider envelope, family defaults) appears once and authors
    /// only retype concrete-model context per backend.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub long_description: String,
}

/// Carries request-scoped context so adapter logs can be attributed to a backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineAdapterDebugContext {
    pub provider: String,
    pub backend: String,
    pub model: String,
    pub sdk: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
}

/// Captures a parsed backend error in a provider-neutral structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineAdapterErrorInfo {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub classification: String,
}

/// Summarises an outbound request in a form suitable for structured logging.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineAdapterRequestLog {
    pub context: EngineAdapterDebugContext,
    pub operation: EngineOperation,
    pub detail: EngineAdapterRequestDetail,
}

/// Records provider-specific request details without forcing the core to parse raw APIs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineAdapterRequestDetail {
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    pub temperature: f64,
    pub instructions_chars: usize,
    pub user_content_chars: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<Value>,
}

/// Summarises a successful provider response for diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineAdapterResponseLog {
    pub context: EngineAdapterDebugContext,
    pub operation: EngineOperation,
    pub status: u16,
    pub detail: EngineAdapterResponseDetail,
}

/// Records provider-specific response details in a stable logging shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineAdapterResponseDetail {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_chars: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

/// Captures an incremental streaming event emitted by a backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineAdapterStreamLog {
    pub context: EngineAdapterDebugContext,
    pub operation: EngineOperation,
    pub event_type: String,
    pub detail: Value,
}

/// Captures a failed provider request together with parsed error details.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineAdapterErrorLog {
    pub context: EngineAdapterDebugContext,
    pub operation: EngineOperation,
    pub detail: EngineAdapterErrorInfo,
}

/// Receives adapter telemetry without constraining the concrete logging backend.
// async-trait adds must_use to boxed futures, which already carry it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait EngineAdapterLogger: Send + Sync {
    async fn on_request(&self, _entry: EngineAdapterRequestLog) {}
    async fn on_response(&self, _entry: EngineAdapterResponseLog) {}
    async fn on_stream_event(&self, _entry: EngineAdapterStreamLog) {}
    async fn on_error(&self, _entry: EngineAdapterErrorLog) {}
}

/// Represents a plain-text backend request after prompt assembly is complete.
#[derive(Debug, Clone)]
pub struct EngineAdapterRequest {
    pub operation: EngineOperation,
    pub messages: Vec<crate::Message>,
    pub chat: EngineChatParameters,
    pub debug_rules: Option<Value>,
}

/// Describes one engine-local function tool exposed during an orchestrated tool loop.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineLocalToolDefinition {
    pub name: String,
    /// Natural-language description forwarded to the provider as the
    /// function tool's `description`. This is the model's primary
    /// signal for whether and how to call the tool. Applications supply it.
    pub description: String,
    pub input_schema: Value,
}

/// Represents one provider-requested local tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineLocalToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Represents the JSON result returned to a provider after a local tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineLocalToolResult {
    pub content: Value,
}

// async-trait adds must_use to boxed futures, which already carry it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait EngineLocalToolExecutor: Send + Sync {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError>;
}

/// Represents a backend request that expects a streaming text response.
pub type MockResponseRenderer = dyn Fn(&str) -> String + Send + Sync;

#[derive(Clone)]
pub struct EngineTextStreamRequest {
    /// Optional host-supplied response formatter for deterministic mock runs.
    pub mock_response: Option<Arc<MockResponseRenderer>>,
    pub shared: EngineAdapterRequest,
    pub local_tools: Vec<EngineLocalToolDefinition>,
    pub local_tool_executor: Option<Arc<dyn EngineLocalToolExecutor>>,
    pub max_tool_rounds: usize,
}

/// Provider-neutral token usage reported before, during, or after text generation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct EngineTokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

/// Provider-neutral event emitted by streaming text backends.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineTextStreamEvent {
    TextDelta {
        text: String,
    },
    /// Streaming chain-of-thought / "thinking" summary deltas. Emitted by
    /// providers that expose a reasoning channel (OpenAI o-series via the
    /// Responses API's `reasoning_summary_text.*` events, Anthropic with
    /// extended thinking via `thinking_delta` content blocks). Models that
    /// do not surface reasoning never emit this variant.
    ReasoningDelta {
        text: String,
    },
    /// A locally-executed (engine-managed) tool round has begun. Emitted
    /// alongside the existing `local_tool_call_started` logger event so
    /// host UIs can surface `Calling tool: <name>` while the adapter
    /// awaits the executor and the model's continuation. `arguments`
    /// carries the JSON the model handed to the tool — the chronicle
    /// captures it so replays and audits see the exact invocation.
    ToolCallStarted {
        id: String,
        name: String,
        arguments: Value,
        started_at: String,
    },
    /// The locally-executed tool round has finished. `ok = false`
    /// indicates the executor returned an error. `result` carries the
    /// executor's return JSON on success (and the empty value on
    /// failure); `error_message` / `error_classification` carry the
    /// failure details so the chronicle can surface them without
    /// scraping the logger event stream.
    ToolCallCompleted {
        id: String,
        name: String,
        ok: bool,
        #[serde(default)]
        result: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_classification: Option<String>,
        finished_at: String,
    },
    TokenUsage {
        usage: EngineTokenUsage,
    },
}

/// Streams text and usage events from a provider while surfacing transport and provider failures.
pub type EngineTextStream =
    BoxStream<'static, Result<EngineTextStreamEvent, EngineAdapterRequestError>>;

/// Wraps a provider text stream with an idle watchdog: if no event arrives
/// within `idle`, the stream surfaces a single timeout error and then ENDS,
/// so the consumer stops and the underlying provider connection is dropped
/// rather than left hanging. The clock resets on every event, so a slow but
/// live stream keeps flowing; only a genuine stall trips it.
///
/// This is the guardrail against a silent, unbounded hang — a model reasoning
/// against a dead stream, burning money with no result and no error. The
/// window is authored in `engine.yaml` (`options.streamIdleTimeoutMs`), never
/// hardcoded here.
pub fn with_stream_idle_watchdog(
    stream: EngineTextStream,
    idle: std::time::Duration,
    provider: impl Into<String>,
) -> EngineTextStream {
    use futures_util::StreamExt as _;
    Box::pin(futures_util::stream::unfold(
        Some((stream, provider.into())),
        move |state| async move {
            let (mut stream, provider) = state?;
            match tokio::time::timeout(idle, stream.next()).await {
                Ok(Some(event)) => Some((event, Some((stream, provider)))),
                Ok(None) => None,
                Err(_) => Some((
                    Err(EngineAdapterRequestError::new(
                        provider,
                        format!(
                            "Provider stream idle deadline exceeded ({}ms).",
                            idle.as_millis()
                        ),
                        error::ErrorKind::Timeout,
                    )),
                    None,
                )),
            }
        },
    ))
}

/// Maps a canonical engine tool name to the portable function-name alphabet.
pub fn local_tool_name(name: &str) -> String {
    name.replace('.', "_")
}

pub fn validate_chat_request(
    backend: &EngineBackendConfig,
    request: &EngineTextStreamRequest,
) -> Result<(), EngineAdapterRequestError> {
    crate::messages::validate_text_messages(&backend.provider, &request.shared.messages)?;
    if !request.shared.chat.stream {
        return Err(EngineAdapterRequestError::new(
            &backend.provider,
            "This request path requires streaming mode.",
            error::ErrorKind::InvalidRequest,
        ));
    }
    if request.max_tool_rounds == 0 || request.max_tool_rounds > 32 {
        return Err(EngineAdapterRequestError::new(
            &backend.provider,
            "Tool rounds must be between 1 and 32.",
            error::ErrorKind::InvalidRequest,
        ));
    }
    let mut names = std::collections::HashSet::new();
    for tool in &request.local_tools {
        if tool.name.trim().is_empty() || !names.insert(local_tool_name(&tool.name)) {
            return Err(EngineAdapterRequestError::new(
                &backend.provider,
                "Duplicate tool names after provider normalization.",
                error::ErrorKind::InvalidRequest,
            ));
        }
        jsonschema::validator_for(&tool.input_schema).map_err(|error| {
            EngineAdapterRequestError::new(
                &backend.provider,
                format!("Invalid tool schema: {error}"),
                error::ErrorKind::InvalidRequest,
            )
        })?;
    }
    backend
        .capabilities
        .validate_chat_parameters(&request.shared.chat)
        .map_err(|message| {
            EngineAdapterRequestError::new(&backend.provider, message, "invalid_request")
        })?;
    if let Some(name) = request.shared.chat.tool_choice.tool_name.as_deref() {
        if request.shared.chat.tool_choice.mode == crate::engine::EngineChatToolChoiceMode::Tool
            && !request
                .local_tools
                .iter()
                .any(|tool| local_tool_name(&tool.name) == local_tool_name(name))
        {
            return Err(EngineAdapterRequestError::new(
                &backend.provider,
                format!("Named tool is not offered: {name}."),
                "invalid_request",
            ));
        }
    }
    Ok(())
}

/// Defines the behaviour every text-generation backend must implement.
// async-trait adds must_use to boxed futures, which already carry it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait EngineAdapter: Send + Sync {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext;

    async fn count_text_stream_input_tokens(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<u64, EngineAdapterRequestError>;

    async fn stream_text(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError>;
}

/// Emits one structured request log entry when a logger is configured and otherwise does nothing.
pub async fn emit_request(
    logger: Option<&Arc<dyn EngineAdapterLogger>>,
    context: &EngineAdapterDebugContext,
    request: &EngineAdapterRequest,
    schema_name: Option<&str>,
    body: Option<Value>,
) {
    if let Some(logger) = logger {
        logger
            .on_request(EngineAdapterRequestLog {
                context: context.clone(),
                operation: request.operation,
                detail: EngineAdapterRequestDetail {
                    stream: matches!(request.operation, EngineOperation::FlowTextStream),
                    schema_name: schema_name.map(ToOwned::to_owned),
                    max_output_tokens: request.chat.max_output_tokens,
                    temperature: request.chat.temperature,
                    instructions_chars: request.text_for_role(crate::Role::System).chars().count(),
                    user_content_chars: request.text_for_role(crate::Role::User).chars().count(),
                    body,
                    rules: request.debug_rules.clone(),
                },
            })
            .await;
    }
}

/// Emits one structured response log entry when a logger is configured and otherwise does nothing.
pub async fn emit_response(
    logger: Option<&Arc<dyn EngineAdapterLogger>>,
    context: &EngineAdapterDebugContext,
    operation: EngineOperation,
    status: u16,
    detail: EngineAdapterResponseDetail,
) {
    if let Some(logger) = logger {
        logger
            .on_response(EngineAdapterResponseLog {
                context: context.clone(),
                operation,
                status,
                detail,
            })
            .await;
    }
}

/// Emits one structured stream-event log entry when a logger is configured and otherwise does nothing.
pub async fn emit_stream_event(
    logger: Option<&Arc<dyn EngineAdapterLogger>>,
    context: &EngineAdapterDebugContext,
    operation: EngineOperation,
    event_type: impl Into<String>,
    detail: Value,
) {
    if let Some(logger) = logger {
        logger
            .on_stream_event(EngineAdapterStreamLog {
                context: context.clone(),
                operation,
                event_type: event_type.into(),
                detail,
            })
            .await;
    }
}

/// Emits one structured error log entry when a logger is configured and otherwise does nothing.
pub async fn emit_error(
    logger: Option<&Arc<dyn EngineAdapterLogger>>,
    context: &EngineAdapterDebugContext,
    operation: EngineOperation,
    error: &EngineAdapterRequestError,
) {
    if let Some(logger) = logger {
        logger
            .on_error(EngineAdapterErrorLog {
                context: context.clone(),
                operation,
                detail: EngineAdapterErrorInfo {
                    message: error.message.clone(),
                    status_code: error.status_code,
                    request_id: error.request_id.clone(),
                    classification: error.classification.to_string(),
                },
            })
            .await;
    }
}

impl EngineAdapterRequest {
    pub fn text_for_role(&self, role: crate::Role) -> String {
        self.messages
            .iter()
            .filter(|message| message.role == role)
            .map(crate::Message::text_content)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::error::EngineAdapterRequestError;
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingLogger {
        requests: Mutex<Vec<EngineAdapterRequestLog>>,
        responses: Mutex<Vec<EngineAdapterResponseLog>>,
        stream_events: Mutex<Vec<EngineAdapterStreamLog>>,
        errors: Mutex<Vec<EngineAdapterErrorLog>>,
    }

    #[async_trait]
    impl EngineAdapterLogger for RecordingLogger {
        async fn on_request(&self, entry: EngineAdapterRequestLog) {
            self.requests.lock().expect("lock requests").push(entry);
        }

        async fn on_response(&self, entry: EngineAdapterResponseLog) {
            self.responses.lock().expect("lock responses").push(entry);
        }

        async fn on_stream_event(&self, entry: EngineAdapterStreamLog) {
            self.stream_events
                .lock()
                .expect("lock stream events")
                .push(entry);
        }

        async fn on_error(&self, entry: EngineAdapterErrorLog) {
            self.errors.lock().expect("lock errors").push(entry);
        }
    }

    fn sample_context() -> EngineAdapterDebugContext {
        EngineAdapterDebugContext {
            provider: "openai".to_string(),
            backend: "backend-1".to_string(),
            model: "synthetic-chat-model".to_string(),
            sdk: "async-openai".to_string(),
            base_url: Some("https://api.example.test/v1".to_string()),
            timeout_ms: Some(1234),
            max_retries: Some(2),
        }
    }

    fn sample_request() -> EngineAdapterRequest {
        EngineAdapterRequest {
            operation: EngineOperation::FlowTextStream,
            messages: vec![
                crate::Message::text(crate::Role::System, "System prompt".to_string()),
                crate::Message::text(crate::Role::User, "User request".to_string()),
            ],
            chat: {
                let mut chat = EngineChatParameters::default_for_provider("openai");
                chat.temperature = 0.7;
                chat.max_output_tokens = Some(256);
                chat
            },
            debug_rules: Some(json!({"mode":"strict"})),
        }
    }

    #[test]
    fn engine_operation_as_str_uses_stable_kebab_case_labels() {
        assert_eq!(EngineOperation::FlowTextStream.as_str(), "flow-text-stream");
    }

    #[test]
    fn backend_option_helpers_honour_valid_values_and_fallbacks() {
        let configured = EngineBackendOptions {
            timeout_ms: Some(9000),
            max_retries: Some(4),
            stream_idle_timeout_ms: Some(180_000),
            background: None,
            background_mode: None,
            background_reconnect_idle_ms: None,
            background_max_reconnects: None,
            background_poll_interval_ms: None,
            background_max_wait_ms: None,
            transport: None,
            mock_response_target_chars: Some(120),
            mock_stream_delay_multiplier: Some(1.5),
            mock_streaming_reasoning_chunks: None,
            mock_tool_calls: None,
            mock_failure: None,
            extra: BTreeMap::new(),
        };
        let defaults = EngineBackendOptions::default();
        let zero_chars = EngineBackendOptions {
            mock_response_target_chars: Some(0),
            ..EngineBackendOptions::default()
        };
        let negative_delay = EngineBackendOptions {
            mock_stream_delay_multiplier: Some(-2.0),
            ..EngineBackendOptions::default()
        };
        let non_finite_delay = EngineBackendOptions {
            mock_stream_delay_multiplier: Some(f64::NAN),
            ..EngineBackendOptions::default()
        };

        assert_eq!(configured.timeout_ms(1000), 9000);
        assert_eq!(configured.max_retries(1), 4);
        assert_eq!(configured.mock_response_target_chars(), Some(120));
        assert_eq!(configured.mock_stream_delay_multiplier(), 1.5);

        assert_eq!(defaults.timeout_ms(1000), 1000);
        assert_eq!(defaults.max_retries(1), 1);
        assert_eq!(defaults.mock_response_target_chars(), None);
        assert_eq!(defaults.mock_stream_delay_multiplier(), 1.0);
        assert_eq!(zero_chars.mock_response_target_chars(), None);
        assert_eq!(negative_delay.mock_stream_delay_multiplier(), 0.0);
        assert_eq!(non_finite_delay.mock_stream_delay_multiplier(), 1.0);
        let openai_chat = EngineChatParameters::default_for_provider("openai");
        assert_eq!(
            openai_chat.tool_execution,
            EngineToolExecutionMode::EngineOrchestrated,
            "engine-orchestrated is the only supported mode now that MCP has been removed"
        );
        assert_eq!(openai_chat.max_output_tokens, None);
        let mock_chat = EngineChatParameters::default_for_provider("mock");
        assert_eq!(
            mock_chat.tool_execution,
            EngineToolExecutionMode::EngineOrchestrated
        );
        assert_eq!(mock_chat.max_output_tokens, None);
        assert_eq!(mock_chat.max_tool_rounds(), 8);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn emit_helpers_are_noops_without_a_logger() {
        let context = sample_context();
        let request = sample_request();
        let error = EngineAdapterRequestError::new("openai", "broken", "request");

        emit_request(
            None,
            &context,
            &request,
            Some("schema"),
            Some(json!({"ok":true})),
        )
        .await;
        emit_response(
            None,
            &context,
            request.operation,
            200,
            EngineAdapterResponseDetail {
                ok: true,
                request_id: Some("req-1".to_string()),
                response_id: Some("resp-1".to_string()),
                output_chars: Some(12),
                usage: Some(json!({"tokens": 12})),
                streaming: Some(false),
                stop_reason: Some("stop".to_string()),
            },
        )
        .await;
        emit_stream_event(
            None,
            &context,
            request.operation,
            "response.delta",
            json!({"delta":"hi"}),
        )
        .await;
        emit_error(None, &context, request.operation, &error).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn emit_helpers_preserve_structured_payload_shapes_with_logger() {
        let context = sample_context();
        let request = sample_request();
        let concrete = Arc::new(RecordingLogger::default());
        let logger: Arc<dyn EngineAdapterLogger> = concrete.clone();
        let error = EngineAdapterRequestError::new("openai", "bad request", "request")
            .with_status_code(400)
            .with_request_id(Some("req-42".to_string()));

        emit_request(
            Some(&logger),
            &context,
            &request,
            Some("story_schema"),
            Some(json!({"model":"logged-model"})),
        )
        .await;
        emit_response(
            Some(&logger),
            &context,
            request.operation,
            200,
            EngineAdapterResponseDetail {
                ok: true,
                request_id: Some("req-42".to_string()),
                response_id: Some("resp-99".to_string()),
                output_chars: Some(18),
                usage: Some(json!({"input_tokens": 7})),
                streaming: Some(true),
                stop_reason: Some("end_turn".to_string()),
            },
        )
        .await;
        emit_stream_event(
            Some(&logger),
            &context,
            request.operation,
            "response.output_text.delta",
            json!({"delta":"hello"}),
        )
        .await;
        emit_error(Some(&logger), &context, request.operation, &error).await;

        let requests = concrete.requests.lock().expect("lock requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].context, context);
        assert_eq!(requests[0].operation, EngineOperation::FlowTextStream);
        assert!(requests[0].detail.stream);
        assert_eq!(
            requests[0].detail.schema_name.as_deref(),
            Some("story_schema")
        );
        assert_eq!(requests[0].detail.max_output_tokens, Some(256));
        assert_eq!(
            requests[0].detail.instructions_chars,
            "System prompt".chars().count()
        );
        assert_eq!(
            requests[0].detail.user_content_chars,
            "User request".chars().count()
        );
        assert_eq!(
            requests[0].detail.body,
            Some(json!({"model":"logged-model"}))
        );
        assert_eq!(requests[0].detail.rules, Some(json!({"mode":"strict"})));
        drop(requests);

        let responses = concrete.responses.lock().expect("lock responses");
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].status, 200);
        assert_eq!(responses[0].detail.response_id.as_deref(), Some("resp-99"));
        assert_eq!(responses[0].detail.output_chars, Some(18));
        assert_eq!(responses[0].detail.stop_reason.as_deref(), Some("end_turn"));
        drop(responses);

        let events = concrete.stream_events.lock().expect("lock events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "response.output_text.delta");
        assert_eq!(events[0].detail, json!({"delta":"hello"}));
        drop(events);

        let errors = concrete.errors.lock().expect("lock errors");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].detail.message, "bad request");
        assert_eq!(errors[0].detail.status_code, Some(400));
        assert_eq!(errors[0].detail.request_id.as_deref(), Some("req-42"));
        assert_eq!(errors[0].detail.classification.as_str(), "request");
    }
}
