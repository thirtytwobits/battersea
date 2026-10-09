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

#[cfg(any(feature = "openai", feature = "anthropic"))]
fn text_messages(request: &EngineAdapterRequest) -> Vec<serde_json::Value> {
    request
        .messages
        .iter()
        .filter(|m| m.role != battersea_model::Role::System)
        .map(|m| serde_json::json!({"role": m.role, "content": m.text_content()}))
        .collect()
}
#[cfg(feature = "google")]
fn google_messages(request: &EngineAdapterRequest) -> Vec<serde_json::Value> {
    request.messages.iter().filter(|m| m.role != battersea_model::Role::System).map(|m| serde_json::json!({"role": if m.role == battersea_model::Role::Assistant { "model" } else { "user" }, "parts": [{"text": m.text_content()}]})).collect()
}
#[cfg(feature = "mock")]
pub(crate) mod mock;
