use super::{
    normalize_base_url, read_bool_option, read_string_option_opt, read_u64_option_opt,
    MediaBackendConfig, MediaGenerationActivityReporter, MediaGenerationActivityUpdate,
    MediaGenerationAdapter, MediaRenderRequest, MediaTimingRecorder,
};
use crate::adapter::error::EngineAdapterRequestError;
use crate::http_payload::BoundedResponse as _;
use async_trait::async_trait;
use battersea_model::media::ControllerActivityState;
use battersea_model::media::{MediaAsset, MediaKind, MediaReference, MediaRenderType};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Builds a Runway media generator from backend config, deriving a reusable base URL and
/// validating auth and version headers during construction.
pub(crate) fn create_runway_media_generator(
    backend: MediaBackendConfig,
) -> Result<Arc<dyn MediaGenerationAdapter>, EngineAdapterRequestError> {
    super::jobs::poll_policy(&backend)?;
    let timeout_ms = require_timeout_ms(&backend)?;
    for (name, value) in [
        ("timeoutMs", timeout_ms),
        (
            "pollIntervalMs",
            require_u64_option(&backend, "pollIntervalMs")?,
        ),
    ] {
        if value == 0 {
            return Err(missing_runway_option(name));
        }
    }
    require_u64_option(&backend, "maxPollRetryCount")?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .default_headers(build_headers(&backend)?)
        .build()
        .map_err(|error| EngineAdapterRequestError::transport("runway", error.to_string()))?;
    Ok(Arc::new(RunwayMediaGenerator {
        client,
        base_url: derive_base_url(&backend.endpoint),
        backend,
    }))
}

#[derive(Clone)]
struct RunwayMediaGenerator {
    client: reqwest::Client,
    base_url: String,
    backend: MediaBackendConfig,
}

/// Error for a Runway backend option that configuration must supply. There
/// are no hardcoded defaults: if config does not provide the value, the
/// request cannot proceed.
fn missing_runway_option(key: &str) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new(
        "runway",
        format!("Runway backend must configure options.{key}."),
        "invalid_request",
    )
}

fn require_timeout_ms(backend: &MediaBackendConfig) -> Result<u64, EngineAdapterRequestError> {
    backend
        .options
        .timeout_ms
        .ok_or_else(|| missing_runway_option("timeoutMs"))
}

fn require_u64_option(
    backend: &MediaBackendConfig,
    key: &str,
) -> Result<u64, EngineAdapterRequestError> {
    read_u64_option_opt(&backend.options.extra, key).ok_or_else(|| missing_runway_option(key))
}

fn require_string_option(
    backend: &MediaBackendConfig,
    key: &str,
) -> Result<String, EngineAdapterRequestError> {
    read_string_option_opt(&backend.options.extra, key).ok_or_else(|| missing_runway_option(key))
}

#[async_trait]
impl MediaGenerationAdapter for RunwayMediaGenerator {
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
        _batch_reporter: Option<Arc<dyn super::MediaGenerationBatchReporter>>,
    ) -> Result<super::MediaSubmission, EngineAdapterRequestError> {
        let request = prepared.render_input.clone();
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

        let submit_body = prepared.body;
        let submit_path = prepared.path;
        let mut timing = MediaTimingRecorder::start();
        let submit_started_at = chrono::Utc::now();
        let submit_started_instant = std::time::Instant::now();
        let submit = self
            .client
            .post(format!("{}/{}", self.base_url, submit_path))
            .json(&submit_body)
            .send();
        let submit_response = tokio::select! {
            _ = cancellation.cancelled() => {
                return Err(EngineAdapterRequestError::new("runway", "Media generation cancelled.", "cancelled"));
            }
            result = submit => result.map_err(|error| EngineAdapterRequestError::transport("runway", error.to_string()))?
        };

        let submit_status = submit_response.status().as_u16();
        if !submit_response.status().is_success() {
            let error_body = submit_response.bounded_text().await.unwrap_or_default();
            return Err(EngineAdapterRequestError::new(
                "runway",
                if error_body.trim().is_empty() {
                    format!("Runway task submission failed with HTTP {}.", submit_status)
                } else {
                    error_body
                },
                classify_http_status(submit_status),
            )
            .with_status_code(submit_status));
        }

        let raw: Value = submit_response.bounded_json().await.map_err(|error| {
            EngineAdapterRequestError::invalid_response("runway", error.to_string())
        })?;
        timing.record_phase(
            "submit task",
            "provider-call",
            false,
            submit_started_at,
            submit_started_instant,
        );
        let job = super::jobs::RemoteJob::new(
            Arc::new(RunwayTransport {
                adapter: self.clone(),
                kind: request.kind,
            }),
            raw,
            submit_body,
            activity_reporter,
            request.options.count.unwrap_or(1),
            timing,
        )?;
        job.announce(cancellation).await?;
        Ok(super::MediaSubmission::Pending(Box::new(job)))
    }

    async fn cancel(&self, provider_job_id: &str) -> Result<(), crate::EngineAdapterRequestError> {
        super::jobs::validate_id("runway", provider_job_id)?;
        let response = self
            .client
            .delete(format!("{}/tasks/{}", self.base_url, provider_job_id))
            .send()
            .await
            .map_err(|error| EngineAdapterRequestError::transport("runway", error.to_string()))?;
        if !response.status().is_success() {
            return Err(EngineAdapterRequestError::new(
                "runway",
                format!(
                    "Runway task cancellation failed with HTTP {}.",
                    response.status().as_u16()
                ),
                classify_http_status(response.status().as_u16()),
            )
            .with_status_code(response.status().as_u16()));
        }
        Ok(())
    }
}

