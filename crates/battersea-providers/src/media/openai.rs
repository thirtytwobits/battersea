use super::{
    read_u64_option_opt, resolve_openai_base_url, with_estimated_generation_progress,
    MediaBackendConfig, MediaGenerationActivityReporter, MediaGenerationActivityUpdate,
    MediaGenerationAdapter, MediaRenderRequest, MediaRenderResult, MediaTimingRecorder,
};
use crate::adapter::error::EngineAdapterRequestError;
use crate::http_payload::BoundedResponse as _;
use async_openai::config::{Config, OpenAIConfig};
#[cfg(test)]
use async_openai::error::OpenAIError;
use async_openai::types::images::{
    CreateImageRequest, ImageBackground, ImageGenCompletedEvent, ImageGenPartialImageEvent,
    ImageGenStreamEvent, ImageModel, ImageModeration, ImageOutputFormat, ImageQuality, ImageSize,
};
use async_trait::async_trait;
use battersea_model::media::ControllerActivityState;
use battersea_model::media::{MediaAsset, MediaKind, MediaRenderType};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Builds an OpenAI media generator from backend config using the resolved provider base URL and
/// configured request timeout.
pub(crate) fn create_openai_media_generator(
    backend: MediaBackendConfig,
) -> Result<Arc<dyn MediaGenerationAdapter>, EngineAdapterRequestError> {
    let timeout_ms = backend
        .options
        .timeout_ms
        .ok_or_else(|| missing_openai_media_option("timeoutMs"))?;
    let base_url = resolve_openai_base_url(&backend.endpoint);
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
        .map_err(|error| EngineAdapterRequestError::transport("openai", error.to_string()))?;
    let config = OpenAIConfig::new()
        .with_api_key(backend.auth.api_key.clone().unwrap_or_default())
        .with_api_base(base_url);
    Ok(Arc::new(OpenAiMediaGenerator {
        http_client,
        config,
        backend,
    }))
}

/// Error for an OpenAI media backend option that configuration must supply.
/// No hardcoded defaults exist in the adapter: absent config → no value.
fn missing_openai_media_option(key: &str) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new(
        "openai",
        format!("OpenAI media backend must configure options.{key}."),
        "invalid_request",
    )
}

