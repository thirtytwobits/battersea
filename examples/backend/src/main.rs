//! A standalone application: explicitly registered providers, a local tool and media generation.
use battersea_model::{engine::*, media::*, *};
use battersea_tools::ToolRegistry;
use clapham::{Bounds, Call, Contribution, Effect, OAuthContext, TypedOperation};
use futures_util::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// The text the echo operation is called with, and returns.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Text {
    text: String,
}
fn auth() -> EngineAuthConfig {
    EngineAuthConfig {
        auth_type: "none".into(),
        api_key_env: String::new(),
        header: None,
        version_header: None,
        version: None,
        has_api_key: false,
        api_key: None,
    }
}
fn chat_backend() -> EngineBackendConfig {
    EngineBackendConfig {
        binding: None,
        id: "example-chat".into(),
        provider: "mock".into(),
        label: "Example chat".into(),
        enabled: true,
        endpoint: String::new(),
        model: "example".into(),
        display_order: None,
        chat: EngineChatParameters::default_for_provider("mock"),
        capabilities: EngineBackendCapabilities::mock(),
        context_window_tokens: 8192,
        options: EngineBackendOptions {
            timeout_ms: Some(1000),
            mock_tool_calls: Some(vec![MockToolCallSpec {
                name: "example.echo".into(),
                arguments: json!({"text":"tool result"}),
                round: 1,
            }]),
            mock_stream_delay_multiplier: Some(0.0),
            ..Default::default()
        },
        auth: auth(),
        short_description: String::new(),
        long_description: String::new(),
    }
}
async fn demonstrate() -> Result<(), Box<dyn std::error::Error>> {
    let mut providers = ProviderRegistry::default();
    battersea_providers::register_builtin_providers(&mut providers)?;
    let echo = TypedOperation::<Text, Text>::new(
        "example.echo",
        "Echo the supplied text.",
        Effect::Query,
    )?
    .title("Echo text");
    let set = clapham::compose(
        [Contribution::new("example")
            .typed_operation(echo, |_: Call, text: Text| async move { Ok(text) })],
        Bounds::default(),
    )?;
    let selection = set.select(["example.echo"])?;
    // The host verifies the grant each run's tool calls are made under.
    let grant = OAuthContext::from_verified(
        "https://identity.example.com",
        "https://backend.example.com",
        "example-agent",
        Vec::<String>::new(),
    );
    let tools = ToolRegistry::new(set, selection, grant)?;
    let backend = chat_backend();
    let adapter = providers.create_chat(backend.clone(), None)?;
    let input = "A quiet landscape.";
    let mut stream = adapter
        .stream_text(EngineTextStreamRequest {
            shared: EngineAdapterRequest {
                operation: EngineOperation::FlowTextStream,
                messages: vec![Message::text(Role::User, input.into())],
                chat: backend.chat,
                debug_rules: None,
            },
            local_tools: tools.definitions(),
            local_tool_executor: Some(Arc::new(tools)),
            max_tool_rounds: 2,
            mock_response: None,
        })
        .await?;
    let mut completed_tool = false;
    let mut output = String::new();
    while let Some(event) = stream.next().await {
        match event? {
            EngineTextStreamEvent::ToolCallCompleted { ok, result, .. } => {
                assert!(ok);
                assert!(result.get("text").is_some());
                completed_tool = true;
            }
            EngineTextStreamEvent::TextDelta { text } => output.push_str(&text),
            _ => {}
        }
    }
    assert!(completed_tool && output.contains(input) && output.contains("tool result"));
    let media_backend = MediaBackendConfig {
        binding: None,
        id: "example-image".into(),
        provider: "mock".into(),
        capability: MediaCapability::ImageGeneration,
        label: "Example image".into(),
        enabled: true,
        endpoint: String::new(),
        model: "example".into(),
        capabilities: MediaBackendCapabilities::mock(),
        options: EngineBackendOptions {
            timeout_ms: Some(1000),
            extra: std::collections::BTreeMap::from([(
                "mockUrl".into(),
                json!("https://example.invalid/image.png"),
            )]),
            ..Default::default()
        },
        auth: auth(),
        short_description: String::new(),
        long_description: String::new(),
    };
    let generator = providers.create_media(media_backend)?;
    let prepared = generator.prepare(MediaRenderRequest {
        kind: MediaKind::Image,
        prompt_text: input.into(),
        negative_prompt: None,
        references: vec![],
        options: MediaGenerationHints::default(),
    })?;
    let submission = generator
        .submit(prepared, CancellationToken::new(), None, None)
        .await?;
    let MediaSubmission::Complete(result) = submission else {
        return Err("Mock generation must complete synchronously".into());
    };
    assert!(!result.assets.is_empty());
    assert!(result
        .assets
        .iter()
        .all(|asset| asset.media_type == MediaRenderType::Image));
    println!("Chat, registered tool and media completed independently.");
    Ok(())
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    demonstrate().await
}
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn standalone_backend_contract() {
        super::demonstrate().await.unwrap();
    }
}