/// Maps a free-form aspect ratio string or pixel-dimension pair to one of
/// Runway Gen-4 Image's accepted `ratio` values. Runway uses pixel-dimension
/// strings (`"1024:1024"`) rather than aspect-ratio fractions (`"16:9"`) —
/// the planner and flow authors emit either form so we normalise here.
/// Unknown values are rejected before dispatch; an unrecognised user choice
/// must never become a silently different square generation.
fn normalize_runway_image_ratio(value: &str) -> Option<&'static str> {
    Some(match value.trim() {
        // Valid Runway Gen-4 image pixel dimensions — pass through unchanged
        "1024:1024" => "1024:1024",
        "1080:1080" => "1080:1080",
        "1168:880" => "1168:880",
        "1360:768" => "1360:768",
        "1440:1080" => "1440:1080",
        "1080:1440" => "1080:1440",
        "1808:768" => "1808:768",
        "1920:1080" => "1920:1080",
        "1080:1920" => "1080:1920",
        "2112:912" => "2112:912",
        "1280:720" => "1280:720",
        "720:1280" => "720:1280",
        "720:720" => "720:720",
        "960:720" => "960:720",
        "720:960" => "720:960",
        "1680:720" => "1680:720",
        // Aspect-ratio fractions and named aliases → nearest pixel dimensions
        "1:1" | "square" => "1024:1024",
        "16:9" | "landscape" => "1280:720",
        "9:16" | "portrait" => "1080:1920",
        "4:3" => "1440:1080",
        "3:4" => "1080:1440",
        "21:9" => "2112:912",
        "3:2" | "5:4" => "1440:1080",
        "2:3" | "4:5" => "1080:1440",
        _ => return None,
    })
}

/// Whether `value` is a ratio Runway's image API recognises (either a
/// pixel-dimension string or a fractional / named alias the
/// `normalize_runway_image_ratio` table maps onto one). Used by the
/// engine-core parity tests to verify the picker vocabulary stays
/// in sync with the adapter.
pub(crate) fn accepts_image_ratio(value: &str) -> bool {
    matches!(
        value.trim(),
        "1024:1024"
            | "1080:1080"
            | "1168:880"
            | "1360:768"
            | "1440:1080"
            | "1080:1440"
            | "1808:768"
            | "1920:1080"
            | "1080:1920"
            | "2112:912"
            | "1280:720"
            | "720:1280"
            | "720:720"
            | "960:720"
            | "720:960"
            | "1680:720"
            | "1:1"
            | "square"
            | "16:9"
            | "landscape"
            | "9:16"
            | "portrait"
            | "4:3"
            | "3:4"
            | "21:9"
            | "3:2"
            | "5:4"
            | "2:3"
            | "4:5"
    )
}

