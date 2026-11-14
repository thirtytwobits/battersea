//! Copyright (c) Scott A Dixon
//!
//! Classifications and evidence a failure carries.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The refusals with which tool admission, the tool registry and the shared tool loop refuse
/// a request or a provider's tool call. Each serialises as its stable snake_case code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolCode {
    InvalidToolContinuation,
    ToolContinuationConflict,
    ToolContinuationStorage,
    ToolContinuationUnresolved,
    ToolContinuationExpired,
    UnsupportedToolContinuation,

    /// The durable approval journal or request revision changed.
    ToolApprovalConflict,
    /// An approval storage operation requires inspection before continuing.
    ToolApprovalStorage,
    /// A durable tool dispatch intent requires reconciliation.
    ToolApprovalUnresolved,

    /// The host supplied an invalid approval scope, capacity or journal.
    InvalidToolApproval,
    /// The host denied the validated tool call.
    ToolApprovalDenied,
    /// The validated tool call requires a host decision before dispatch.
    ToolApprovalPending,
    /// The approval scope expired before dispatch.
    ToolApprovalExpired,
    /// The approval executor exhausted its call or payload capacity.
    ToolApprovalCapacity,

    /// A request's tool round limit is outside the supported range.
    InvalidToolRounds,
    /// Two tools share a name, or a tool's name is blank, once names are normalised.
    DuplicateTool,
    /// A tool's input schema is not a valid JSON Schema.
    InvalidToolSchema,
    /// A tool's record cannot be offered: its name is outside the tool-name alphabet, it has
    /// no description, its input schema is not an object schema, or its profiles are not
    /// distinct names.
    InvalidToolDefinition,
    /// The request's tool choice names a tool the request does not offer.
    UnofferedToolChoice,
    /// The provider returned a tool call without an id, name or object arguments.
    MalformedToolCall,
    /// The provider requested tools although tool use was disabled.
    ToolUseDisabled,
    /// The provider returned a missing or duplicate tool call id.
    InvalidToolCallId,
    /// The provider called a tool the model was not offered.
    UnofferedTool,
    /// The provider called a tool other than the one the tool choice names.
    ToolOutsideChoice,
    /// A tool call's arguments do not satisfy the tool's input schema.
    InvalidToolArguments,
    /// A tool's result does not satisfy the output schema the tool declares.
    InvalidToolResult,
    /// The provider conversation cannot represent the tool's returned content.
    UnsupportedToolContent,
    /// The registry holds no tool with the called name.
    UnregisteredTool,
    /// The tool calls and results of a run exceed their encoded-byte limit.
    ToolTranscriptTooLarge,
    /// The provider was still calling tools after the request's last tool round.
    ToolRoundsExceeded,
}
impl ToolCode {
    /// Every tool runner refusal.
    pub const ALL: &[Self] = &[
        Self::InvalidToolContinuation,
        Self::ToolContinuationConflict,
        Self::ToolContinuationStorage,
        Self::ToolContinuationUnresolved,
        Self::ToolContinuationExpired,
        Self::UnsupportedToolContinuation,
        Self::ToolApprovalConflict,
        Self::ToolApprovalStorage,
        Self::ToolApprovalUnresolved,
        Self::InvalidToolApproval,
        Self::ToolApprovalDenied,
        Self::ToolApprovalPending,
        Self::ToolApprovalExpired,
        Self::ToolApprovalCapacity,
        Self::InvalidToolRounds,
        Self::DuplicateTool,
        Self::InvalidToolSchema,
        Self::InvalidToolDefinition,
        Self::UnofferedToolChoice,
        Self::MalformedToolCall,
        Self::ToolUseDisabled,
        Self::InvalidToolCallId,
        Self::UnofferedTool,
        Self::ToolOutsideChoice,
        Self::InvalidToolArguments,
        Self::InvalidToolResult,
        Self::UnsupportedToolContent,
        Self::UnregisteredTool,
        Self::ToolTranscriptTooLarge,
        Self::ToolRoundsExceeded,
    ];
    /// The stable snake_case code.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidToolContinuation => "invalid_tool_continuation",
            Self::ToolContinuationConflict => "tool_continuation_conflict",
            Self::ToolContinuationStorage => "tool_continuation_storage",
            Self::ToolContinuationUnresolved => "tool_continuation_unresolved",
            Self::ToolContinuationExpired => "tool_continuation_expired",
            Self::UnsupportedToolContinuation => "unsupported_tool_continuation",

            Self::ToolApprovalConflict => "tool_approval_conflict",
            Self::ToolApprovalStorage => "tool_approval_storage",
            Self::ToolApprovalUnresolved => "tool_approval_unresolved",

