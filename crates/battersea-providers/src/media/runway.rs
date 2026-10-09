use super::{
    normalize_base_url, read_bool_option, read_string_option_opt, read_u64_option_opt,
    MediaBackendConfig, MediaGenerationActivityReporter, MediaGenerationActivityUpdate,
    MediaGenerationAdapter, MediaRenderRequest, MediaRenderResult, MediaTimingRecorder,
};
use crate::adapter::error::EngineAdapterRequestError;
use async_trait::async_trait;
use battersea_model::media::{ControllerActivityEvent, ControllerActivityState};
use battersea_model::media::{MediaAsset, MediaKind, MediaReference, MediaRenderType};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Builds a Runway media generator from backend config, deriving a reusable base URL and
/// validating auth and version headers during construction.
pub(crate) fn create_runway_media_generator(
    backend: MediaBackendConfig,
) -> Result<Arc<dyn MediaGenerationAdapter>, EngineAdapterRequestError> {
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
            reporter.report_activity(MediaGenerationActivityUpdate {
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
            });
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
            let error_body = submit_response.text().await.unwrap_or_default();
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

        let task: RunwayTask = submit_response.json().await.map_err(|error| {
            EngineAdapterRequestError::invalid_response("runway", error.to_string())
        })?;
        timing.record_phase(
            "submit task",
            "provider-call",
            false,
            submit_started_at,
            submit_started_instant,
        );
        if let Some(reporter) = &activity_reporter {
            reporter.report_activity(MediaGenerationActivityUpdate {
                state: ControllerActivityState::Waiting,
                event: None,
                message: String::new(),
                provider_job_id: Some(task.id.clone()),
                error_code: None,
                slot_id: None,
                slot_index: None,
                progress: None,
                eta_ms: None,
                preview_asset: None,
                partial_index: None,
            });
        }

        if task.id.trim().is_empty() {
            return Err(EngineAdapterRequestError::invalid_response(
                "runway",
                "Provider returned a blank job identity.",
            ));
        }
        let id = task.id.clone();
        Ok(super::MediaSubmission::Pending(Box::new(RunwayJob {
            id,
            adapter: self.clone(),
            task: Some(task),
            request,
            submit_body,
            timing: Some(timing),
            activity_reporter,
        })))
    }

    async fn cancel(&self, provider_job_id: &str) -> Result<(), EngineAdapterRequestError> {
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
            MediaKind::Image => Ok("generate/image"),
            MediaKind::Video => Ok("generate/video"),
        };
    }
    Ok(match request.kind {
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

/// Heuristically extracts media asset URLs of the requested kind from a Runway task payload.
fn extract_asset_urls(value: Value, kind: MediaKind) -> Vec<String> {
    let mut urls = Vec::new();
    extract_urls_recursive(&value, &kind, &mut urls);
    urls
}

/// Recursively walks a JSON value collecting strings that look like image or video asset URLs for the given kind.
fn extract_urls_recursive(value: &Value, kind: &MediaKind, urls: &mut Vec<String>) {
    match value {
        Value::String(inner) => {
            let looks_like_asset = if matches!(kind, &MediaKind::Video) {
                inner.starts_with("http") && (inner.contains(".mp4") || inner.contains("video"))
            } else {
                inner.starts_with("http")
                    && (inner.contains(".png")
                        || inner.contains(".jpg")
                        || inner.contains(".jpeg")
                        || inner.contains(".webp")
                        || inner.contains("image"))
            };
            if looks_like_asset {
                urls.push(inner.clone());
            }
        }
        Value::Array(items) => {
            for item in items {
                extract_urls_recursive(item, kind, urls);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                extract_urls_recursive(item, kind, urls);
            }
        }
        _ => {}
    }
}

/// Normalises provider task-status strings into a small terminal-or-pending state machine.
fn normalize_task_status(value: Option<&str>) -> TaskStatus {
    match value.unwrap_or("").trim().to_ascii_uppercase().as_str() {
        "SUCCEEDED" | "COMPLETED" => TaskStatus::Succeeded,
        "FAILED" | "ERROR" => TaskStatus::Failed,
        "CANCELLED" | "CANCELED" => TaskStatus::Cancelled,
        _ => TaskStatus::Pending,
    }
}

fn is_retryable_poll_error(error: &EngineAdapterRequestError) -> bool {
    matches!(
        error.classification.as_str(),
        "transport" | "rate_limit" | "server"
    )
}

fn report_transient_poll_error(
    activity_reporter: &Option<Arc<dyn MediaGenerationActivityReporter>>,
    provider_job_id: &str,
    error: &EngineAdapterRequestError,
) {
    if let Some(reporter) = activity_reporter {
        reporter.report_activity(MediaGenerationActivityUpdate {
            state: ControllerActivityState::Waiting,
            event: Some(ControllerActivityEvent::ErrorTransient),
            message: error.message.clone(),
            provider_job_id: Some(provider_job_id.to_string()),
            error_code: Some(error.classification.to_string()),
            slot_id: None,
            slot_index: None,
            progress: None,
            eta_ms: None,
            preview_asset: None,
            partial_index: None,
        });
    }
}

/// Forwards Runway's task-level `progress` reading to the engine as a
/// per-slot provider-sourced update for every requested image.
///
/// Runway returns one task per submit; the same progress value applies to
/// all assets the task yields, so we broadcast it across every slot index
/// the request asked for. The engine's slot tracker resolves
/// `slot_index → slot_id` against the UUIDs minted at strike start.
///
/// A no-op when there's no reporter, when the poll didn't include a
/// progress reading, or when the value isn't a finite number.
fn broadcast_runway_progress_to_all_slots(
    activity_reporter: &Option<Arc<dyn MediaGenerationActivityReporter>>,
    submitted_task: &RunwayTask,
    polled_task: &RunwayTask,
    request: &MediaRenderRequest,
) {
    let Some(reporter) = activity_reporter else {
        return;
    };
    let Some(raw) = polled_task.progress else {
        return;
    };
    if !raw.is_finite() {
        return;
    }
    let progress = raw.clamp(0.0, 1.0) as f32;
    let slot_count = request.options.count.unwrap_or(1).max(1);
    for index in 0..slot_count {
        reporter.report_activity(MediaGenerationActivityUpdate {
            state: ControllerActivityState::Working,
            event: None,
            message: String::new(),
            provider_job_id: Some(submitted_task.id.clone()),
            error_code: None,
            slot_id: None,
            slot_index: Some(index),
            progress: Some(progress),
            eta_ms: None,
            preview_asset: None,
            partial_index: None,
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskStatus {
    Pending,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Deserialize, Serialize)]
struct RunwayTask {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    /// Runway returns this on `RUNNING` tasks: a fractional 0..1 hint of how
    /// much of the work is done. Forwarded to the editor as provider-sourced
    /// per-slot progress so placeholders animate against real adapter state
    /// instead of the engine's estimated timer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    progress: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure: Option<RunwayFailure>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RunwayFailure {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

struct RunwayJob {
    id: String,
    adapter: RunwayMediaGenerator,
    task: Option<RunwayTask>,
    request: MediaRenderRequest,
    submit_body: Value,
    timing: Option<MediaTimingRecorder>,
    activity_reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
}
#[async_trait]
impl super::MediaJob for RunwayJob {
    fn id(&self) -> &str {
        &self.id
    }
    async fn wait(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        let task = self.task.take().ok_or_else(|| {
            EngineAdapterRequestError::new(
                "runway",
                "Job completion has already been consumed.",
                "invalid_request",
            )
        })?;
        self.adapter
            .wait_for_task(
                task,
                self.request.clone(),
                self.submit_body.clone(),
                self.timing.take().expect("job timing"),
                cancellation,
                self.activity_reporter.clone(),
            )
            .await
    }
    async fn cancel(&self) -> Result<(), EngineAdapterRequestError> {
        self.adapter.cancel(&self.id).await
    }
}
impl RunwayMediaGenerator {
    async fn wait_for_task(
        &self,
        task: RunwayTask,
        request: MediaRenderRequest,
        submit_body: Value,
        mut timing: MediaTimingRecorder,
        cancellation: CancellationToken,
        activity_reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        let poll_interval_ms = require_u64_option(&self.backend, "pollIntervalMs")?;
        // Retry backoff falls back to the (config-provided) poll interval when
        // its own key is absent — a value derived from configuration, not a
        // hardcoded literal.
        let poll_retry_delay_ms =
            read_u64_option_opt(&self.backend.options.extra, "pollRetryDelayMs")
                .unwrap_or(poll_interval_ms);
        let max_poll_retry_count = require_u64_option(&self.backend, "maxPollRetryCount")?;
        let mut consecutive_retryable_errors = 0_u64;
        loop {
            if cancellation.is_cancelled() {
                return Err(EngineAdapterRequestError::new(
                    "runway",
                    "Media generation cancelled.",
                    "cancelled",
                ));
            }

            let poll_started_at = chrono::Utc::now();
            let poll_started_instant = std::time::Instant::now();
            let poll = self
                .client
                .get(format!("{}/tasks/{}", self.base_url, task.id))
                .send();
            let poll_response = tokio::select! {
                _ = cancellation.cancelled() => {
                        return Err(EngineAdapterRequestError::new("runway", "Media generation cancelled.", "cancelled"));
                }
                result = poll => match result {
                    Ok(response) => response,
                    Err(error) => {
                        let mapped = EngineAdapterRequestError::transport("runway", error.to_string());
                        if is_retryable_poll_error(&mapped) && consecutive_retryable_errors < max_poll_retry_count {
                            consecutive_retryable_errors += 1;
                            report_transient_poll_error(&activity_reporter, &task.id, &mapped);
                            let retry_started_at = chrono::Utc::now();
                            let retry_started_instant = std::time::Instant::now();
                            tokio::select! {
                                _ = cancellation.cancelled() => {
                                                        return Err(EngineAdapterRequestError::new("runway", "Media generation cancelled.", "cancelled"));
                                }
                                _ = tokio::time::sleep(std::time::Duration::from_millis(poll_retry_delay_ms)) => {}
                            }
                            timing.record_phase(
                                "poll retry wait",
                                "retry-wait",
                                false,
                                retry_started_at,
                                retry_started_instant,
                            );
                            continue;
                        }
                        return Err(mapped);
                    }
                }
            };

            let poll_status = poll_response.status().as_u16();
            if !poll_response.status().is_success() {
                let error_body = poll_response.text().await.unwrap_or_default();
                let mapped = EngineAdapterRequestError::new(
                    "runway",
                    if error_body.trim().is_empty() {
                        format!("Runway task polling failed with HTTP {}.", poll_status)
                    } else {
                        error_body
                    },
                    classify_http_status(poll_status),
                )
                .with_status_code(poll_status);
                if is_retryable_poll_error(&mapped)
                    && consecutive_retryable_errors < max_poll_retry_count
                {
                    consecutive_retryable_errors += 1;
                    report_transient_poll_error(&activity_reporter, &task.id, &mapped);
                    let retry_started_at = chrono::Utc::now();
                    let retry_started_instant = std::time::Instant::now();
                    tokio::select! {
                        _ = cancellation.cancelled() => {
                                        return Err(EngineAdapterRequestError::new("runway", "Media generation cancelled.", "cancelled"));
                        }
                        _ = tokio::time::sleep(std::time::Duration::from_millis(poll_retry_delay_ms)) => {}
                    }
                    timing.record_phase(
                        "poll retry wait",
                        "retry-wait",
                        false,
                        retry_started_at,
                        retry_started_instant,
                    );
                    continue;
                }
                return Err(mapped);
            }

            let task_state: RunwayTask = poll_response.json().await.map_err(|error| {
                EngineAdapterRequestError::invalid_response("runway", error.to_string())
            })?;
            timing.record_phase(
                "poll task",
                "poll",
                false,
                poll_started_at,
                poll_started_instant,
            );
            // Forward Runway's per-task progress to the engine as a per-slot
            // reading for every requested image. Runway returns one task per
            // submit; the same `progress` value applies to all assets the
            // task will yield. The engine resolves `slot_index → slot_id`
            // against the slot UUIDs it minted at strike start.
            broadcast_runway_progress_to_all_slots(
                &activity_reporter,
                &task,
                &task_state,
                &request,
            );
            if matches!(
                task_state.status.as_deref().map(str::trim),
                Some(status) if status.eq_ignore_ascii_case("THROTTLED")
            ) {
                if let Some(reporter) = &activity_reporter {
                    reporter.report_activity(MediaGenerationActivityUpdate {
                        state: ControllerActivityState::Waiting,
                        event: Some(ControllerActivityEvent::ErrorTransient),
                        message: "Runway task throttled.".to_string(),
                        provider_job_id: Some(task.id.clone()),
                        error_code: Some("rate_limit".to_string()),
                        slot_id: None,
                        slot_index: None,
                        progress: None,
                        eta_ms: None,
                        preview_asset: None,
                        partial_index: None,
                    });
                }
                let retry_started_at = chrono::Utc::now();
                let retry_started_instant = std::time::Instant::now();
                tokio::select! {
                    _ = cancellation.cancelled() => {
                                return Err(EngineAdapterRequestError::new("runway", "Media generation cancelled.", "cancelled"));
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(poll_retry_delay_ms)) => {}
                }
                timing.record_phase(
                    "throttled retry wait",
                    "retry-wait",
                    false,
                    retry_started_at,
                    retry_started_instant,
                );
                continue;
            }

            match normalize_task_status(task_state.status.as_deref()) {
                TaskStatus::Succeeded => {
                    let kind = request.kind.clone();
                    let provider_response = serde_json::to_value(&task_state).ok();
                    let assets =
                        extract_asset_urls(task_state.output.unwrap_or(Value::Null), kind.clone())
                            .into_iter()
                            .map(|url| MediaAsset {
                                url,
                                mime_type: Some(
                                    if matches!(kind, MediaKind::Video) {
                                        "video/mp4"
                                    } else {
                                        "image/png"
                                    }
                                    .to_string(),
                                ),
                                media_type: if matches!(kind, MediaKind::Video) {
                                    MediaRenderType::Video
                                } else {
                                    MediaRenderType::Image
                                },

                                width: None,
                                height: None,
                                duration_seconds: if matches!(kind, MediaKind::Video) {
                                    request.options.duration_seconds
                                } else {
                                    None
                                },

                                provider_asset_id: None,
                            })
                            .collect::<Vec<_>>();
                    return Ok(MediaRenderResult {
                        provider_job_id: Some(task.id.clone()),
                        assets,
                        provider_request: Some(submit_body),
                        provider_response,
                        timing: Some(timing.finish_client_estimate()),
                    });
                }
                TaskStatus::Failed => {
                    return Err(EngineAdapterRequestError::new(
                        "runway",
                        task_state
                            .failure
                            .and_then(|failure| failure.message)
                            .unwrap_or_else(|| "Runway task failed.".to_string()),
                        "request",
                    ));
                }
                TaskStatus::Cancelled => {
                    return Err(EngineAdapterRequestError::new(
                        "runway",
                        "Runway task was cancelled.",
                        "cancelled",
                    ));
                }
                TaskStatus::Pending => {
                    consecutive_retryable_errors = 0;
                    let wait_started_at = chrono::Utc::now();
                    let wait_started_instant = std::time::Instant::now();
                    tokio::select! {
                        _ = cancellation.cancelled() => {
                                        return Err(EngineAdapterRequestError::new("runway", "Media generation cancelled.", "cancelled"));
                        }
                        _ = tokio::time::sleep(std::time::Duration::from_millis(poll_interval_ms)) => {}
                    }
                    timing.record_phase(
                        "poll wait",
                        "poll-wait",
                        false,
                        wait_started_at,
                        wait_started_instant,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        broadcast_runway_progress_to_all_slots, build_headers, build_submit_body,
        build_submit_path, classify_http_status, create_runway_media_generator, derive_base_url,
        extract_asset_urls, extract_urls_recursive, has_reference_image,
        normalize_runway_image_ratio, normalize_task_status, reference_to_runway, RunwayTask,
        TaskStatus,
    };
    use crate::adapter::{EngineAuthConfig, EngineBackendOptions};
    use crate::media::{
        MediaBackendConfig, MediaGenerationActivityReporter, MediaGenerationActivityUpdate,
        MediaRenderRequest,
    };
    use battersea_model::media::{
        MediaBackendCapabilities, MediaCapability, MediaGenerationHints, MediaKind, MediaReference,
        MediaReferenceRole,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

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
        assert_eq!(error.provider, "runway");
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

    #[test]
    fn classify_http_status_extract_urls_and_normalize_task_status_cover_known_cases() {
        assert_eq!(classify_http_status(401), "auth");
        assert_eq!(classify_http_status(429), "rate_limit");
        assert_eq!(classify_http_status(503), "server");
        assert_eq!(classify_http_status(400), "request");

        let nested = json!({
            "image": "https://example.test/frame.png",
            "ignored": "not-a-url",
            "nested": {
                "video": "https://example.test/clip.mp4",
                "list": [
                    "https://example.test/extra.webp",
                    42,
                    {"deep": "https://example.test/also-image.jpg"}
                ]
            }
        });
        assert_eq!(
            extract_asset_urls(nested.clone(), MediaKind::Image),
            vec![
                "https://example.test/frame.png".to_string(),
                "https://example.test/extra.webp".to_string(),
                "https://example.test/also-image.jpg".to_string(),
            ]
        );
        assert_eq!(
            extract_asset_urls(nested, MediaKind::Video),
            vec!["https://example.test/clip.mp4".to_string()]
        );

        let mut urls = Vec::new();
        extract_urls_recursive(
            &json!({"items":["https://example.test/image.png", false, "text"]}),
            &MediaKind::Image,
            &mut urls,
        );
        assert_eq!(urls, vec!["https://example.test/image.png".to_string()]);

        assert_eq!(
            normalize_task_status(Some("SUCCEEDED")),
            TaskStatus::Succeeded
        );
        assert_eq!(
            normalize_task_status(Some("completed")),
            TaskStatus::Succeeded
        );
        assert_eq!(normalize_task_status(Some("FAILED")), TaskStatus::Failed);
        assert_eq!(normalize_task_status(Some("error")), TaskStatus::Failed);
        assert_eq!(
            normalize_task_status(Some("cancelled")),
            TaskStatus::Cancelled
        );
        assert_eq!(
            normalize_task_status(Some("canceled")),
            TaskStatus::Cancelled
        );
        assert_eq!(normalize_task_status(Some("running")), TaskStatus::Pending);
        assert_eq!(normalize_task_status(None), TaskStatus::Pending);
    }

    /// Captures every `report_activity` call so tests can assert on the
    /// per-slot fan-out without standing up the real channel-backed reporter.
    #[derive(Default)]
    struct RecordingReporter {
        updates: Mutex<Vec<MediaGenerationActivityUpdate>>,
    }

    impl MediaGenerationActivityReporter for RecordingReporter {
        fn report_activity(&self, update: MediaGenerationActivityUpdate) {
            self.updates.lock().expect("lock").push(update);
        }
    }

    fn make_request(count: Option<u8>) -> MediaRenderRequest {
        MediaRenderRequest {
            kind: MediaKind::Image,
            prompt_text: "a lighthouse".to_string(),
            negative_prompt: None,
            references: Vec::new(),
            options: MediaGenerationHints {
                count,
                ..MediaGenerationHints::default()
            },
        }
    }

    fn submitted(task_id: &str) -> RunwayTask {
        RunwayTask {
            id: task_id.to_string(),
            status: Some("PENDING".to_string()),
            progress: None,
            output: None,
            failure: None,
        }
    }

    fn polled(progress: Option<f64>) -> RunwayTask {
        RunwayTask {
            id: "task-x".to_string(),
            status: Some("RUNNING".to_string()),
            progress,
            output: None,
            failure: None,
        }
    }

    /// Builds the (concrete, trait-object) pair tests use: the concrete
    /// `Arc<RecordingReporter>` to read the captured updates, plus a
    /// `Some(Arc<dyn ...>)` to pass into the broadcaster, with the same
    /// backing buffer.
    fn make_test_reporter() -> (
        Arc<RecordingReporter>,
        Option<Arc<dyn MediaGenerationActivityReporter>>,
    ) {
        let concrete = Arc::new(RecordingReporter::default());
        let dyn_handle: Arc<dyn MediaGenerationActivityReporter> = concrete.clone();
        (concrete, Some(dyn_handle))
    }

    #[test]
    fn runway_progress_fans_out_one_per_slot_with_clamped_value_and_job_id() {
        let (recorder, reporter) = make_test_reporter();
        broadcast_runway_progress_to_all_slots(
            &reporter,
            &submitted("task-x"),
            &polled(Some(0.42)),
            &make_request(Some(3)),
        );
        let updates = recorder.updates.lock().unwrap().clone();
        assert_eq!(updates.len(), 3, "one update per requested slot");
        for (index, update) in updates.iter().enumerate() {
            assert_eq!(update.slot_index, Some(index as u8));
            assert_eq!(update.provider_job_id.as_deref(), Some("task-x"));
            assert!(
                (update.progress.unwrap() - 0.42_f32).abs() < 1e-6,
                "expected ~0.42, got {:?}",
                update.progress
            );
        }
    }

    #[test]
    fn runway_progress_clamps_out_of_range_readings_and_defaults_count_to_one() {
        let (recorder, reporter) = make_test_reporter();
        broadcast_runway_progress_to_all_slots(
            &reporter,
            &submitted("task-x"),
            &polled(Some(1.7)),
            &make_request(None),
        );
        let updates = recorder.updates.lock().unwrap().clone();
        assert_eq!(updates.len(), 1, "count=None defaults to one slot");
        assert_eq!(updates[0].progress, Some(1.0));
    }

    #[test]
    fn runway_progress_is_a_noop_when_the_poll_carried_no_progress_or_was_non_finite() {
        let (recorder, reporter) = make_test_reporter();
        broadcast_runway_progress_to_all_slots(
            &reporter,
            &submitted("task-x"),
            &polled(None),
            &make_request(Some(2)),
        );
        broadcast_runway_progress_to_all_slots(
            &reporter,
            &submitted("task-x"),
            &polled(Some(f64::NAN)),
            &make_request(Some(2)),
        );
        assert!(recorder.updates.lock().unwrap().is_empty());
    }
}