/// Builds the Runway task-submission payload for image or video generation.
///
/// Blank prompt text is rejected. Prompt text exceeding the configured
/// `options.maxPromptChars` limit is rejected with an explicit error — the
/// flow or the planner must produce a shorter prompt; the prompt is never
/// silently truncated. Image and video requests map ratios, duration, seed,
/// and reference images from the render request first and fall back to the
/// backend's configured defaults — never to hardcoded literals.
pub(crate) fn build_submit_body(
    backend: &MediaBackendConfig,
    request: &MediaRenderRequest,
) -> Result<Value, EngineAdapterRequestError> {
    let prompt_text = request.prompt_text.trim();
    if prompt_text.is_empty() {
        return Err(EngineAdapterRequestError::new(
            "runway",
            "Runway prompt text must not be empty.",
            "invalid_request",
        ));
    }
    let max_prompt_chars = usize::try_from(require_u64_option(backend, "maxPromptChars")?)
        .map_err(|_| missing_runway_option("maxPromptChars"))?;
    if prompt_text.len() > max_prompt_chars {
        return Err(EngineAdapterRequestError::new(
            "runway",
            format!(
                "Runway prompt text is {} characters but the limit is {}. \
                 Shorten the prompt in your flow before sending to Runway.",
                prompt_text.len(),
                max_prompt_chars
            ),
            "invalid_request",
        ));
    }

    match request.kind {
        MediaKind::Audio => Err(EngineAdapterRequestError::new(
            "runway",
            "Runway transport does not support audio generation.",
            "invalid_request",
        )),
        MediaKind::Image => {
            if backend.options.transport.as_deref() == Some("model-router") {
                let config_id = require_string_option(backend, "configId")?;
                let requested_ratio = request.options.aspect_ratio.as_deref().unwrap_or("1:1");
                let aspect_ratio = normalize_router_aspect_ratio(requested_ratio).ok_or_else(|| {
                    EngineAdapterRequestError::new(
                        "runway",
                        format!("Runway Model Router does not support image aspect ratio \"{requested_ratio}\"."),
                        "invalid_request",
                    )
                })?;
                let mut input = json!({
                    "promptText": prompt_text,
                    "aspectRatio": aspect_ratio,
                });
                if !request.references.is_empty() {
                    input["referenceImages"] = json!(request
                        .references
                        .iter()
                        .map(reference_to_runway)
                        .collect::<Vec<_>>());
                }
                if let Some(count) = request.options.count {
                    input["outputCount"] = json!(count.max(1));
                }
                if let Some(resolution) =
                    read_string_option_opt(&backend.options.extra, "resolution")
                {
                    input["resolution"] = json!(resolution);
                }
                return Ok(json!({ "configId": config_id, "input": input }));
            }
            let requested_ratio = match request.options.aspect_ratio.clone() {
                Some(ratio) => ratio,
                None => require_string_option(backend, "defaultAspectRatio")?,
            };
            let ratio = normalize_runway_image_ratio(&requested_ratio).ok_or_else(|| {
                EngineAdapterRequestError::new(
                    "runway",
                    format!("Runway does not support image aspect ratio \"{requested_ratio}\"."),
                    "invalid_request",
                )
            })?;
            let mut body = json!({
                "model": backend.model,
                "promptText": prompt_text,
                "ratio": ratio,
            });
            if !request.references.is_empty() {
                body["referenceImages"] = json!(request
                    .references
                    .iter()
                    .map(reference_to_runway)
                    .collect::<Vec<_>>());
            }
            if let Some(seed) = request.options.seed {
                body["seed"] = json!(seed);
            }
            if let Some(count) = request.options.count {
                body["outputCount"] = json!(count.max(1));
            }
            Ok(body)
        }
        MediaKind::Video => {
            let duration = match request.options.duration_seconds {
                Some(duration) => duration,
                None => u32::try_from(require_u64_option(backend, "defaultVideoDurationSeconds")?)
                    .map_err(|_| missing_runway_option("defaultVideoDurationSeconds"))?,
            };
            let ratio = match request.options.aspect_ratio.clone() {
                Some(ratio) => ratio,
                None => {
                    let key = if has_reference_image(&request.references) {
                        "defaultVideoReferenceRatio"
                    } else {
                        "defaultVideoRatio"
                    };
                    require_string_option(backend, key)?
                }
            };
            let mut body = json!({
                "model": backend.model,
                "promptText": prompt_text,
                "ratio": ratio,
                "duration": duration,
            });
            if let Some(seed) = request.options.seed {
                body["seed"] = json!(seed);
            }
            if let Some(reference) = request.references.iter().find(|reference| {
                reference
                    .mime_type
                    .as_deref()
                    .map(|mime| mime.starts_with("image/"))
                    .unwrap_or(true)
            }) {
                body["promptImage"] = json!(reference.url);
            }
            Ok(body)
        }
    }
}

