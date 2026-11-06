//! Copyright (c) Scott A Dixon
//!
//! The refusal type a kit's handlers return before the host maps it to its own error.

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    /// A refusal named by a code from [`battersea_runtime::error_codes`].
    #[error("{message}")]
    InvalidRequest {
        reason: &'static str,
        message: String,
    },
    #[error("{0}")]
    Internal(String),
}
pub type NodeResult<T> = Result<T, NodeError>;
impl NodeError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
    pub fn invalid_request(reason: &'static str, message: impl Into<String>) -> Self {
        Self::InvalidRequest {
            reason,
            message: message.into(),
        }
    }
    /// The code naming a refusal; `None` for an internal fault.
    pub fn reason_code(&self) -> Option<&'static str> {
        match self {
            Self::InvalidRequest { reason, .. } => Some(reason),
            Self::Internal(_) => None,
        }
    }
    /// The host's error for this refusal.
    pub fn into_host<E: battersea_runtime::ExecutionError>(self) -> E {
        match self {
            Self::InvalidRequest { reason, message } => E::invalid_request(reason, message),
            Self::Internal(message) => E::internal(message),
        }
    }
}