struct OpenAiMediaGenerator {
    http_client: reqwest::Client,
    config: OpenAIConfig,
    backend: MediaBackendConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
struct OpenAiImagesResponse {
    created: u64,
    #[serde(default)]
    data: Vec<OpenAiImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_format: Option<String>,
    /// The provider may return exact generated dimensions that are not valid request sizes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
struct OpenAiImage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    b64_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revised_prompt: Option<String>,
}

#[async_trait]
impl MediaGenerationAdapter for OpenAiMediaGenerator {
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
impl OpenAiMediaGenerator {
    async fn render(
        &self,
        prepared: super::PreparedMediaRequest,
        cancellation: CancellationToken,
        activity_reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
        _batch_reporter: Option<Arc<dyn super::MediaGenerationBatchReporter>>,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        let request = prepared.render_input.clone();
        if request.kind != MediaKind::Image {
            return Err(EngineAdapterRequestError::new(
                "openai",
                "OpenAI media backend only supports image generation.",
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

        let provider_request = prepared.body;
        let requested_asset_count = provider_request["n"].as_u64().expect("prepared count") as u8;
        let target_asset_count = usize::from(requested_asset_count);
        let stream_partial_images = provider_request["stream"].as_bool().unwrap_or(false);
        let partial_images = provider_request["partial_images"].as_u64().unwrap_or(0) as u8;
        let mut timing = MediaTimingRecorder::start();
        let phase_started_at = chrono::Utc::now();
        let phase_started_instant = std::time::Instant::now();
        let response = if stream_partial_images {
            let request_future = send_image_generation_stream_request(
                &self.http_client,
                &self.config,
                provider_request.clone(),
                target_asset_count,
                partial_images,
                activity_reporter.clone(),
                cancellation.clone(),
            );
            tokio::select! {
                _ = cancellation.cancelled() => {
                    return Err(EngineAdapterRequestError::new("openai", "Media generation cancelled.", "cancelled"));
                }
                result = request_future => result?
            }
        } else {
            with_estimated_generation_progress(
                &self.backend,
                target_asset_count,
                activity_reporter.clone(),
                cancellation.clone(),
                send_image_generation_json_request(
                    &self.http_client,
                    &self.config,
                    provider_request.clone(),
                ),
            )
            .await?
        };
        timing.record_phase(
            "image generation request",
            "provider-call",
            true,
            phase_started_at,
            phase_started_instant,
        );
        let provider_response = serde_json::to_value(&response).ok();

        let output_format = response
            .output_format
            .as_deref()
            .and_then(|value| parse_output_format(Some(value)))
            .or_else(|| {
                self.backend
                    .options
                    .extra
                    .get("outputFormat")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| parse_output_format(Some(value)))
            })
            .ok_or_else(|| missing_openai_media_option("outputFormat"))?;

        let mime_type = mime_type_for_output_format(&output_format);

        let (width, height) = parse_dimensions(response.size.as_deref());
        let assets = response
            .data
            .into_iter()
            .map(|image| {
                let url = image
                    .url
                    .or_else(|| {
                        image
                            .b64_json
                            .map(|b64_json| format!("data:{};base64,{}", mime_type, b64_json))
                    })
                    .ok_or_else(|| {
                        EngineAdapterRequestError::new(
                            "openai",
                            "OpenAI image response did not contain a URL or inline image data.",
                            "request",
                        )
                    })?;
                Ok(MediaAsset {
                    url,
                    mime_type: Some(mime_type.to_string()),
                    media_type: MediaRenderType::Image,

                    width,
                    height,
                    duration_seconds: None,

                    provider_asset_id: None,
                })
            })
            .collect::<Result<Vec<_>, EngineAdapterRequestError>>()?;

        Ok(MediaRenderResult {
            provider_job_id: None,
            assets,
            provider_request: Some(provider_request),
            provider_response,
            timing: Some(timing.finish_client_estimate()),
        })
    }
}

async fn send_image_generation_json_request(
    http_client: &reqwest::Client,
    config: &OpenAIConfig,
    request_body: Value,
) -> Result<OpenAiImagesResponse, EngineAdapterRequestError> {
    let response = http_client
        .post(config.url("/images/generations"))
        .headers(config.headers())
        .query(&config.query())
        .json(&request_body)
        .send()
        .await
        .map_err(|error| EngineAdapterRequestError::transport("openai", error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .bounded_text()
            .await
            .map_err(|error| EngineAdapterRequestError::transport("openai", error.to_string()))?;
        return Err(parse_openai_error_body(&body).unwrap_or_else(|| {
            EngineAdapterRequestError::new(
                "openai",
                format!(
                    "OpenAI image generation failed with HTTP status {}.",
                    status
                ),
                "request",
            )
        }));
    }

    response
        .bounded_json::<OpenAiImagesResponse>()
        .await
        .map_err(|error| EngineAdapterRequestError::invalid_response("openai", error.to_string()))
}

async fn send_image_generation_stream_request(
    http_client: &reqwest::Client,
    config: &OpenAIConfig,
    request_body: Value,
    target_asset_count: usize,
    partial_images: u8,
    activity_reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
    cancellation: CancellationToken,
) -> Result<OpenAiImagesResponse, EngineAdapterRequestError> {
    let response = http_client
        .post(config.url("/images/generations"))
        .headers(config.headers())
        .query(&config.query())
        .json(&request_body)
        .send()
        .await
        .map_err(|error| EngineAdapterRequestError::transport("openai", error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .bounded_text()
            .await
            .map_err(|error| EngineAdapterRequestError::transport("openai", error.to_string()))?;
        return Err(parse_openai_error_body(&body).unwrap_or_else(|| {
            EngineAdapterRequestError::new(
                "openai",
                format!(
                    "OpenAI image generation failed with HTTP status {}.",
                    status
                ),
                "request",
            )
        }));
    }

    let mut stream = crate::http_payload::bounded_stream(response).eventsource();
    let mut completed_events = Vec::new();
    while let Some(event_result) = tokio::select! {
        _ = cancellation.cancelled() => {
            return Err(EngineAdapterRequestError::new("openai", "Media generation cancelled.", "cancelled"));
        }
        event_result = stream.next() => event_result
    } {
        let event = event_result
            .map_err(|error| EngineAdapterRequestError::transport("openai", error.to_string()))?;
        if event.data.trim() == "[DONE]" {
            break;
        }
        let parsed: ImageGenStreamEvent = serde_json::from_str(&event.data).map_err(|error| {
            EngineAdapterRequestError::invalid_response("openai", error.to_string())
        })?;
        match parsed {
            ImageGenStreamEvent::PartialImage(partial) => {
                emit_openai_partial_image_update(
                    &activity_reporter,
                    target_asset_count,
                    &partial,
                    partial_images,
                )
                .await?;
            }
            ImageGenStreamEvent::Completed(completed) => {
                completed_events.push(completed);
            }
        }
    }

    if completed_events.is_empty() {
        return Err(EngineAdapterRequestError::invalid_response(
            "openai",
            "OpenAI image generation stream ended without a completed image event.",
        ));
    }

    Ok(openai_stream_events_to_response(completed_events))
}

async fn emit_openai_partial_image_update(
    activity_reporter: &Option<Arc<dyn MediaGenerationActivityReporter>>,
    slot_count: usize,
    partial: &ImageGenPartialImageEvent,
    requested_partial_images: u8,
) -> Result<(), crate::EngineAdapterRequestError> {
    const PROGRESS_CAP: f32 = 0.9;
    let Some(reporter) = activity_reporter else {
        return Ok(());
    };
    let addressable_slots = slot_count.min(u8::MAX as usize);
    if addressable_slots == 0 {
        return Ok(());
    }
    let denominator = u16::from(requested_partial_images.clamp(1, 3)) + 1;
    let ordinal = (u16::from(partial.partial_image_index) + 1).min(denominator - 1);
    let progress =
        ((ordinal as f32) / (denominator as f32) * PROGRESS_CAP).clamp(0.0, PROGRESS_CAP);
    let preview_asset = openai_partial_image_to_asset(partial);
    for index in 0..addressable_slots {
        reporter
            .report_activity(MediaGenerationActivityUpdate {
                state: ControllerActivityState::Working,
                event: None,
                message: String::new(),
                provider_job_id: None,
                error_code: None,
                slot_id: None,
                slot_index: Some(index as u8),
                progress: Some(progress),
                eta_ms: None,
                preview_asset: Some(preview_asset.clone()),
                partial_index: Some(partial.partial_image_index),
            })
            .await?;
    }
    Ok(())
}

fn openai_partial_image_to_asset(partial: &ImageGenPartialImageEvent) -> MediaAsset {
    let mime_type = mime_type_for_output_format(&partial.output_format);
    let size = enum_json_string(&partial.size);
    let (width, height) = parse_dimensions(size.as_deref());
    MediaAsset {
        url: format!("data:{};base64,{}", mime_type, partial.b64_json),
        mime_type: Some(mime_type.to_string()),
        media_type: MediaRenderType::Image,

        width,
        height,
        duration_seconds: None,

        provider_asset_id: None,
    }
}

fn mime_type_for_output_format(output_format: &ImageOutputFormat) -> &'static str {
    match output_format {
        ImageOutputFormat::Png => "image/png",
        ImageOutputFormat::Jpeg => "image/jpeg",
        ImageOutputFormat::Webp => "image/webp",
    }
}

fn openai_stream_events_to_response(
    completed_events: Vec<ImageGenCompletedEvent>,
) -> OpenAiImagesResponse {
    let created = completed_events
        .first()
        .map(|event| event.created_at)
        .unwrap_or_default();
    let background = completed_events
        .last()
        .and_then(|event| enum_json_string(&event.background));
    let output_format = completed_events
        .last()
        .and_then(|event| enum_json_string(&event.output_format));
    let size = completed_events
        .last()
        .and_then(|event| enum_json_string(&event.size));
    let quality = completed_events
        .last()
        .and_then(|event| enum_json_string(&event.quality));
    let usage = completed_events
        .last()
        .and_then(|event| serde_json::to_value(&event.usage).ok());
    OpenAiImagesResponse {
        created,
        data: completed_events
            .into_iter()
            .map(|event| OpenAiImage {
                url: None,
                b64_json: Some(event.b64_json),
                revised_prompt: None,
            })
            .collect(),
        background,
        output_format,
        size,
        quality,
        usage,
    }
}

fn enum_json_string<T: Serialize>(value: &T) -> Option<String> {
    match serde_json::to_value(value).ok()? {
        Value::String(value) => Some(value),
        _ => None,
    }
}

/// Reads the configured partial-image count. Required from config — the
/// adapter carries no hardcoded default.
fn require_openai_partial_images(
    backend: &MediaBackendConfig,
) -> Result<u8, EngineAdapterRequestError> {
    let value = read_u64_option_opt(&backend.options.extra, "partialImages")
        .ok_or_else(|| missing_openai_media_option("partialImages"))?;
    u8::try_from(value)
        .map(|value| value.clamp(0, 3))
        .map_err(|_| missing_openai_media_option("partialImages"))
}

fn should_stream_openai_partial_images(
    backend_supports_partial_streaming: bool,
    requested_asset_count: u8,
    partial_images: u8,
) -> bool {
    backend_supports_partial_streaming && requested_asset_count == 1 && partial_images > 0
}

#[cfg(test)]
fn parse_openai_images_response_body(
    body: &str,
) -> Result<OpenAiImagesResponse, EngineAdapterRequestError> {
    serde_json::from_str(body).map_err(|error| {
        EngineAdapterRequestError::transport(
            "openai",
            format!(
                "failed to deserialize api response: error:{} content:{}",
                error,
                truncate_for_error(body)
            ),
        )
    })
}

fn parse_openai_error_body(body: &str) -> Option<EngineAdapterRequestError> {
    #[derive(Deserialize)]
    struct ErrorEnvelope {
        error: OpenAiErrorBody,
    }

    #[derive(Deserialize)]
    struct OpenAiErrorBody {
        message: String,
    }

    serde_json::from_str::<ErrorEnvelope>(body)
        .ok()
        .map(|envelope| EngineAdapterRequestError::new("openai", envelope.error.message, "request"))
}

#[cfg(test)]
fn truncate_for_error(body: &str) -> String {
    const LIMIT: usize = 4096;
    if body.len() <= LIMIT {
        return body.to_string();
    }
    format!(
        "{}… <truncated {} bytes>",
        &body[..LIMIT],
        body.len() - LIMIT
    )
}

/// Normalises OpenAI media-generation failures into the shared provider-neutral error shape.
#[cfg(test)]
fn normalize_openai_error(error: OpenAIError) -> EngineAdapterRequestError {
    match error {
        OpenAIError::ApiError(inner) => {
            EngineAdapterRequestError::new("openai", inner.message.clone(), "request")
        }
        OpenAIError::Reqwest(error) => {
            EngineAdapterRequestError::transport("openai", error.to_string())
        }
        other => EngineAdapterRequestError::transport("openai", other.to_string()),
    }
}

/// Maps supported aspect-ratio aliases onto the image sizes accepted by
/// the OpenAI image API. Must cover every value the UI's
/// `ASPECT_RATIO_OPTIONS` list emits (`1:1`, `3:4`, `9:16`, `4:3`,
/// `16:9`); missing an option silently degrades to the provider
/// default, which manifested as "I picked portrait but got landscape".
pub(crate) fn parse_size(value: Option<&str>) -> Option<ImageSize> {
    match value.unwrap_or("").trim() {
        "1:1" | "square" => Some(ImageSize::S1024x1024),
        "3:2" | "4:3" | "16:9" | "landscape" => Some(ImageSize::S1536x1024),
        "2:3" | "3:4" | "4:5" | "9:16" | "portrait" => Some(ImageSize::S1024x1536),
        _ => None,
    }
}

/// Maps an explicit `WIDTHxHEIGHT` string from the size hint onto an
/// OpenAI `ImageSize`. The hint can also be `auto` / empty, in which
/// case the caller falls back to deriving the size from `aspect_ratio`.
pub(crate) fn parse_explicit_size(value: Option<&str>) -> Option<ImageSize> {
    match value.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "1024x1024" => Some(ImageSize::S1024x1024),
        "1024x1536" => Some(ImageSize::S1024x1536),
        "1536x1024" => Some(ImageSize::S1536x1024),
        _ => None,
    }
}

fn parse_dimensions(value: Option<&str>) -> (Option<u32>, Option<u32>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return (None, None);
    };
    let Some((width, height)) = value.split_once('x') else {
        return (None, None);
    };
    let (Ok(width), Ok(height)) = (width.parse::<u32>(), height.parse::<u32>()) else {
        return (None, None);
    };
    (Some(width), Some(height))
}

/// Maps configured quality strings onto OpenAI image quality values.
fn parse_quality(value: Option<&str>) -> Option<ImageQuality> {
    match value.unwrap_or("").trim() {
        "low" => Some(ImageQuality::Low),
        "medium" => Some(ImageQuality::Medium),
        "high" => Some(ImageQuality::High),
        "auto" => Some(ImageQuality::Auto),
        _ => None,
    }
}

/// Validates the current GPT Image 2.5 quality vocabulary. `async-openai`
/// 0.33 predates `xhigh` and `max`; request serialization below deliberately
/// preserves those values instead of silently dropping them.
fn configured_openai_quality(value: Option<&str>) -> Result<String, EngineAdapterRequestError> {
    let quality = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| missing_openai_media_option("quality"))?;
    if matches!(
        quality,
        "low" | "medium" | "high" | "xhigh" | "max" | "auto"
    ) {
        Ok(quality.to_string())
    } else {
        Err(EngineAdapterRequestError::new(
            "openai",
            format!("OpenAI media backend has unsupported options.quality \"{quality}\"."),
            "invalid_request",
        ))
    }
}

/// Maps configured output-format strings onto OpenAI image output formats.
fn parse_output_format(value: Option<&str>) -> Option<ImageOutputFormat> {
    match value.unwrap_or("").trim() {
        "png" => Some(ImageOutputFormat::Png),
        "jpeg" | "jpg" => Some(ImageOutputFormat::Jpeg),
        "webp" => Some(ImageOutputFormat::Webp),
        _ => None,
    }
}

/// Maps configured moderation strings onto OpenAI image moderation values.
fn parse_moderation(value: Option<&str>) -> Option<ImageModeration> {
    match value.unwrap_or("").trim() {
        "low" => Some(ImageModeration::Low),
        "auto" => Some(ImageModeration::Auto),
        _ => None,
    }
}

/// Maps configured background strings onto OpenAI image background values.
fn parse_background(value: Option<&str>) -> Option<ImageBackground> {
    match value.unwrap_or("").trim() {
        "transparent" => Some(ImageBackground::Transparent),
        "opaque" => Some(ImageBackground::Opaque),
        "auto" => Some(ImageBackground::Auto),
        _ => None,
    }
}

/// Builds the generation-defining submit body without constructing a client.
pub(crate) fn prepare_body(
    backend: &MediaBackendConfig,
    request: MediaRenderRequest,
) -> Result<Value, EngineAdapterRequestError> {
    let requested_asset_count = match request.options.count {
        Some(count) => count.max(1),
        None => u8::try_from(
            read_u64_option_opt(&backend.options.extra, "defaultCount")
                .ok_or_else(|| missing_openai_media_option("defaultCount"))?,
        )
        .map_err(|_| missing_openai_media_option("defaultCount"))?
        .max(1),
    };
    let partial_images = require_openai_partial_images(backend)?;
    let stream_partial_images = should_stream_openai_partial_images(
        backend.capabilities.supports_partial_image_streaming,
        requested_asset_count,
        partial_images,
    );
    let configured_quality = configured_openai_quality(
        backend
            .options
            .extra
            .get("quality")
            .and_then(serde_json::Value::as_str),
    )?;
    let create_request = CreateImageRequest {
        prompt: request.prompt_text,
        model: Some(ImageModel::Other(backend.model.clone())),
        n: Some(requested_asset_count),
        quality: parse_quality(Some(&configured_quality)),
        response_format: None,
        output_format: parse_output_format(
            backend
                .options
                .extra
                .get("outputFormat")
                .and_then(serde_json::Value::as_str),
        ),
        output_compression: backend
            .options
            .extra
            .get("outputCompression")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u8::try_from(value).ok()),
        stream: stream_partial_images.then_some(true),
        partial_images: stream_partial_images.then_some(partial_images),
        // Explicit size hint wins (so the size dropdown in the UI
        // can override the aspect-ratio picker); fall through to the
        // aspect-ratio mapping otherwise.
        size: parse_explicit_size(request.options.size.as_deref())
            .or_else(|| parse_size(request.options.aspect_ratio.as_deref())),
        moderation: parse_moderation(
            backend
                .options
                .extra
                .get("moderation")
                .and_then(serde_json::Value::as_str),
        ),
        background: parse_background(
            backend
                .options
                .extra
                .get("background")
                .and_then(serde_json::Value::as_str),
        ),
        style: None,
        user: backend
            .options
            .extra
            .get("user")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    };