fn normalize_router_aspect_ratio(value: &str) -> Option<&'static str> {
    match value.trim() {
        "1:1" | "square" | "1024:1024" | "1080:1080" => Some("1:1"),
        "16:9" | "landscape" | "1360:768" | "1808:768" | "1920:1080" => Some("16:9"),
        "9:16" | "portrait" | "1080:1920" | "720:1280" => Some("9:16"),
        "4:3" | "1440:1080" | "960:720" => Some("4:3"),
        "3:4" | "1080:1440" | "720:960" => Some("3:4"),
        "21:9" | "2112:912" | "1680:720" => Some("21:9"),
        "3:2" => Some("3:2"),
        "2:3" => Some("2:3"),
        _ => None,
    }
}

/// Chooses the Runway submission path from the render kind and whether an image reference is present.
pub(crate) fn build_submit_path(
    backend: &MediaBackendConfig,
    request: &MediaRenderRequest,
) -> Result<&'static str, EngineAdapterRequestError> {
    if backend.options.transport.as_deref() == Some("model-router") {
        return match request.kind {
            MediaKind::Audio => Err(EngineAdapterRequestError::new(
                "runway",
                "Unsupported audio generation.",
                "invalid_request",
            )),
            MediaKind::Image => Ok("generate/image"),
            MediaKind::Video => Ok("generate/video"),
        };
    }
    Ok(match request.kind {
        MediaKind::Audio => {
            return Err(EngineAdapterRequestError::new(
                "runway",
                "Unsupported audio generation.",
                "invalid_request",
            ))
        }
        MediaKind::Image => "text_to_image",
        MediaKind::Video => {
            if has_reference_image(&request.references) {
                "image_to_video"
            } else {
                "text_to_video"
            }
        }
    })
}

/// Returns whether the request should be treated as having an image reference.
///
/// Missing MIME types are treated as potentially image-like so they can still drive image-to-video flows.
fn has_reference_image(references: &[MediaReference]) -> bool {
    references.iter().any(|reference| {
        reference
            .mime_type
            .as_deref()
            .map(|mime| mime.starts_with("image/"))
            .unwrap_or(true)
    })
}

/// Converts a provider-neutral media reference into the Runway reference payload shape.
fn reference_to_runway(reference: &MediaReference) -> Value {
    json!({
        "uri": reference.url,
        "tag": format!("{:?}", reference.role).to_lowercase(),
    })
}

/// Reduces a configured Runway endpoint to a reusable base URL by trimming known task-path suffixes.
fn derive_base_url(endpoint: &str) -> String {
    let trimmed = normalize_base_url(endpoint);
    for suffix in [
        "/text_to_image",
        "/text_to_video",
        "/image_to_video",
        "/tasks",
    ] {
        if trimmed.ends_with(suffix) {
            return trimmed.trim_end_matches(suffix).to_string();
        }
    }
    trimmed
}

/// Builds the default Runway request headers, applying provider defaults for the version header and
/// optional trace diagnostics.
fn build_headers(backend: &MediaBackendConfig) -> Result<HeaderMap, EngineAdapterRequestError> {
    let mut headers = HeaderMap::new();
    let api_key = backend.auth.api_key.clone().unwrap_or_default();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|error| EngineAdapterRequestError::transport("runway", error.to_string()))?,
    );

    let version_header = backend.auth.version_header.clone().ok_or_else(|| {
        EngineAdapterRequestError::new(
            "runway",
            "Runway backend must configure auth.versionHeader.",
            "invalid_request",
        )
    })?;
    let version = backend.auth.version.clone().ok_or_else(|| {
        EngineAdapterRequestError::new(
            "runway",
            "Runway backend must configure auth.version.",
            "invalid_request",
        )
    })?;
    headers.insert(
        HeaderName::from_bytes(version_header.as_bytes())
            .map_err(|error| EngineAdapterRequestError::transport("runway", error.to_string()))?,
        HeaderValue::from_str(&version)
            .map_err(|error| EngineAdapterRequestError::transport("runway", error.to_string()))?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if read_bool_option(&backend.options.extra, "trace", false) {
        headers.insert(
            HeaderName::from_static("x-battersea-trace"),
            HeaderValue::from_static("true"),
        );
    }
    Ok(headers)
}

