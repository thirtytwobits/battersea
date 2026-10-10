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
        }
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
