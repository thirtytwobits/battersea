//! Built-in registrations. Applications can register additional providers in the same registry.
use crate::adapter::*;
#[cfg(any(
    feature = "openai",
    feature = "google",
    feature = "runway",
    feature = "mock"
))]
use crate::media::*;
#[cfg(any(
    feature = "openai",
    feature = "anthropic",
    feature = "google",
    feature = "mock"
))]
use battersea_model::ChatProvider;
use battersea_model::ProviderRegistry;
#[cfg(any(
    feature = "openai",
    feature = "google",
    feature = "runway",
    feature = "mock"
))]
use battersea_model::{MediaProvider, MediaProviderCapabilities};
#[cfg(any(
    feature = "openai",
    feature = "anthropic",
    feature = "google",
    feature = "runway",
    feature = "mock"
))]
use std::sync::Arc;
use std::sync::OnceLock;

pub fn builtin_registry() -> &'static ProviderRegistry {
    static REGISTRY: OnceLock<ProviderRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = ProviderRegistry::default();
        register_builtin_providers(&mut registry).expect("unique built-in provider identifiers");
        registry
    })
}
pub fn register_builtin_providers(
    registry: &mut ProviderRegistry,
) -> Result<(), error::EngineAdapterRequestError> {
    #[cfg(not(any(
        feature = "openai",
        feature = "anthropic",
        feature = "google",
        feature = "runway",
        feature = "mock"
    )))]
    let _ = registry;
    #[cfg(any(
        feature = "openai",
        feature = "anthropic",
        feature = "google",
        feature = "mock",
        feature = "runway"
    ))]
    fn parameters(values: &str) -> Vec<String> {
        values.split_whitespace().map(str::to_owned).collect()
    }
    #[cfg(feature = "openai")]
    registry.register_chat("openai", ChatProvider { content: crate::content::capabilities("openai"), background_modes: parameters("poll stream"), factory: Arc::new(create_openai_adapter), parameters: parameters("responseFormat outputModalities model stream maxOutputTokens temperature topP stopSequences toolExecution maxToolRounds toolChoice parallelToolCalls strictToolInputs serviceTier safetyIdentifier requestMetadata reasoningEffort reasoningSummary responseVerbosity contextOverflow promptCacheKey promptCacheRetention storeResponse maxProviderToolCalls logprobs topLogprobs") })?;
    #[cfg(feature = "anthropic")]
    registry.register_chat("anthropic", ChatProvider { content: crate::content::capabilities("anthropic"), background_modes: vec![], factory: Arc::new(create_anthropic_adapter), parameters: parameters("responseFormat outputModalities model stream maxOutputTokens temperature topP topK stopSequences toolExecution maxToolRounds toolChoice parallelToolCalls strictToolInputs serviceTier safetyIdentifier requestMetadata reasoningEffort thinkingBudgetTokens thinkingVisibility contextOverflow") })?;
    #[cfg(feature = "google")]
    registry.register_chat("google", ChatProvider { content: crate::content::capabilities("google"), background_modes: vec![], factory: Arc::new(create_google_adapter), parameters: parameters("responseFormat outputModalities model stream maxOutputTokens temperature topP topK stopSequences toolExecution maxToolRounds toolChoice reasoningEffort thinkingBudgetTokens thinkingVisibility contextOverflow") })?;
    #[cfg(feature = "mock")]
    registry.register_chat("mock", ChatProvider { content: crate::content::capabilities("mock"), background_modes: vec![], factory: Arc::new(crate::adapter::mock::create_mock_adapter), parameters: parameters("responseFormat outputModalities model stream maxOutputTokens temperature topP topK stopSequences toolExecution maxToolRounds toolChoice parallelToolCalls strictToolInputs serviceTier safetyIdentifier requestMetadata reasoningEffort reasoningSummary thinkingBudgetTokens thinkingVisibility responseVerbosity contextOverflow promptCacheKey promptCacheRetention storeResponse maxProviderToolCalls logprobs topLogprobs") })?;
    #[cfg(feature = "openai")]
    registry.register_media(
        "openai",
        MediaProvider {
            parameters: parameters("backend count size aspect_ratio"),
            factory: Arc::new(create_openai_media_generator),
            prepare: Arc::new(|backend, request| {
                let body = crate::media::openai::prepare_body(backend, request.clone())?;
                prepared(
                    request,
                    "/images/generations",
                    body,
                    "/prompt",
                    &backend.provider,
                )
            }),
            capabilities: MediaProviderCapabilities {
                kinds: vec![MediaKind::Image],
                references: false,
                negative_prompt: false,
                partial_images: true,
                remote_jobs: false,
                remote_cancellation: false,
            },
            accepts_aspect_ratio: |_, value| {
                crate::media::openai::parse_size(Some(value)).is_some()
            },
            accepts_size: |_, value| {
                value == "auto" || crate::media::openai::parse_explicit_size(Some(value)).is_some()
            },
        },
    )?;
    #[cfg(feature = "google")]
    registry.register_media(
        "google",
        MediaProvider {
            parameters: parameters("backend aspect_ratio"),
            factory: Arc::new(create_google_media_generator),
            prepare: Arc::new(|backend, request| {
                let body = crate::media::google::build_interaction_body(backend, &request)?;
                prepared(
                    request,
                    "/v1beta/interactions",
                    body,
                    "/input",
                    &backend.provider,
                )
            }),
            capabilities: MediaProviderCapabilities {
                kinds: vec![MediaKind::Image],
                references: false,
                negative_prompt: false,
                partial_images: true,
                remote_jobs: false,
                remote_cancellation: false,
            },
            accepts_aspect_ratio: |_, value| {
                crate::media::google::normalize_aspect_ratio(Some(value)).is_some()
            },
            accepts_size: |_, _| false,
        },
    )?;
    #[cfg(feature = "runway")]
    registry.register_media(
        "runway",
        MediaProvider {
            parameters: parameters("backend count aspect_ratio seed duration_seconds"),
            factory: Arc::new(create_runway_media_generator),
            prepare: Arc::new(|backend, request| {
                let path = crate::media::runway::build_submit_path(backend, &request)?;
                let body = crate::media::runway::build_submit_body(backend, &request)?;
                let pointer = if body.get("configId").is_some() {
                    "/input/promptText"
                } else {
                    "/promptText"
                };
                prepared(request, path, body, pointer, &backend.provider)
            }),
            capabilities: MediaProviderCapabilities {
                kinds: vec![MediaKind::Image, MediaKind::Video],
                references: true,
                negative_prompt: false,
                partial_images: false,
                remote_jobs: true,
                remote_cancellation: true,
            },
            accepts_aspect_ratio: |_, value| crate::media::runway::accepts_image_ratio(value),
            accepts_size: |_, _| false,
        },
    )?;
    #[cfg(feature = "mock")]
    registry.register_media(
        "mock",
        MediaProvider {
            parameters: parameters("backend count size aspect_ratio seed duration_seconds"),
            factory: Arc::new(create_mock_media_generator),
            prepare: Arc::new(|backend, request| {
                let body = crate::media::mock::build_mock_request_envelope(backend, &request);
                prepared(request, "", body, "/prompt_text", &backend.provider)
            }),
            capabilities: MediaProviderCapabilities {
                kinds: vec![MediaKind::Image, MediaKind::Video],
                references: true,
                negative_prompt: true,
                partial_images: true,
                remote_jobs: false,
                remote_cancellation: false,
            },
            accepts_aspect_ratio: |_, _| true,
            accepts_size: |_, _| true,
        },
    )?;
    Ok(())
}

#[cfg(any(
    feature = "openai",
    feature = "google",
    feature = "runway",
    feature = "mock"
))]
fn prepared(
    render_input: MediaRenderRequest,
    path: &str,
    body: serde_json::Value,
    prompt_pointer: &str,
    provider: &str,
) -> Result<PreparedMediaRequest, error::EngineAdapterRequestError> {
    let prompt_text = body
        .pointer(prompt_pointer)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            error::EngineAdapterRequestError::new(
                provider,
                "Prepared request has no prompt text.",
                "invalid_request",
            )
        })?
        .to_owned();
    Ok(PreparedMediaRequest {
        render_input,
        path: path.into(),
        body,
        prompt_text,
    })
}