/// Classifies Runway HTTP statuses into the shared adapter error categories.
fn classify_http_status(status: u16) -> &'static str {
    match status {
        401 | 403 => "auth",
        429 => "rate_limit",
        500..=599 => "server",
        _ => "request",
    }
}

#[derive(Clone)]
struct RunwayTransport {
    adapter: RunwayMediaGenerator,
    kind: MediaKind,
}
#[async_trait]
impl super::jobs::JobTransport for RunwayTransport {
    fn provider(&self) -> &str {
        "runway"
    }
    fn backend(&self) -> &MediaBackendConfig {
        &self.adapter.backend
    }
    fn decode(
        &self,
        raw: Value,
        submission: bool,
    ) -> Result<super::jobs::RemoteUpdate, EngineAdapterRequestError> {
        decode_task(raw, self.kind.clone(), submission)
    }
    async fn poll(&self, id: &str) -> Result<super::jobs::RemoteUpdate, EngineAdapterRequestError> {
        let response = self
            .adapter
            .client
            .get(format!("{}/tasks/{}", self.adapter.base_url, id))
            .send()
            .await
            .map_err(|e| EngineAdapterRequestError::transport("runway", e.to_string()))?;
        decode_task(
            super::jobs::json_response("runway", response).await?,
            self.kind.clone(),
            false,
        )
    }
    async fn cancel(&self, id: &str) -> Result<super::MediaJobStatus, EngineAdapterRequestError> {
        self.adapter.cancel(id).await?;
        Ok(super::MediaJobStatus::Cancelled)
    }
}
fn decode_task(
    raw: Value,
    kind: MediaKind,
    submission: bool,
) -> Result<super::jobs::RemoteUpdate, EngineAdapterRequestError> {
    use super::MediaJobStatus as Status;
    let bad = |message| EngineAdapterRequestError::invalid_response("runway", message);
    let id = raw["id"]
        .as_str()
        .ok_or_else(|| bad("Task has no identity."))?
        .to_owned();
    super::jobs::validate_id("runway", &id)?;
    let status = match raw["status"].as_str() {
        None if submission => Status::Queued,
        Some("PENDING" | "THROTTLED") => Status::Queued,
        Some("RUNNING") => Status::Running,
        Some("SUCCEEDED") => Status::Succeeded,
        Some("FAILED") => Status::Failed,
        Some("CANCELED" | "CANCELLED") => Status::Cancelled,
        _ => return Err(bad("Unknown Runway task status.")),
    };
    let mut assets = Vec::new();
    if status == Status::Succeeded {
        let outputs = raw["output"]
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| bad("Succeeded task has no output URLs."))?;
        for output in outputs {
            let url = output
                .as_str()
                .ok_or_else(|| bad("Task output must be a URL."))?;
            reqwest::Url::parse(url).map_err(|_| bad("Invalid task output URL."))?;
            assets.push(MediaAsset {
                url: url.into(),
                mime_type: None,
                media_type: if kind == MediaKind::Video {
                    MediaRenderType::Video
                } else {
                    MediaRenderType::Image
                },
                width: None,
                height: None,
                duration_seconds: None,
                provider_asset_id: Some(id.clone()),
            });
        }
    }
    Ok(super::jobs::RemoteUpdate {
        id,
        status,
        assets,
        expires_at: (status == Status::Succeeded)
            .then(|| chrono::Utc::now() + chrono::Duration::hours(24)),
        expiry_is_estimate: true,
        failure: raw["failure"]
            .as_str()
            .or_else(|| raw["failure"]["message"].as_str())
            .map(str::to_owned),
        progress: raw["progress"]
            .as_f64()
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, 1.0) as f32),
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_headers, build_submit_body, build_submit_path, create_runway_media_generator,
        derive_base_url, has_reference_image, normalize_runway_image_ratio, reference_to_runway,
    };
    use crate::adapter::{EngineAuthConfig, EngineBackendOptions};
    use crate::media::{MediaBackendConfig, MediaRenderRequest};
    use battersea_model::media::{
        MediaBackendCapabilities, MediaCapability, MediaGenerationHints, MediaKind, MediaReference,
        MediaReferenceRole,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    fn sample_backend() -> MediaBackendConfig {
        MediaBackendConfig {
            id: "runway-1".to_string(),
            provider: "runway".to_string(),
            capability: MediaCapability::ImageGeneration,
            label: "Runway".to_string(),
            enabled: true,
            endpoint: "https://api.runwayml.com/v1/text_to_image".to_string(),
            model: "gen4".to_string(),
            capabilities: MediaBackendCapabilities {
                supported_media_parameters: vec![
                    "backend".into(),
                    "count".into(),
                    "aspect_ratio".into(),
                    "seed".into(),
                    "duration_seconds".into(),
                ],
                supports_partial_image_streaming: false,
                ..MediaBackendCapabilities::mock()
            },
            options: EngineBackendOptions {
                extra: std::collections::BTreeMap::from([
                    ("pollIntervalMs".to_string(), serde_json::json!(2000)),
                    ("maxPollRetryCount".to_string(), serde_json::json!(2)),
                    ("maxPromptChars".to_string(), serde_json::json!(1000)),
                    (
                        "defaultAspectRatio".to_string(),
                        serde_json::json!("1024:1024"),
                    ),
                    (
                        "defaultVideoRatio".to_string(),
                        serde_json::json!("1280:720"),
                    ),
                    (
                        "defaultVideoReferenceRatio".to_string(),
                        serde_json::json!("1280:768"),
                    ),
                    (
                        "defaultVideoDurationSeconds".to_string(),
                        serde_json::json!(5),
                    ),
                ]),
                timeout_ms: Some(600_000),
                ..EngineBackendOptions::default()
            },
            auth: EngineAuthConfig {
                auth_type: "bearer".to_string(),
                api_key_env: "RUNWAY_API_KEY".to_string(),
                header: None,
                version_header: Some("X-Runway-Version".to_string()),
                version: Some("2024-11-06".to_string()),
                has_api_key: true,
                api_key: Some("secret".to_string()),
            },
            short_description: String::new(),
            long_description: String::new(),
        }
    }

    fn image_reference(
        url: &str,
        mime_type: Option<&str>,
        role: MediaReferenceRole,
    ) -> MediaReference {
        MediaReference {
            url: url.to_string(),
            mime_type: mime_type.map(ToOwned::to_owned),
            role,
        }
    }

    fn image_request() -> MediaRenderRequest {
        MediaRenderRequest {
            kind: MediaKind::Image,
            prompt_text: "  Paint a scene  ".to_string(),
            negative_prompt: None,
            references: vec![
                image_reference(
                    "https://example.test/style.png",
                    Some("image/png"),
                    MediaReferenceRole::Style,
                ),
                image_reference(
                    "https://example.test/character.jpg",
                    Some("image/jpeg"),
                    MediaReferenceRole::Character,
                ),
            ],
            options: MediaGenerationHints {
                aspect_ratio: Some("16:9".to_string()),
                duration_seconds: None,
                count: Some(1),
                seed: Some(99),
                size: None,
            },
        }
    }

    fn video_request(references: Vec<MediaReference>) -> MediaRenderRequest {
        MediaRenderRequest {
            kind: MediaKind::Video,
            prompt_text: "  Animate this moment  ".to_string(),
            negative_prompt: None,
            references,
            options: MediaGenerationHints {
                aspect_ratio: None,
                duration_seconds: None,
                count: None,
                seed: Some(7),
                size: None,
            },
        }
    }

    #[test]
    fn constructor_derives_trimmed_base_url_and_rejects_invalid_headers() {
        let mut backend = sample_backend();
        backend.endpoint = "https://api.runwayml.com/v1/tasks".to_string();
        assert!(create_runway_media_generator(backend).is_ok());

        let mut invalid = sample_backend();
        invalid.auth.version_header = Some("bad header".to_string());
        let error = match create_runway_media_generator(invalid) {
            Ok(_) => panic!("invalid header should fail"),
            Err(error) => error,
        };
        assert_eq!(&*error.provider, "runway");
        assert_eq!(error.classification.as_str(), "transport");
    }

    #[test]
    fn build_submit_body_rejects_prompt_text_over_1000_chars() {
        let backend = sample_backend();
        let mut request = image_request();
        request.prompt_text = "x".repeat(1001);
        let error = build_submit_body(&backend, &request).expect_err("over-long prompt");
        assert_eq!(error.classification.as_str(), "invalid_request");
        assert!(
            error.message.contains("1001"),
            "should report actual length"
        );
        assert!(error.message.contains("1000"), "should report limit");

        // Exactly at the limit is accepted
        let mut at_limit = image_request();
        at_limit.prompt_text = "x".repeat(1000);
        assert!(build_submit_body(&backend, &at_limit).is_ok());
    }

    #[test]
    fn build_submit_body_rejects_blank_prompts_and_shapes_image_requests() {
        let backend = sample_backend();
        let mut blank = image_request();
        blank.prompt_text = "   ".to_string();
        let error = build_submit_body(&backend, &blank).expect_err("blank prompt");
        assert_eq!(error.classification.as_str(), "invalid_request");

        let body = build_submit_body(&backend, &image_request()).expect("image body");
        assert_eq!(body["model"], json!("gen4"));
        assert_eq!(body["promptText"], json!("Paint a scene"));
        // "16:9" normalises to the nearest valid Runway pixel dimension
        assert_eq!(body["ratio"], json!("1280:720"));
        assert_eq!(body["seed"], json!(99));
        assert_eq!(body["outputCount"], json!(1));
        assert_eq!(
            body["referenceImages"][0]["uri"],
            json!("https://example.test/style.png")
        );
        assert_eq!(body["referenceImages"][0]["tag"], json!("style"));
        assert_eq!(body["referenceImages"][1]["tag"], json!("character"));
    }

    #[test]
    fn model_router_uses_its_distinct_config_id_transport() {
        let mut backend = sample_backend();
        backend.options.transport = Some("model-router".into());
        backend
            .options
            .extra
            .insert("configId".to_string(), json!("draft-images"));
        backend
            .options
            .extra
            .insert("resolution".to_string(), json!("2k"));
        let body = build_submit_body(&backend, &image_request()).expect("router body");
        assert_eq!(body["configId"], json!("draft-images"));
        assert_eq!(body["input"]["promptText"], json!("Paint a scene"));
        assert_eq!(body["input"]["aspectRatio"], json!("16:9"));
        assert_eq!(body["input"]["outputCount"], json!(1));
        assert_eq!(body["input"]["resolution"], json!("2k"));
        assert_eq!(
            build_submit_path(&backend, &image_request()).expect("router path"),
            "generate/image"
        );
    }

    #[test]
    fn normalize_runway_image_ratio_maps_fractions_and_aliases_to_pixel_dimensions() {
        for (input, expected) in [
            ("1:1", "1024:1024"),
            ("square", "1024:1024"),
            ("16:9", "1280:720"),
            ("landscape", "1280:720"),
            ("9:16", "1080:1920"),
            ("portrait", "1080:1920"),
            ("4:3", "1440:1080"),
            ("3:4", "1080:1440"),
            ("21:9", "2112:912"),
            ("3:2", "1440:1080"),
            ("2:3", "1080:1440"),
        ] {
            assert_eq!(
                normalize_runway_image_ratio(input),
                Some(expected),
                "input: {input}"
            );
        }
    }

    #[test]
    fn normalize_runway_image_ratio_passes_through_valid_pixel_dimensions() {
        for dim in [
            "1024:1024",
            "1080:1080",
            "1168:880",
            "1360:768",
            "1440:1080",
            "1080:1440",
            "1808:768",
            "1920:1080",
            "1080:1920",
            "2112:912",
            "1280:720",
            "720:1280",
            "720:720",
            "960:720",
            "720:960",
            "1680:720",
        ] {
            assert_eq!(
                normalize_runway_image_ratio(dim),
                Some(dim),
                "dimension: {dim}"
            );
        }
    }

    #[test]
    fn normalize_runway_image_ratio_rejects_unknown_values() {
        assert_eq!(normalize_runway_image_ratio("custom-21x9"), None);
        assert_eq!(normalize_runway_image_ratio(""), None);
    }

    #[test]
    fn build_submit_body_shapes_video_requests_with_and_without_image_references() {
        let backend = sample_backend();
        let with_image = video_request(vec![image_reference(
            "https://example.test/frame.png",
            Some("image/png"),
            MediaReferenceRole::Scene,
        )]);
        let with_image_body = build_submit_body(&backend, &with_image).expect("video body");
        assert_eq!(with_image_body["promptText"], json!("Animate this moment"));
        assert_eq!(with_image_body["duration"], json!(5));
        assert_eq!(with_image_body["ratio"], json!("1280:768"));
        assert_eq!(
            with_image_body["promptImage"],
            json!("https://example.test/frame.png")
        );
        assert_eq!(with_image_body["seed"], json!(7));

        let without_image = video_request(vec![image_reference(
            "https://example.test/video-reference.txt",
            Some("text/plain"),
            MediaReferenceRole::Scene,
        )]);
        let without_image_body = build_submit_body(&backend, &without_image).expect("video body");
        assert_eq!(without_image_body["ratio"], json!("1280:720"));
        assert!(without_image_body.get("promptImage").is_none());
    }

    #[test]
    fn build_submit_path_switches_between_text_and_image_video_modes() {
        assert_eq!(
            build_submit_path(&sample_backend(), &image_request()).expect("path"),
            "text_to_image"
        );
        assert_eq!(
            build_submit_path(
                &sample_backend(),
                &video_request(vec![image_reference(
                    "https://example.test/frame.png",
                    Some("image/png"),
                    MediaReferenceRole::Scene,
                )])
            )
            .expect("path"),
            "image_to_video"
        );
        assert_eq!(
            build_submit_path(
                &sample_backend(),
                &video_request(vec![image_reference(
                    "https://example.test/meta.txt",
                    Some("text/plain"),
                    MediaReferenceRole::Scene,
                )])
            )
            .expect("path"),
            "text_to_video"
        );
    }

    #[test]
    fn has_reference_image_and_reference_to_runway_follow_documented_rules() {
        assert!(has_reference_image(&[image_reference(
            "https://example.test/image.png",
            Some("image/png"),
            MediaReferenceRole::Character,
        )]));
        assert!(has_reference_image(&[image_reference(
            "https://example.test/unknown",
            None,
            MediaReferenceRole::Style,
        )]));
        assert!(!has_reference_image(&[image_reference(
            "https://example.test/file.txt",
            Some("text/plain"),
            MediaReferenceRole::Style,
        )]));

        assert_eq!(
            reference_to_runway(&image_reference(
                "https://example.test/style.png",
                Some("image/png"),
                MediaReferenceRole::Style,
            )),
            json!({"uri":"https://example.test/style.png","tag":"style"})
        );
    }

    #[test]
    fn derive_base_url_and_build_headers_apply_provider_defaults() {
        assert_eq!(
            derive_base_url("https://api.runwayml.com/v1/text_to_image"),
            "https://api.runwayml.com/v1"
        );
        assert_eq!(
            derive_base_url("https://api.runwayml.com/v1/image_to_video"),
            "https://api.runwayml.com/v1"
        );
        assert_eq!(
            derive_base_url("https://api.runwayml.com/v1/tasks"),
            "https://api.runwayml.com/v1"
        );

        let default_headers = build_headers(&sample_backend()).expect("headers");
        assert_eq!(
            default_headers.get("authorization").expect("authorization"),
            "Bearer secret"
        );
        assert_eq!(
            default_headers
                .get("x-runway-version")
                .expect("default version"),
            "2024-11-06"
        );

        let mut traced = sample_backend();
        traced.options.extra = BTreeMap::from([("trace".to_string(), json!(true))]);
        let traced_headers = build_headers(&traced).expect("traced headers");
        assert_eq!(
            traced_headers
                .get("x-battersea-trace")
                .expect("trace header"),
            "true"
        );

        let mut invalid_value = sample_backend();
        invalid_value.auth.version = Some("bad\nvalue".to_string());
        assert_eq!(
            build_headers(&invalid_value)
                .expect_err("invalid value")
                .classification
                .as_str(),
            "transport"
        );
    }
}