            Self::InvalidToolApproval => "invalid_tool_approval",
            Self::ToolApprovalDenied => "tool_approval_denied",
            Self::ToolApprovalPending => "tool_approval_pending",
            Self::ToolApprovalExpired => "tool_approval_expired",
            Self::ToolApprovalCapacity => "tool_approval_capacity",

            Self::InvalidToolRounds => "invalid_tool_rounds",
            Self::DuplicateTool => "duplicate_tool",
            Self::InvalidToolSchema => "invalid_tool_schema",
            Self::InvalidToolDefinition => "invalid_tool_definition",
            Self::UnofferedToolChoice => "unoffered_tool_choice",
            Self::MalformedToolCall => "malformed_tool_call",
            Self::ToolUseDisabled => "tool_use_disabled",
            Self::InvalidToolCallId => "invalid_tool_call_id",
            Self::UnofferedTool => "unoffered_tool",
            Self::ToolOutsideChoice => "tool_outside_choice",
            Self::InvalidToolArguments => "invalid_tool_arguments",
            Self::InvalidToolResult => "invalid_tool_result",
            Self::UnsupportedToolContent => "unsupported_tool_content",
            Self::UnregisteredTool => "unregistered_tool",
            Self::ToolTranscriptTooLarge => "tool_transcript_too_large",
            Self::ToolRoundsExceeded => "tool_rounds_exceeded",
        }
    }
}
impl std::fmt::Display for ToolCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Dispatch failures are classified independently from their diagnostic text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidRequest,
    InvalidResponse,
    /// The provider declined to produce the result.
    Refusal,
    /// The provider stopped before the result was complete.
    Incomplete,
    Authentication,
    Permission,
    RateLimit,
    Request,
    Server,
    Transport,
    Timeout,
    Cancelled,
    Provider,
    Expired,
    Retrieval,
    Mock,
    /// Raised by the flow runtime or a host; the host error's code is its class.
    Runtime,
    Other(Box<str>),
}
impl ErrorKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidResponse => "invalid_response",
            Self::Refusal => "refusal",
            Self::Incomplete => "incomplete",
            Self::Authentication => "auth",
            Self::Permission => "permission",
            Self::RateLimit => "rate_limit",
            Self::Server => "server",
            Self::Request => "request",
            Self::Transport => "transport",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::Retrieval => "retrieval",
            Self::Provider => "provider",
            Self::Mock => "mock",
            Self::Runtime => "runtime",
            Self::Other(value) => value,
        }
    }
}
impl From<&str> for ErrorKind {
    fn from(value: &str) -> Self {
        match value {
            "invalid_request" => Self::InvalidRequest,
            "invalid_response" => Self::InvalidResponse,
            "refusal" => Self::Refusal,
            "incomplete" => Self::Incomplete,
            "authentication" | "auth" => Self::Authentication,
            "permission" => Self::Permission,
            "server" => Self::Server,
            "rate_limit" => Self::RateLimit,
            "request" => Self::Request,
            "transport" => Self::Transport,
            "timeout" => Self::Timeout,
            "cancelled" => Self::Cancelled,
            "expired" => Self::Expired,
            "retrieval" => Self::Retrieval,
            "provider" => Self::Provider,
            "mock" => Self::Mock,
            "runtime" => Self::Runtime,
            value => Self::Other(value.into()),
        }
    }
}
impl From<String> for ErrorKind {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}
impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl serde::Serialize for ErrorKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl<'de> serde::Deserialize<'de> for ErrorKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <String as serde::Deserialize>::deserialize(deserializer).map(Self::from)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WatchdogFailure {
    pub limit_ms: u64,
}

/// What is known about the remote effect, independently of the failure category.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DispatchState {
    NotSent,
    Rejected,
    Accepted,
    #[default]
    Unknown,
}

/// What is known about a tool invocation, independently from result delivery and durability.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallOutcome {
    /// The executor or transport confirms the call was not dispatched.
    NotDispatched,
    /// The executor returned a complete result, including a tool-reported execution error.
    Completed,
    /// The call may have run; cancellation and connection loss do not resolve its effects.
    Uncertain,
}

/// Host evidence about one tool call. Completion does not establish durable publication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ToolCallEvidence {
    pub call_id: String,
    pub tool_name: String,
    pub outcome: ToolCallOutcome,
    /// The remote transport's request identity, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::ToolCode;
    use std::collections::HashSet;

    #[test]
    fn tool_codes_are_distinct_snake_case_names_that_serialise_as_themselves() {
        let mut seen = HashSet::new();
        for code in ToolCode::ALL {
            let name = code.as_str();
            assert!(seen.insert(name), "{name} is declared twice");
            assert!(
                name.split('_')
                    .all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase())),
                "{name} is not snake_case"
            );
            assert_eq!(serde_json::to_value(code).unwrap(), name);
            assert_eq!(
                serde_json::from_value::<ToolCode>(name.into()).unwrap(),
                *code
            );
        }
    }
}
