use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct EngineAdapterRequestError {
    pub provider: String,
    pub message: String,
    pub classification: ErrorKind,
    pub status_code: Option<u16>,
    pub request_id: Option<String>,
}

impl EngineAdapterRequestError {
    pub fn new(
        provider: impl Into<String>,
        message: impl Into<String>,
        classification: impl Into<ErrorKind>,
    ) -> Self {
        Self {
            provider: provider.into(),
            message: message.into(),
            classification: classification.into(),
            status_code: None,
            request_id: None,
        }
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
    Other(String),
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