    // Capture the request body before it's moved into the SDK
    // call so the provenance row can reproduce the exact image
    // request. Credentials ride in headers, not this body.
    let mut provider_request = serde_json::to_value(&create_request).map_err(|error| {
        EngineAdapterRequestError::invalid_response("openai", error.to_string())
    })?;
    provider_request["quality"] = Value::String(configured_quality);
    Ok(provider_request)
}

#[cfg(test)]
mod tests {
    use super::{
        configured_openai_quality, create_openai_media_generator, normalize_openai_error,
        parse_background, parse_dimensions, parse_explicit_size, parse_moderation,
        parse_openai_images_response_body, parse_output_format, parse_quality, parse_size,
    };
    use crate::adapter::{EngineAuthConfig, EngineBackendOptions};
    use crate::media::{
        MediaBackendConfig, MediaGenerationActivityReporter, MediaGenerationActivityUpdate,
        MediaRenderRequest,
    };
    use async_openai::error::{ApiError, OpenAIError};
    use async_openai::types::images::{
        ImageBackground, ImageModeration, ImageOutputFormat, ImageQuality, ImageSize,
    };
    use battersea_model::media::{
        MediaBackendCapabilities, MediaCapability, MediaGenerationHints, MediaKind,
    };
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn sample_backend() -> MediaBackendConfig {
        MediaBackendConfig {
            id: "openai-image".to_string(),
            provider: "openai".to_string(),
            capability: MediaCapability::ImageGeneration,
            label: "OpenAI image".to_string(),
            enabled: true,
            endpoint: "https://api.openai.com/v1/images/generations".to_string(),
            model: "synthetic-image-model".to_string(),
            capabilities: MediaBackendCapabilities {
                supports_partial_image_streaming: true,
                supported_media_parameters: vec![
                    "backend".into(),
                    "count".into(),
                    "size".into(),
                    "aspect_ratio".into(),
                ],
                ..MediaBackendCapabilities::mock()
            },
            options: EngineBackendOptions {
                extra: std::collections::BTreeMap::from([
                    ("quality".to_string(), serde_json::json!("high")),
                    ("outputFormat".to_string(), serde_json::json!("png")),
                    ("defaultCount".to_string(), serde_json::json!(1)),
                    ("partialImages".to_string(), serde_json::json!(3)),
                ]),
                timeout_ms: Some(600_000),
                ..EngineBackendOptions::default()
            },
            auth: EngineAuthConfig {
                auth_type: "bearer".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
                header: None,
                version_header: None,
                version: None,
                has_api_key: true,
                api_key: Some("secret".to_string()),
            },
            short_description: String::new(),
            long_description: String::new(),
        }
    }

