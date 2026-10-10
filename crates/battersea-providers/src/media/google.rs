//! Gemini native image-generation adapter.
//!
//! Gemini image models use the interactions API, rather than Imagen's retired
//! `models/*:predict` surface. Configuration owns the host, model, timeout,
//! and output defaults.

use super::{
    read_string_option_opt, with_estimated_generation_progress, MediaBackendConfig,
    MediaGenerationActivityReporter, MediaGenerationActivityUpdate, MediaGenerationAdapter,
    MediaGenerationBatchReporter, MediaRenderRequest, MediaRenderResult, MediaTimingRecorder,
};
use crate::adapter::error::EngineAdapterRequestError;
use crate::http_payload::BoundedResponse as _;
use async_trait::async_trait;
use battersea_model::media::ControllerActivityState;
use battersea_model::media::{MediaAsset, MediaKind, MediaRenderType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const PROVIDER: &str = "google";

pub(crate) fn create_google_media_generator(
    backend: MediaBackendConfig,
) -> Result<Arc<dyn MediaGenerationAdapter>, EngineAdapterRequestError> {
    let timeout_ms = backend
        .options
        .timeout_ms
        .ok_or_else(|| missing_google_media("options.timeoutMs"))?;
    if backend.endpoint.trim().is_empty() {
        return Err(missing_google_media("endpoint"));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
        .map_err(|error| EngineAdapterRequestError::transport(PROVIDER, error.to_string()))?;
    Ok(Arc::new(GoogleMediaGenerator {
        client,
        base_url: resolve_google_base_url(&backend.endpoint),
        api_key: resolve_google_api_key(&backend)?,
        backend,
    }))
}

struct GoogleMediaGenerator {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    backend: MediaBackendConfig,
}

#[async_trait]
impl MediaGenerationAdapter for GoogleMediaGenerator {
    fn backend(&self) -> &MediaBackendConfig {
        &self.backend
    }

    fn prepare(
        &self,
        request: MediaRenderRequest,
    ) -> Result<super::PreparedMediaRequest, EngineAdapterRequestError> {
        super::prepare_media_request(self.backend(), request)
    }

    async fn submit(
        &self,
        prepared: super::PreparedMediaRequest,
        cancellation: CancellationToken,
        activity_reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
        batch_reporter: Option<Arc<dyn super::MediaGenerationBatchReporter>>,
    ) -> Result<super::MediaSubmission, EngineAdapterRequestError> {
        self.render(prepared, cancellation, activity_reporter, batch_reporter)
            .await
            .map(super::MediaSubmission::Complete)
    }
}
impl GoogleMediaGenerator {
    async fn render(
        &self,
        prepared: super::PreparedMediaRequest,
        cancellation: CancellationToken,
        activity_reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
        _batch_reporter: Option<Arc<dyn MediaGenerationBatchReporter>>,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        let request = prepared.render_input.clone();
        if request.kind != MediaKind::Image {
            return Err(EngineAdapterRequestError::new(
                PROVIDER,
                "Gemini media backend only supports image generation.",
                "invalid_request",
            ));
        }
        if !request.references.is_empty() {
            return Err(EngineAdapterRequestError::new(
                PROVIDER,
                "Gemini image references require a persisted interaction and are not supported by this one-shot backend.",
                "invalid_request",
            ));
        }
        if let Some(reporter) = &activity_reporter {
            reporter
                .report_activity(MediaGenerationActivityUpdate {
                    state: ControllerActivityState::Working,
                    event: None,
                    message: String::new(),
                    provider_job_id: None,
                    error_code: None,
                    slot_id: None,
                    slot_index: None,
                    progress: None,
                    eta_ms: None,
                    preview_asset: None,
                    partial_index: None,
                })
                .await?;
        }
        let body = prepared.body;
        let default_mime_type =
            read_string_option_opt(&self.backend.options.extra, "defaultMimeType")
                .ok_or_else(|| missing_google_media("options.defaultMimeType"))?;
        let mut timing = MediaTimingRecorder::start();
        let phase_started_at = chrono::Utc::now();
        let phase_started_instant = std::time::Instant::now();
        let payload: GeminiInteractionResponse = with_estimated_generation_progress(
            &self.backend,
            1,
            activity_reporter,
            cancellation.clone(),
            async {
                let response = self
                    .client
                    .post(self.build_interaction_url())
                    .header("x-goog-api-key", &self.api_key)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|error| {
                        EngineAdapterRequestError::transport(PROVIDER, error.to_string())
                    })?;
                if !response.status().is_success() {
                    return Err(normalize_google_http_error(
                        response.status().as_u16(),
                        response.bounded_text().await.ok(),
                    ));
                }
                response.bounded_json().await.map_err(|error| {
                    EngineAdapterRequestError::invalid_response(PROVIDER, error.to_string())
                })
            },
        )
        .await?;
        timing.record_phase(
            "Gemini image interaction",
            "provider-call",
            true,
            phase_started_at,
            phase_started_instant,
        );
        let asset = interaction_to_asset(&payload, &default_mime_type)?;
        Ok(MediaRenderResult {
            provider_job_id: payload.id.clone(),
            assets: vec![asset],
            provider_request: Some(body),
            provider_response: serde_json::to_value(payload).ok(),
            timing: Some(timing.finish_client_estimate()),
        })
    }
}

impl GoogleMediaGenerator {
    fn build_interaction_url(&self) -> String {
        format!("{}/v1beta/interactions", self.base_url)
    }
}

fn missing_google_media(field: &str) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new(
        PROVIDER,
        format!("Gemini media backend must configure {field}."),
        "invalid_request",
    )
}

