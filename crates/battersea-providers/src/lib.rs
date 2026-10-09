//! Chat and media providers using Battersea's shared model and tool contracts.
pub mod adapter;
pub mod media;
pub mod registry;
#[cfg(test)]
#[path = "test_endpoints.rs"]
mod test_endpoints;
pub use battersea_model::adapter::error::EngineAdapterRequestError;
pub use battersea_model::adapter::*;
pub use battersea_model::media::*;
pub use registry::{builtin_registry, register_builtin_providers};
