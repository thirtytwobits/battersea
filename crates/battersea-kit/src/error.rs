//! Copyright (c) Scott A Dixon
//!
//! The refusal type a kit's handlers return before the host maps it to its own error.

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    /// A refusal named by a code from [`battersea_runtime::error_codes`].
    #[error("{failure}")]
    InvalidRequest {
        reason: &'static str,
        failure: battersea_runtime::Failure,
    },
    #[error("{0}")]
    Internal(battersea_runtime::Failure),
}
pub type NodeResult<T> = Result<T, NodeError>;
impl NodeError {
    pub fn internal(failure: impl Into<battersea_runtime::Failure>) -> Self {
        Self::Internal(failure.into())
    }
    pub fn invalid_request(
        reason: &'static str,
        failure: impl Into<battersea_runtime::Failure>,
    ) -> Self {
        Self::InvalidRequest {
            reason,
            failure: failure.into(),
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
            Self::InvalidRequest { reason, failure } => E::invalid_request(reason, failure),
            Self::Internal(failure) => E::internal(failure),
        }
    }
}
