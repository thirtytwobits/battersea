use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct EngineAdapterRequestError {
    pub provider: Box<str>,
    pub message: Box<str>,
    pub classification: ErrorKind,
    pub status_code: Option<u16>,
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watchdog: Option<Box<WatchdogFailure>>,
    #[serde(default)]
    pub dispatch: DispatchState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default)]
    pub attempts: u32,
    /// The tool runner's name for this refusal; `classification` is its class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_code: Option<ToolCode>,
}

impl EngineAdapterRequestError {
    pub fn new(
        provider: impl Into<String>,
        message: impl Into<String>,
        classification: impl Into<ErrorKind>,
    ) -> Self {
        let classification = classification.into();
        let dispatch = if classification == ErrorKind::InvalidRequest {
            DispatchState::NotSent
        } else {
            DispatchState::Unknown
        };
        Self {
            provider: provider.into().into_boxed_str(),
            message: message.into().into_boxed_str(),
            classification,
            status_code: None,
            request_id: None,
            watchdog: None,
            dispatch,
            retry_after_ms: None,
            attempts: 0,
            tool_code: None,
        }
    }

    /// Names a refusal of the tool runner.
    pub fn with_tool_code(mut self, code: ToolCode) -> Self {
        self.tool_code = Some(code);
        self
    }

    pub fn with_dispatch(mut self, dispatch: DispatchState) -> Self {
        self.dispatch = dispatch;
        self
    }

    pub fn with_idle_watchdog(mut self, limit_ms: u64) -> Self {
        self.watchdog = Some(Box::new(WatchdogFailure { limit_ms }));
        self
    }
    pub fn with_status_code(mut self, status_code: u16) -> Self {
        self.status_code = Some(status_code);
        self
    }

    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }

    pub fn invalid_response(provider: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(provider, message, "invalid_response")
    }

    pub fn transport(provider: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(provider, message, "transport")
    }

    pub fn mock(message: impl Into<String>) -> Self {
        Self::new("mock", message, "mock")
    }
}

/// The refusals with which tool admission, the tool registry and the shared tool loop refuse
/// a request or a provider's tool call. Each serialises as its stable snake_case code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolCode {
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
        Self::UnregisteredTool,
        Self::ToolTranscriptTooLarge,
        Self::ToolRoundsExceeded,
    ];
    /// The stable snake_case code.
    pub fn as_str(self) -> &'static str {
        match self {
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
    Other(Box<str>),
}
impl ErrorKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidResponse => "invalid_response",
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
            Self::Other(value) => value,
        }
    }
}
impl From<&str> for ErrorKind {
    fn from(value: &str) -> Self {
        match value {
            "invalid_request" => Self::InvalidRequest,
            "invalid_response" => Self::InvalidResponse,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchdogFailure {
    pub limit_ms: u64,
}

/// What is known about the remote effect, independently of the failure category.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DispatchState {
    NotSent,
    Rejected,
    Accepted,
    #[default]
    Unknown,
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
