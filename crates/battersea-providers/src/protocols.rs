//! Concrete protocol registrations. Service identities are supplied by configuration.
use battersea_model::{
    ProtocolAdapter, ProtocolRegistry, ProviderConfigurationError, TokenCounting,
};
use serde_json::{json, Value};

pub fn register_builtin_adapters(
    registry: &mut ProtocolRegistry,
) -> Result<(), ProviderConfigurationError> {
    let legacy = crate::builtin_registry();
    for (id, provider, chat, media, counting) in [
        (
            "openai-responses",
            "openai",
            true,
            false,
            TokenCounting::Native,
        ),
        (
            "anthropic-messages",
            "anthropic",
            true,
            false,
            TokenCounting::Native,
        ),
        (
            "google-generate-content",
            "google",
            true,
            false,
            TokenCounting::Native,
        ),
        (
            "openai-images",
            "openai",
            false,
            true,
            TokenCounting::Unavailable,
        ),
        (
            "google-interactions",
            "google",
            false,
            true,
            TokenCounting::Unavailable,
        ),
        (
            "runway-tasks",
            "runway",
            false,
            true,
            TokenCounting::Unavailable,
        ),
        (
            "replicate-predictions",
            "replicate",
            false,
            true,
            TokenCounting::Unavailable,
        ),
        ("mock", "mock", true, true, TokenCounting::Estimate),
    ] {
        let chat = chat
            .then(|| legacy.chat_provider(provider).ok().cloned())
            .flatten();
        let media = media
            .then(|| legacy.media_provider(provider).ok().cloned())
            .flatten();
        if chat.is_none() && media.is_none() {
            continue;
        }
        registry.register(ProtocolAdapter {
            id: id.into(),
            contract_version: 1,
            chat_streaming: if id == "openai-responses" {
                vec![true, false]
            } else {
                vec![true]
            },
            context_overflow: if id == "openai-responses" {
                vec![
                    battersea_model::engine::EngineContextOverflow::Error,
                    battersea_model::engine::EngineContextOverflow::ProviderTruncate,
                ]
            } else {
                vec![battersea_model::engine::EngineContextOverflow::Error]
            },
            chat,
            media,
            counting,
            requires_endpoint: provider != "mock",
            required_headers: match id {
                "anthropic-messages" => vec!["anthropic-version".into()],
                "runway-tasks" => vec!["x-runway-version".into()],
                _ => vec![],
            },
            options_schema: options_schema(id),
        })?;
    }
    #[cfg(feature = "openai")]
    registry.register(ProtocolAdapter {
        id: "openai-chat-completions".into(),
        contract_version: 1,
        chat_streaming: vec![true, false],
        context_overflow: vec![battersea_model::engine::EngineContextOverflow::Error],
        chat: Some(crate::adapter::completions::registration()),
        media: None,
        counting: TokenCounting::Unavailable,
        requires_endpoint: true,
        required_headers: vec![],
        options_schema: options_schema("openai-chat-completions"),
    })?;
    Ok(())
}