pub(crate) fn build_interaction_body(
    backend: &MediaBackendConfig,
    request: &MediaRenderRequest,
) -> Result<Value, EngineAdapterRequestError> {
    let prompt = request.prompt_text.trim();
    if prompt.is_empty() {
        return Err(EngineAdapterRequestError::new(
            PROVIDER,
            "Gemini image prompt text must not be empty.",
            "invalid_request",
        ));
    }
    let mime_type = read_string_option_opt(&backend.options.extra, "defaultMimeType")
        .ok_or_else(|| missing_google_media("options.defaultMimeType"))?;
    let image_size = read_string_option_opt(&backend.options.extra, "imageSize")
        .and_then(|value| normalize_image_size(&value).map(ToOwned::to_owned));
    let aspect_ratio = request
        .options
        .aspect_ratio
        .as_deref()
        .and_then(|value| normalize_aspect_ratio(Some(value)));
    let mut response_format = serde_json::Map::from_iter([
        ("type".to_string(), json!("image")),
        ("mime_type".to_string(), json!(mime_type)),
    ]);
    if let Some(aspect_ratio) = aspect_ratio {
        response_format.insert("aspect_ratio".to_string(), json!(aspect_ratio));
    }
    if let Some(image_size) = image_size {
        response_format.insert("image_size".to_string(), json!(image_size));
    }
    Ok(json!({
        "model": backend.model,
        "input": prompt,
        "response_format": response_format,
    }))
}

pub(crate) fn normalize_aspect_ratio(value: Option<&str>) -> Option<&'static str> {
    match value.unwrap_or("").trim() {
        "1:1" | "square" => Some("1:1"),
        "3:4" => Some("3:4"),
        "4:3" => Some("4:3"),
        "9:16" | "portrait" => Some("9:16"),
        "16:9" | "landscape" => Some("16:9"),
        "2:3" | "4:5" => Some("3:4"),
        "3:2" | "5:4" => Some("4:3"),
        _ => None,
    }
}

fn normalize_image_size(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_uppercase().as_str() {
        "0.5K" => Some("0.5K"),
        "1K" => Some("1K"),
        "2K" => Some("2K"),
        "4K" => Some("4K"),
        _ => None,
    }
}

fn resolve_google_base_url(endpoint: &str) -> String {
    endpoint.trim().trim_end_matches('/').to_string()
}

fn resolve_google_api_key(
    backend: &MediaBackendConfig,
) -> Result<String, EngineAdapterRequestError> {
    let env_var = backend.auth.api_key_env.as_str();
    if !env_var.is_empty() {
        if let Ok(value) = std::env::var(env_var) {
            if !value.trim().is_empty() {
                return Ok(value.trim().to_string());
            }
        }
    }
    if let Some(value) = backend
        .auth
        .api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(value.trim().to_string());
    }
    Err(EngineAdapterRequestError::transport(
        PROVIDER,
        format!(
            "Gemini media backend \"{}\" has no API key. Set {} or configure auth.apiKey.",
            backend.id,
            if env_var.is_empty() {
                "GOOGLE_API_KEY"
            } else {
                env_var
            }
        ),
    ))
}

