//! Session-independent node execution and host contracts.
mod delivery;
mod failure;
pub use battersea_flow::FlowFailurePolicy;
pub use delivery::ProviderEvent;
pub use failure::{HandledNodeFailure, RecoverableNodeFailure};
pub mod checkpoint;
pub mod durable;
pub mod error_codes;
pub mod recovery;
pub mod retention;
pub use retention::{Retained, RetentionBudget};
mod event;
mod handler;
mod lifecycle;
pub use event::{EventKind, ExecutionEvent};
pub use lifecycle::{ActivationHost, Outcome, RunIdentity, RunPhase, RunRecord};
pub mod dispatch;
mod host;
pub mod payload_pump;
pub mod pump;
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

/// A host's error, raised by the runtime through these constructors.
///
/// Each refusal names its failure with a `reason` from [`error_codes`], or a host's own code
/// for a host refusal; the message is for people. [`internal`](Self::internal) reports a
/// fault and carries no reason.
pub trait ExecutionError: std::error::Error + Send + Sync + 'static {
    /// Opt in only for a recoverable node refusal with a host reason code and settled
    /// effects. Cancellation, interruption, admission and storage errors remain unclassified.
    fn recoverable_node_failure(&self) -> Option<RecoverableNodeFailure> {
        None
    }
    /// The request cannot be carried out.
    fn invalid_request(reason: &'static str, message: impl Into<String>) -> Self;
    /// A fault in the runtime or host.
    fn internal(message: impl Into<String>) -> Self;
    /// The activation was cancelled.
    fn cancelled(reason: &'static str, message: impl Into<String>) -> Self;
    /// The activation stopped before an outcome was decided.
    fn interrupted(reason: &'static str, message: impl Into<String>) -> Self;
    /// The activation stopped where its journal must be recovered before it can continue.
    fn recovery_required(reason: &'static str, message: impl Into<String>) -> Self
    where
        Self: Sized,
    {
        Self::interrupted(reason, message)
    }
    /// The error's class, chosen by the host for each constructor.
    fn code(&self) -> &str;
    /// The reason the error was raised with; `None` for an [`internal`](Self::internal) fault.
    fn reason_code(&self) -> Option<&str>;
    fn message(&self) -> &str;
}
