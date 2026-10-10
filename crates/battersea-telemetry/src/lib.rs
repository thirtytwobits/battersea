//! Runtime observations are independent of tracing filters and exporters.
//! Hosts persist accounting transactions before making their effects visible.
pub mod accounting;
pub mod capture;
pub mod config;
pub mod content;
pub mod execution;
pub mod otlp;
pub mod prices;
pub mod schema;
pub mod usage;
pub mod view;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("Invalid telemetry configuration: {0}")]
    Invalid(String),
    #[error("Telemetry retention capacity exceeded")]
    Capacity,
    #[error("Runtime cursor requires a snapshot")]
    ResyncRequired,
    #[error("Budget admission refused: {0}")]
    Budget(String),
    #[error("Accounting arithmetic overflow")]
    Overflow,
    #[error("Conflicting request accounting transition")]
    Conflict,
    #[error("Telemetry persistence or export failed: {0}")]
    Delivery(String),
}
pub type Result<T> = std::result::Result<T, Error>;