    fn sample_request(prompt: &str) -> MediaRenderRequest {
        MediaRenderRequest {
            kind: MediaKind::Image,
            prompt_text: prompt.to_string(),
            negative_prompt: None,
            references: Vec::new(),
            options: MediaGenerationHints {
                count: Some(1),
                ..MediaGenerationHints::default()
            },
        }
    }

    struct CapturingReporter {
        updates: Mutex<Vec<MediaGenerationActivityUpdate>>,
    }

    impl CapturingReporter {
        fn new() -> Self {
            Self {
                updates: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl MediaGenerationActivityReporter for CapturingReporter {
        async fn report_activity(
            &self,
            update: MediaGenerationActivityUpdate,
        ) -> Result<(), crate::EngineAdapterRequestError> {
            self.updates.lock().expect("updates lock").push(update);
            Ok(())
        }
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let size = stream.read(&mut buffer).expect("read request");
            assert!(size > 0, "client closed request before body was complete");
            bytes.extend_from_slice(&buffer[..size]);
            let request_text = String::from_utf8_lossy(&bytes);
            let Some(header_end) = request_text.find("\r\n\r\n") else {
                continue;
            };
            let content_length = request_text
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length: ")
                        .or_else(|| line.strip_prefix("Content-Length: "))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if bytes.len() >= header_end + 4 + content_length {
                return request_text.into_owned();
            }
        }
    }

    fn spawn_openai_image_stream_server() -> (String, thread::JoinHandle<()>) {
        let listener =
            TcpListener::bind(crate::test_endpoints::TEST_BIND_ADDRESS).expect("bind loopback");
        let address = listener.local_addr().expect("local addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let request_text = read_http_request(&mut stream);
            assert!(request_text.starts_with("POST /v1/images/generations HTTP/1.1"));
            assert!(request_text.contains("\"stream\":true"));
            assert!(request_text.contains("\"partial_images\":3"));

            let body = concat!(
                "event: image_generation.partial_image\n",
                "data: {\"type\":\"image_generation.partial_image\",\"b64_json\":\"cGFydGlhbA==\",\"created_at\":1780292000,\"size\":\"1024x1024\",\"quality\":\"high\",\"background\":\"opaque\",\"output_format\":\"png\",\"partial_image_index\":0}\n\n",
                "event: image_generation.completed\n",
                "data: {\"type\":\"image_generation.completed\",\"b64_json\":\"ZmluYWw=\",\"created_at\":1780292001,\"size\":\"1024x1024\",\"quality\":\"high\",\"background\":\"opaque\",\"output_format\":\"png\",\"usage\":{\"input_tokens\":1,\"total_tokens\":3,\"output_tokens\":2,\"output_token_details\":{\"text_tokens\":0,\"image_tokens\":2},\"input_tokens_details\":{\"text_tokens\":1,\"image_tokens\":0}}}\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            stream.flush().expect("flush response");
        });
        (format!("http://{address}/v1"), handle)
    }

    fn spawn_openai_image_json_server() -> (String, thread::JoinHandle<()>) {
        let listener =
            TcpListener::bind(crate::test_endpoints::TEST_BIND_ADDRESS).expect("bind loopback");
        let address = listener.local_addr().expect("local addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let request_text = read_http_request(&mut stream);
            assert!(request_text.starts_with("POST /v1/images/generations HTTP/1.1"));
            assert!(request_text.contains("\"n\":2"));
            assert!(!request_text.contains("\"stream\""));
            assert!(!request_text.contains("\"partial_images\""));

            thread::sleep(Duration::from_millis(50));
            let body = serde_json::to_string(&json!({
                "created": 1780292100_u64,
                "data": [
                    { "b64_json": "Zmlyc3Q=" },
                    { "b64_json": "c2Vjb25k" }
                ],
                "output_format": "png",
                "size": "1024x1024",
                "quality": "high"
            }))
            .expect("json body");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            stream.flush().expect("flush response");
        });
        (format!("http://{address}/v1"), handle)
    }

