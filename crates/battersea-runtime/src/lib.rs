//! Session-independent node execution and host contracts.
mod event;
mod handler;
mod lifecycle;
pub use event::{EventKind, ExecutionEvent};
pub use lifecycle::{ActivationHost, Outcome, RunIdentity, RunPhase, RunRecord};
pub mod dispatch;
mod host;
mod registry;
mod state;
pub use handler::NodeHandler;
pub use host::{order_source_phase_materialization, ExecutionHost};
pub use registry::{ExecutableCatalog, HandlerRegistry, RegistryBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use state::SchedulerState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token_type: String,
    pub value: Value,
}

pub trait ExecutionError: std::error::Error + Send + Sync + 'static {
    fn invalid_request(message: impl Into<String>) -> Self;
    fn internal(message: impl Into<String>) -> Self;
    fn cancelled(message: impl Into<String>) -> Self;
    fn interrupted(message: impl Into<String>) -> Self;
    fn code(&self) -> &str;
    fn message(&self) -> &str;
}
