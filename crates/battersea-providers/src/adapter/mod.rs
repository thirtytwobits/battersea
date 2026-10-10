pub use battersea_model::adapter::*;
#[cfg(feature = "anthropic")]
pub(crate) mod anthropic;
#[cfg(feature = "google")]
pub(crate) mod google;
#[cfg(feature = "openai")]
pub(crate) mod openai;
#[cfg(feature = "anthropic")]
pub(crate) use anthropic::create_anthropic_adapter;
#[cfg(feature = "google")]
pub(crate) use google::create_google_adapter;
#[cfg(feature = "openai")]
pub(crate) use openai::create_openai_adapter;

#[cfg(feature = "mock")]
pub(crate) mod mock;