    #[test]
    fn create_openai_media_generator_succeeds_for_valid_backend_config() {
        assert!(create_openai_media_generator(sample_backend()).is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn generate_rejects_non_image_media_kinds() {
        let generator = create_openai_media_generator(sample_backend()).expect("generator");
        let request = MediaRenderRequest {
            kind: MediaKind::Video,
            prompt_text: "A moving shot".to_string(),
            negative_prompt: None,
            references: Vec::new(),
            options: MediaGenerationHints::default(),
        };

        let error = generator
            .generate(request, CancellationToken::new(), None, None)
            .await
            .expect_err("video should be rejected");
        assert_eq!(error.provider, "openai");
        assert_eq!(error.classification.as_str(), "invalid_request");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn generate_streams_partial_image_progress_and_completed_asset() {
        let (endpoint, handle) = spawn_openai_image_stream_server();
        let mut backend = sample_backend();
        backend.endpoint = endpoint;
        let generator = create_openai_media_generator(backend).expect("generator");
        let reporter = Arc::new(CapturingReporter::new());

        let result = generator
            .generate(
                sample_request("A streamed image"),
                CancellationToken::new(),
                Some(reporter.clone()),
                None,
            )
            .await
            .expect("streamed image generation succeeds");
        handle.join().expect("server joins");

        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].url, "data:image/png;base64,ZmluYWw=");
        assert_eq!(result.assets[0].width, Some(1024));
        assert_eq!(result.assets[0].height, Some(1024));

        let updates = reporter.updates.lock().expect("updates lock");
        let progress_updates = updates
            .iter()
            .filter(|update| update.progress.is_some())
            .collect::<Vec<_>>();
        assert_eq!(progress_updates.len(), 1);
        assert_eq!(progress_updates[0].slot_index, Some(0));
        let progress = progress_updates[0].progress.expect("progress");
        assert!(progress > 0.0 && progress < 1.0);
        let preview_asset = progress_updates[0]
            .preview_asset
            .as_ref()
            .expect("partial image update carries preview asset");
        assert_eq!(preview_asset.url, "data:image/png;base64,cGFydGlhbA==");
        assert_eq!(preview_asset.mime_type.as_deref(), Some("image/png"));
        assert_eq!(preview_asset.width, Some(1024));
        assert_eq!(preview_asset.height, Some(1024));
        assert_eq!(progress_updates[0].partial_index, Some(0));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn generate_multiple_openai_images_uses_json_response_with_estimated_progress() {
        let (endpoint, handle) = spawn_openai_image_json_server();
        let mut backend = sample_backend();
        backend.endpoint = endpoint;
        backend
            .options
            .extra
            .insert("estimatedProgressIntervalMs".to_string(), json!(10));
        let generator = create_openai_media_generator(backend).expect("generator");
        let reporter = Arc::new(CapturingReporter::new());
        let mut request = sample_request("Two images without streaming");
        request.options.count = Some(2);

        let result = generator
            .generate(
                request,
                CancellationToken::new(),
                Some(reporter.clone()),
                None,
            )
            .await
            .expect("non-streamed image generation succeeds");
        handle.join().expect("server joins");

        assert_eq!(result.assets.len(), 2);
        assert_eq!(result.assets[0].url, "data:image/png;base64,Zmlyc3Q=");
        assert_eq!(result.assets[1].url, "data:image/png;base64,c2Vjb25k");

        let updates = reporter.updates.lock().expect("updates lock");
        let slot_indexes = updates
            .iter()
            .filter(|update| update.progress.is_some())
            .filter_map(|update| update.slot_index)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(slot_indexes, [0, 1].into_iter().collect());
        assert!(updates.iter().all(|update| update.progress != Some(1.0)));
        assert!(updates.iter().all(|update| update.preview_asset.is_none()));
    }

    #[test]
    fn parse_size_maps_supported_aliases_and_unknowns() {
        assert_eq!(parse_size(Some("square")), Some(ImageSize::S1024x1024));
        assert_eq!(parse_size(Some("1:1")), Some(ImageSize::S1024x1024));
        // Every aspect-ratio value the UI's ASPECT_RATIO_OPTIONS list emits
        // must resolve to a concrete portrait/landscape size — otherwise the
        // adapter sends `size: None` and OpenAI picks its own default
        // (often landscape), which the user perceives as "portrait does
        // nothing".
        assert_eq!(parse_size(Some("3:4")), Some(ImageSize::S1024x1536));
        assert_eq!(parse_size(Some("9:16")), Some(ImageSize::S1024x1536));
        assert_eq!(parse_size(Some("4:3")), Some(ImageSize::S1536x1024));
        assert_eq!(parse_size(Some("16:9")), Some(ImageSize::S1536x1024));
        assert_eq!(parse_size(Some("portrait")), Some(ImageSize::S1024x1536));
        assert_eq!(parse_size(Some("landscape")), Some(ImageSize::S1536x1024));
        assert_eq!(parse_size(Some("unknown")), None);
        assert_eq!(parse_size(None), None);
    }

    #[test]
    fn parse_explicit_size_accepts_width_x_height_and_falls_through_for_auto() {
        assert_eq!(
            parse_explicit_size(Some("1024x1024")),
            Some(ImageSize::S1024x1024)
        );
        assert_eq!(
            parse_explicit_size(Some("1024x1536")),
            Some(ImageSize::S1024x1536)
        );
        assert_eq!(
            parse_explicit_size(Some("1536x1024")),
            Some(ImageSize::S1536x1024)
        );
        assert_eq!(parse_explicit_size(Some("auto")), None);
        assert_eq!(parse_explicit_size(Some("")), None);
        assert_eq!(parse_explicit_size(None), None);
        // Unsupported provider sizes also fall through.
        assert_eq!(parse_explicit_size(Some("4096x4096")), None);
    }

    #[test]
    fn parse_dimensions_accepts_provider_returned_exact_sizes() {
        assert_eq!(
            parse_dimensions(Some("1402x1122")),
            (Some(1402), Some(1122))
        );
        assert_eq!(
            parse_dimensions(Some("1024x1024")),
            (Some(1024), Some(1024))
        );
        assert_eq!(parse_dimensions(Some("auto")), (None, None));
        assert_eq!(parse_dimensions(Some("")), (None, None));
        assert_eq!(parse_dimensions(None), (None, None));
    }

    #[test]
    fn openai_image_response_accepts_unknown_provider_size() {
        let response = parse_openai_images_response_body(
            r#"{
              "created": 1780292123,
              "background": "opaque",
              "data": [
                { "b64_json": "abc123", "revised_prompt": null }
              ],
              "output_format": "png",
              "quality": "high",
              "size": "1402x1122",
              "usage": {
                "input_tokens": 368,
                "input_tokens_details": {
                  "image_tokens": 0,
                  "text_tokens": 368
                },
                "output_tokens": 13206,
                "output_tokens_details": {
                  "image_tokens": 13206,
                  "text_tokens": 0
                },
                "total_tokens": 13574
              }
            }"#,
        )
        .expect("provider dimensions are response metadata, not a request-size enum");

        assert_eq!(response.size.as_deref(), Some("1402x1122"));
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].b64_json.as_deref(), Some("abc123"));
    }

