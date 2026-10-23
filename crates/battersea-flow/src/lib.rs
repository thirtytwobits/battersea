//! Flow documents, extensible catalogues and pure validation.
#![forbid(unsafe_code)]
mod types;
pub use types::*;
pub mod catalog;
pub mod document;
pub mod execution;
pub mod failure;
pub use execution::{
    FlowExecutionLimits, FlowExecutionPolicy, FlowPortMode, FlowPortPhase, FlowQueueLimits,
    FlowQueuePolicy,
};
pub use failure::{FlowFailurePolicy, FlowFailurePort, FlowNodeFailure};
pub mod dsl;
pub mod issue_codes;
pub mod output_encoding;
pub mod ports;
pub mod prompt_markdown;
pub mod registry;
pub mod schema;
pub mod template_engine;
pub mod validation;
