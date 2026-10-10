//! Chat and media providers using Battersea's shared model and tool contracts.
pub mod adapter;
#[cfg(any(
    feature = "openai",
    feature = "anthropic",
    feature = "google",
    feature = "runway"
))]
mod http_payload;
pub mod media;
pub mod registry;
#[cfg(feature = "mock")]
pub use adapter::mock::estimate_mock_token_count;
#[cfg(test)]
#[path = "test_endpoints.rs"]
mod test_endpoints;
pub use battersea_model::adapter::error::EngineAdapterRequestError;
pub use battersea_model::adapter::*;
pub use battersea_model::media::*;
pub use registry::{builtin_registry, register_builtin_providers};