    #[test]
    fn parse_quality_output_format_moderation_and_background_cover_known_values() {
        assert_eq!(parse_quality(Some("low")), Some(ImageQuality::Low));
        assert_eq!(parse_quality(Some("medium")), Some(ImageQuality::Medium));
        assert_eq!(parse_quality(Some("high")), Some(ImageQuality::High));
        assert_eq!(parse_quality(Some("auto")), Some(ImageQuality::Auto));
        assert_eq!(parse_quality(Some("other")), None);

        assert_eq!(
            parse_output_format(Some("png")),
            Some(ImageOutputFormat::Png)
        );
        assert_eq!(
            parse_output_format(Some("jpeg")),
            Some(ImageOutputFormat::Jpeg)
        );
        assert_eq!(
            parse_output_format(Some("jpg")),
            Some(ImageOutputFormat::Jpeg)
        );
        assert_eq!(
            parse_output_format(Some("webp")),
            Some(ImageOutputFormat::Webp)
        );
        assert_eq!(parse_output_format(Some("gif")), None);

        assert_eq!(parse_moderation(Some("low")), Some(ImageModeration::Low));
        assert_eq!(parse_moderation(Some("auto")), Some(ImageModeration::Auto));
        assert_eq!(parse_moderation(Some("other")), None);

        assert_eq!(
            parse_background(Some("transparent")),
            Some(ImageBackground::Transparent)
        );
        assert_eq!(
            parse_background(Some("opaque")),
            Some(ImageBackground::Opaque)
        );
        assert_eq!(parse_background(Some("auto")), Some(ImageBackground::Auto));
        assert_eq!(parse_background(Some("other")), None);
    }

    #[test]
    fn configured_quality_preserves_the_full_gpt_image_2_5_vocabulary() {
        for quality in ["low", "medium", "high", "xhigh", "max", "auto"] {
            assert_eq!(
                configured_openai_quality(Some(quality)).expect("valid quality"),
                quality
            );
        }
        assert!(configured_openai_quality(Some("ultra")).is_err());
    }

    #[test]
    fn normalize_openai_error_maps_api_request_and_transport_failures() {
        let api = normalize_openai_error(OpenAIError::ApiError(ApiError {
            message: "invalid prompt".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: None,
            code: None,
        }));
        assert_eq!(api.provider, "openai");
        assert_eq!(api.classification.as_str(), "request");
        assert_eq!(api.message, "invalid prompt");

        let transport = normalize_openai_error(OpenAIError::StreamError(Box::new(
            async_openai::error::StreamError::EventStream("socket closed".to_string()),
        )));
        assert_eq!(transport.classification.as_str(), "transport");

        let other = normalize_openai_error(OpenAIError::InvalidArgument("bad args".to_string()));
        assert_eq!(other.classification.as_str(), "transport");
    }
}