fn normalize_google_http_error(status: u16, body: Option<String>) -> EngineAdapterRequestError {
    let message = body
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|value| {
            value
                .get("error")?
                .get("message")?
                .as_str()
                .map(ToOwned::to_owned)
        })
        .or(body)
        .unwrap_or_else(|| format!("Gemini image request failed with HTTP {status}."));
    let mut error = EngineAdapterRequestError::transport(PROVIDER, message);
    error.status_code = Some(status);
    error
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct GeminiInteractionResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    output_image: Option<GeminiOutputImage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct GeminiOutputImage {
    data: String,
    #[serde(default)]
    mime_type: Option<String>,
}

fn interaction_to_asset(
    response: &GeminiInteractionResponse,
    default_mime_type: &str,
) -> Result<MediaAsset, EngineAdapterRequestError> {
    let image = response.output_image.as_ref().ok_or_else(|| {
        EngineAdapterRequestError::invalid_response(
            PROVIDER,
            "Gemini interaction did not contain output_image.",
        )
    })?;
    let mime_type = image.mime_type.as_deref().unwrap_or(default_mime_type);
    Ok(MediaAsset {
        url: format!("data:{mime_type};base64,{}", image.data),
        mime_type: Some(mime_type.to_string()),
        media_type: MediaRenderType::Image,

        width: None,
        height: None,
        duration_seconds: None,

        provider_asset_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{EngineAuthConfig, EngineBackendOptions};
    use battersea_model::media::{MediaBackendCapabilities, MediaCapability, MediaGenerationHints};

    fn backend() -> MediaBackendConfig {
        MediaBackendConfig {
            id: "google-image".to_string(),
            provider: PROVIDER.to_string(),
            capability: MediaCapability::ImageGeneration,
            label: "Gemini".to_string(),
            enabled: true,
            endpoint: "https://generativelanguage.googleapis.com".to_string(),
            model: "gemini-3.1-flash-image".to_string(),
            capabilities: MediaBackendCapabilities {
                supported_media_parameters: vec!["backend".into(), "aspect_ratio".into()],
                supports_partial_image_streaming: false,
                ..MediaBackendCapabilities::mock()
            },
            options: EngineBackendOptions {
                timeout_ms: Some(600_000),
                extra: std::collections::BTreeMap::from([
                    ("defaultMimeType".to_string(), json!("image/png")),
                    ("imageSize".to_string(), json!("2K")),
                ]),
                ..EngineBackendOptions::default()
            },
            auth: EngineAuthConfig {
                auth_type: "api-key".to_string(),
                api_key_env: "NO_TEST_KEY".to_string(),
                header: None,
                version_header: None,
                version: None,
                has_api_key: true,
                api_key: Some("test-key".to_string()),
            },
            short_description: String::new(),
            long_description: String::new(),
        }
    }

    fn request() -> MediaRenderRequest {
        MediaRenderRequest {
            kind: MediaKind::Image,
            prompt_text: "A neon city".to_string(),
            negative_prompt: None,
            references: Vec::new(),
            options: MediaGenerationHints {
                aspect_ratio: Some("16:9".to_string()),
                ..MediaGenerationHints::default()
            },
        }
    }

    #[test]
    fn native_interaction_body_uses_configured_model_and_image_format() {
        let body = build_interaction_body(&backend(), &request()).expect("body");
        assert_eq!(body["model"], "gemini-3.1-flash-image");
        assert_eq!(body["input"], "A neon city");
        assert_eq!(body["response_format"]["type"], "image");
        assert_eq!(body["response_format"]["aspect_ratio"], "16:9");
        assert_eq!(body["response_format"]["image_size"], "2K");
    }

    #[test]
    fn interaction_image_becomes_an_engine_asset() {
        let response = GeminiInteractionResponse {
            id: Some("interaction_1".to_string()),
            output_image: Some(GeminiOutputImage {
                data: "aGVsbG8=".to_string(),
                mime_type: Some("image/jpeg".to_string()),
            }),
        };
        let asset = interaction_to_asset(&response, "image/png").expect("asset");
        assert_eq!(asset.url, "data:image/jpeg;base64,aGVsbG8=");
        assert_eq!(asset.mime_type.as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn native_adapter_requires_an_available_key() {
        let mut configured = backend();
        configured.auth.api_key = None;
        let error = match create_google_media_generator(configured) {
            Ok(_) => panic!("missing key should fail"),
            Err(error) => error,
        };
        assert_eq!(error.provider, PROVIDER);
    }
}