fn options_schema(id: &str) -> Value {
    let mut properties = serde_json::Map::new();
    let mut add = |names: &str, schema: Value| {
        for name in names.split_whitespace() {
            properties.insert(name.into(), schema.clone());
        }
    };
    let chat = matches!(
        id,
        "openai-responses"
            | "openai-chat-completions"
            | "anthropic-messages"
            | "google-generate-content"
    );
    add("timeoutMs", json!({"type":"integer","minimum":1}));
    if chat || id == "mock" {
        add(
            "streamIdleTimeoutMs retryInitialDelayMs retryMaxDelayMs",
            json!({"type":"integer","minimum":1}),
        );
        add(
            "maxRetries",
            json!({"type":"integer","minimum":0,"maximum":100}),
        );
    }
    if id == "openai-responses" {
        add("background", json!({"type":"boolean"}));
        add("backgroundMode", json!({"enum":["poll","stream"]}));
        add(
            "backgroundReconnectIdleMs backgroundPollIntervalMs backgroundMaxWaitMs",
            json!({"type":"integer","minimum":1}),
        );
        add(
            "backgroundMaxReconnects",
            json!({"type":"integer","minimum":0}),
        );
    }
    if matches!(id, "openai-images" | "google-interactions") {
        add(
            "estimatedProgressIntervalMs",
            json!({"type":"integer","minimum":1}),
        );
    }
    if matches!(id, "runway-tasks" | "replicate-predictions") {
        add(
            "pollIntervalMs pollRetryDelayMs maxPollRetryDelayMs",
            json!({"type":"integer","minimum":1}),
        );
        add(
            "maxOutputBytes",
            json!({"type":"integer","minimum":1,"maximum":battersea_model::adapter::payload::PROVIDER_PAYLOAD_BYTES}),
        );
        add(
            "maxPollRetryCount",
            json!({"type":"integer","minimum":0,"maximum":100}),
        );
    }
    match id {
        "openai-images" => {
            add(
                "defaultCount",
                json!({"type":"integer","minimum":1,"maximum":255}),
            );
            add(
                "partialImages",
                json!({"type":"integer","minimum":0,"maximum":3}),
            );
            add(
                "outputCompression",
                json!({"type":"integer","minimum":0,"maximum":100}),
            );
            add(
                "quality",
                json!({"enum":["auto","low","medium","high","xhigh","max"]}),
            );
            add("outputFormat", json!({"enum":["png","jpeg","jpg","webp"]}));
            add("moderation", json!({"enum":["auto","low"]}));
            add("user", json!({"type":"string"}));
        }
        "google-interactions" => {
            add(
                "defaultMimeType",
                json!({"type":"string","pattern":"^image/[^ ]+$"}),
            );
            add("imageSize", json!({"enum":["0.5K","1K","2K","4K"]}));
        }
        "runway-tasks" => {
            add("transport", json!({"enum":["model-router","direct"]}));
            add(
                "configId resolution defaultAspectRatio defaultVideoRatio defaultVideoReferenceRatio",
                json!({"type":"string","minLength":1}),
            );
            add(
                "maxPromptChars defaultVideoDurationSeconds",
                json!({"type":"integer","minimum":1}),
            );
        }
        "replicate-predictions" => {
            add("input", json!({"type":"object"}));
            add(
                "inputFields",
                json!({"type":"object","additionalProperties":{"type":"string","minLength":1},"required":["prompt"]}),
            );
            add("outputPointer webhookUrl", json!({"type":"string"}));
        }
        "mock" => {
            add(
                "mockDelayMs mockProgressDurationMs mockProgressStepCount",
                json!({"type":"integer","minimum":0}),
            );
            add("mockSequence", json!({"type":"array"}));
            add("mockProgressTicks", json!({"type":"boolean"}));
            add(
                "mockResponseTargetChars",
                json!({"type":"integer","minimum":1}),
            );
            add(
                "mockStreamDelayMultiplier",
                json!({"type":"number","minimum":0}),
            );
            add(
                "mockStreamingReasoningChunks",
                json!({"type":"array","items":{"type":"string"}}),
            );
            add("mockToolCalls", json!({"type":"array"}));
            add("mockFailure", json!({"type":"object"}));
            add("mockUrl fontPath", json!({"type":"string"}));
        }
        _ => (),
    }
    let mut required = vec!["timeoutMs"];
    if chat {
        required.extend(["maxRetries", "streamIdleTimeoutMs"]);
    }
    if matches!(id, "runway-tasks" | "replicate-predictions") {
        required.extend(["pollIntervalMs", "maxPollRetryCount"]);
    }
    match id {
        "replicate-predictions" => required.push("inputFields"),
        "openai-images" => required.extend(["defaultCount", "partialImages", "quality"]),
        "google-interactions" => required.push("defaultMimeType"),
        "runway-tasks" => required.push("maxPromptChars"),
        _ => (),
    }
    let mut schema = json!({"type":"object","additionalProperties":false,"properties":properties,"required":required});
    if id == "openai-responses" {
        schema["allOf"] = json!([
            {"if":{"required":["background"],"properties":{"background":{"const":true}}},"then":{"required":["backgroundMode","backgroundMaxWaitMs"]}},
            {"if":{"required":["backgroundMode"],"properties":{"backgroundMode":{"const":"poll"}}},"then":{"required":["backgroundPollIntervalMs"]}},
            {"if":{"required":["backgroundMode"],"properties":{"backgroundMode":{"const":"stream"}}},"then":{"required":["backgroundReconnectIdleMs","backgroundMaxReconnects"]}}
        ]);
    }
    schema
}
