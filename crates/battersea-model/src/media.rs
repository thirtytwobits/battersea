//! Copyright (c) Scott A Dixon
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MediaCapability {
    PromptPlanner,
    ImageGeneration,
    VideoGeneration,
    AudioGeneration,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaRenderType {
    Image,
    Video,
    Audio,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MediaReferenceRole {
    Character,
    Style,
    Scene,
    Pose,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct MediaReference {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub role: MediaReferenceRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq, Default)]
pub struct MediaGenerationHints {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Output-size hint. Free-form string so each provider can interpret it.
    /// Backends that don't advertise `size` in their `supported_media_parameters`
    /// have this filtered out by the engine before dispatch -- same pattern as
    /// `filter_chat_parameters_for_dispatch`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct MediaApiOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaBackendCapabilities {
    #[serde(default)]
    pub supported_media_parameters: Vec<String>,
    #[serde(default)]
    pub aspect_ratio_options: Vec<MediaApiOption>,
    #[serde(default)]
    pub size_options: Vec<MediaApiOption>,
    pub supports_partial_image_streaming: bool,
    pub estimated_generation_seconds: f64,
}

impl MediaBackendCapabilities {
    pub fn mock() -> Self {
        Self {
            supported_media_parameters: vec![
                "backend".to_string(),
                "count".to_string(),
                "size".to_string(),
                "aspect_ratio".to_string(),
                "seed".to_string(),
                "duration_seconds".to_string(),
            ],
            aspect_ratio_options: vec![
                MediaApiOption {
                    value: "1:1".to_string(),
                    label: "Square (1:1)".to_string(),
                },
                MediaApiOption {
                    value: "3:4".to_string(),
                    label: "Portrait (3:4)".to_string(),
                },
                MediaApiOption {
                    value: "9:16".to_string(),
                    label: "Portrait tall (9:16)".to_string(),
                },
                MediaApiOption {
                    value: "4:3".to_string(),
                    label: "Landscape (4:3)".to_string(),
                },
                MediaApiOption {
                    value: "16:9".to_string(),
                    label: "Landscape wide (16:9)".to_string(),
                },
            ],
            size_options: vec![
                MediaApiOption {
                    value: "auto".to_string(),
                    label: "Auto (let provider choose)".to_string(),
                },
                MediaApiOption {
                    value: "1024x1024".to_string(),
                    label: "1024x1024 (square)".to_string(),
                },
                MediaApiOption {
                    value: "1024x1536".to_string(),
                    label: "1024x1536 (portrait)".to_string(),
                },
                MediaApiOption {
                    value: "1536x1024".to_string(),
                    label: "1536x1024 (landscape)".to_string(),
                },
            ],
            supports_partial_image_streaming: false,
            estimated_generation_seconds: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MediaProvenanceTimingSource {
    Provider,
    ClientEstimate,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MediaProvenanceTimingConfidence {
    Authoritative,
    BestEffort,
    NotAvailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct MediaProvenanceTimingPhase {
    pub name: String,
    pub started_at: String,
    pub completed_at: String,
    pub duration_ms: u64,
    pub category: String,
    pub included_in_model_processing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct MediaProvenanceTiming {
    pub started_at: String,
    pub completed_at: String,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_processing_ms: Option<u64>,
    pub model_processing_source: MediaProvenanceTimingSource,
    pub confidence: MediaProvenanceTimingConfidence,
    #[serde(default)]
    pub phases: Vec<MediaProvenanceTimingPhase>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaAsset {
    pub url: String,
    pub mime_type: Option<String>,
    pub media_type: MediaRenderType,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_seconds: Option<u32>,
    pub provider_asset_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControllerActivityState {
    Idle,
    Working,
    Waiting,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControllerActivityEvent {
    ErrorTransient,
    Error,
    Success,
}

use crate::adapter::{EngineAuthConfig, EngineBackendOptions};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaBackendConfig {
    pub id: String,
    pub provider: String,
    pub capability: MediaCapability,
    pub label: String,
    pub enabled: bool,
    pub endpoint: String,
    pub model: String,
    pub capabilities: MediaBackendCapabilities,
    pub options: EngineBackendOptions,
    pub auth: EngineAuthConfig,
    /// Single-line summary, ≤100 characters. Same role as
    /// `EngineBackendConfig::short_description`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub short_description: String,
    /// Multi-line markdown description, assembled across the
    /// engine.yaml family/mixin merge chain. See the chat-backend
    /// counterpart for the full semantics.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub long_description: String,
}

use crate::EngineAdapterRequestError;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
#[derive(Debug, Clone, PartialEq)]
pub struct MediaRenderRequest {
    pub kind: MediaKind,
    pub prompt_text: String,
    pub negative_prompt: Option<String>,
    pub references: Vec<MediaReference>,
    pub options: MediaGenerationHints,
}

/// Returns the provider job handle and the assets produced for a render request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaRenderResult {
    pub provider_job_id: Option<String>,
    pub assets: Vec<MediaAsset>,
    /// The literal request body the adapter sent to the provider, as
    /// JSON. For multi-step flows (submit + poll) this is the
    /// generation-defining submit body. `None` when the adapter does
    /// not capture it. Captured for media provenance / replay; never
    /// contains credentials (those ride in headers, not the body).
    pub provider_request: Option<Value>,
    /// The literal response body the adapter received from the
    /// provider, as JSON. For multi-step flows this is the terminal
    /// (success) response. `None` when the adapter does not capture
    /// it. Partial batch results never carry this — only the final
    /// result of a `generate` call does.
    pub provider_response: Option<Value>,
    /// Provider-call timing captured by the adapter. This covers the
    /// active render call only; staging, asset downloads, persistence,
    /// and dossier-save work happen later and are intentionally excluded.
    pub timing: Option<MediaProvenanceTiming>,
}

impl MediaRenderResult {
    /// Builds a result with no captured provider envelopes — used by
    /// partial batch reports and by adapters that don't (yet) capture
    /// the raw bodies.
    pub fn without_envelopes(provider_job_id: Option<String>, assets: Vec<MediaAsset>) -> Self {
        Self {
            provider_job_id,
            assets,
            provider_request: None,
            provider_response: None,
            timing: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaGenerationActivityUpdate {
    pub state: ControllerActivityState,
    pub event: Option<ControllerActivityEvent>,
    pub message: String,
    pub provider_job_id: Option<String>,
    pub error_code: Option<String>,
    /// Slot id this progress update describes, when the adapter can attribute
    /// progress to a single requested image and happens to know the engine's
    /// slot id. Adapters typically don't (slot ids are minted engine-side); use
    /// [`Self::slot_index`] instead, which the engine resolves to the matching
    /// slot id.
    pub slot_id: Option<String>,
    /// Zero-based index of the requested image this progress update describes.
    /// The engine maps the index to the corresponding slot id when both
    /// `slot_id` and `slot_index` are present, `slot_id` wins.
    pub slot_index: Option<u8>,
    /// Fractional progress in `[0.0, 1.0]` for the addressed slot.
    pub progress: Option<f32>,
    /// Adapter's best estimate of remaining time for this slot, in ms.
    pub eta_ms: Option<u64>,
    /// Transient, browser-renderable preview asset for the addressed slot.
    /// The engine emits it as a slot preview event; it is not staged into the
    /// temporary media store.
    pub preview_asset: Option<MediaAsset>,
    /// Zero-based provider partial index for `preview_asset`, when known.
    pub partial_index: Option<u8>,
}

/// Receives coarse activity updates from provider-specific media generators.
#[allow(clippy::double_must_use)] // async_trait marks its boxed Future must_use.
#[async_trait]
pub trait MediaGenerationActivityReporter: Send + Sync {
    async fn report_activity(
        &self,
        update: MediaGenerationActivityUpdate,
    ) -> Result<(), EngineAdapterRequestError>;
}

/// Receives staged partial media batches while a generation call is still running.
#[allow(clippy::double_must_use)] // async_trait marks its boxed Future must_use.
#[async_trait]
pub trait MediaGenerationBatchReporter: Send + Sync {
    async fn report_batch(
        &self,
        result: MediaRenderResult,
    ) -> Result<(), EngineAdapterRequestError>;
}

/// Immutable inputs shared by capture and dispatch. Only preparation constructs this value.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedMediaRequest {
    pub render_input: MediaRenderRequest,
    pub path: String,
    pub body: Value,
    pub prompt_text: String,
}

impl PreparedMediaRequest {
    pub fn body(&self) -> &Value {
        &self.body
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn provider_prompt_text(&self) -> &str {
        &self.prompt_text
    }
}

/// Generates media assets from a fully resolved render request.
// async-trait adds must_use to boxed futures, which already carry it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait MediaGenerationAdapter: Send + Sync {
    fn backend(&self) -> &MediaBackendConfig;

    fn prepare(
        &self,
        request: MediaRenderRequest,
    ) -> Result<PreparedMediaRequest, EngineAdapterRequestError>;

    async fn generate(
        &self,
        request: MediaRenderRequest,
        cancellation: CancellationToken,
        activity_reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
        batch_reporter: Option<Arc<dyn MediaGenerationBatchReporter>>,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        let prepared = self.prepare(request)?;
        let deadline = self
            .backend()
            .options
            .timeout_ms
            .map(|ms| tokio::time::Instant::now() + std::time::Duration::from_millis(ms));
        let submit = self.submit(
            prepared,
            cancellation.clone(),
            activity_reporter,
            batch_reporter,
        );
        let submission_work = async {
            Ok::<_, EngineAdapterRequestError>(match deadline {
                Some(deadline) => {
                    tokio::time::timeout_at(deadline, submit)
                        .await
                        .map_err(|_| {
                            EngineAdapterRequestError::new(
                                &self.backend().provider,
                                "Media submission deadline exceeded.",
                                crate::ErrorKind::Timeout,
                            )
                        })??
                }
                None => submit.await?,
            })
        };
        let submission = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(EngineAdapterRequestError::new(&self.backend().provider, "Media submission cancelled.", crate::ErrorKind::Cancelled)),
            result = submission_work => result?,
        };
        submission.wait(cancellation, deadline).await
    }

    async fn submit(
        &self,
        prepared: PreparedMediaRequest,
        cancellation: CancellationToken,
        activity_reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
        batch_reporter: Option<Arc<dyn MediaGenerationBatchReporter>>,
    ) -> Result<MediaSubmission, EngineAdapterRequestError>;

    async fn cancel(&self, _provider_job_id: &str) -> Result<(), EngineAdapterRequestError> {
        Err(EngineAdapterRequestError::new(
            &self.backend().provider,
            "Provider has no remote cancellation operation.",
            crate::ErrorKind::InvalidRequest,
        ))
    }
}

/// A synchronous provider returns its result directly. A remote provider returns an accepted job.
pub enum MediaSubmission {
    Complete(MediaRenderResult),
    Pending(Box<dyn MediaJob>),
}
// async-trait adds must_use to boxed futures, which already carry it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait MediaJob: Send + Sync {
    fn id(&self) -> &str;
    async fn wait(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        let policy = self.poll_policy();
        policy.validate()?;
        let mut retries = 0_u32;
        loop {
            let result = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(EngineAdapterRequestError::new("media", "Media job cancelled.", crate::ErrorKind::Cancelled)),
                result = self.poll(cancellation.clone()) => result,
            };
            let delay = match result {
                Ok(MediaJobStatus::Succeeded) => return self.retrieve(cancellation).await,
                Ok(MediaJobStatus::Queued | MediaJobStatus::Running) => {
                    retries = 0;
                    policy.interval_ms
                }
                Ok(status) => {
                    return Err(EngineAdapterRequestError::new(
                        "media",
                        format!("Media job is {status:?}."),
                        match status {
                            MediaJobStatus::Cancelled => crate::ErrorKind::Cancelled,
                            MediaJobStatus::Expired => crate::ErrorKind::Expired,
                            _ => crate::ErrorKind::Provider,
                        },
                    ))
                }
                Err(error)
                    if !self.snapshot().status.is_terminal()
                        && matches!(
                            error.classification,
                            crate::ErrorKind::Transport
                                | crate::ErrorKind::RateLimit
                                | crate::ErrorKind::Server
                        )
                        && retries < policy.max_retries =>
                {
                    let delay = policy
                        .retry_delay_ms
                        .saturating_mul(1_u64 << retries.min(63))
                        .min(policy.max_retry_delay_ms)
                        .max(error.retry_after_ms.unwrap_or(0));
                    if delay > policy.max_retry_delay_ms {
                        return Err(error);
                    }
                    retries += 1;
                    delay
                }
                Err(error) => return Err(error),
            };
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(EngineAdapterRequestError::new("media", "Media job cancelled.", crate::ErrorKind::Cancelled)),
                _ = tokio::time::sleep(std::time::Duration::from_millis(delay)) => {}
            }
        }
    }
    /// Inspect local state without I/O.
    fn snapshot(&self) -> MediaJobSnapshot;
    fn poll_policy(&self) -> MediaPollPolicy;
    /// Retrieve one provider status; never submit generation.
    async fn poll(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<MediaJobStatus, EngineAdapterRequestError>;
    /// Retrieve outputs of a succeeded job. Repeated calls return the retained result.
    async fn retrieve(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError>;
    async fn cancel(&mut self) -> Result<(), EngineAdapterRequestError>;
    async fn webhook(
        &mut self,
        _webhook: MediaWebhook<'_>,
        _secret: &str,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<MediaJobStatus, EngineAdapterRequestError> {
        Err(EngineAdapterRequestError::new(
            "media",
            "This job does not support webhooks.",
            crate::ErrorKind::InvalidRequest,
        ))
    }
}
impl MediaSubmission {
    /// Wait for completion. Cancellation and expiry allow up to five seconds for remote cleanup.
    pub async fn wait(
        self,
        cancellation: CancellationToken,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        match self {
            Self::Complete(result) => Ok(result),
            Self::Pending(mut job) => {
                let result = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => Err(EngineAdapterRequestError::new("media", "Media job cancelled.", crate::ErrorKind::Cancelled)),
                    result = async {
                        match deadline {
                            Some(deadline) if deadline <= tokio::time::Instant::now() => Err(EngineAdapterRequestError::new("media", "Media job deadline exceeded.", crate::ErrorKind::Timeout)),
                            Some(deadline) => tokio::time::timeout_at(deadline, job.wait(cancellation.clone())).await
                                .map_err(|_| EngineAdapterRequestError::new("media", "Media job deadline exceeded.", crate::ErrorKind::Timeout))?,
                            None => job.wait(cancellation.clone()).await,
                        }
                    } => result,
                };
                let mut result = result.map_err(|error| {
                    error
                        .with_request_id(Some(job.id().to_owned()))
                        .with_dispatch(crate::adapter::error::DispatchState::Accepted)
                });
                if result.is_err() && !job.snapshot().status.is_terminal() {
                    let cleanup =
                        tokio::time::timeout(std::time::Duration::from_secs(5), job.cancel()).await;
                    if let Err(error) = &mut result {
                        match cleanup {
                            Ok(Ok(())) => {}
                            Ok(Err(cleanup)) => {
                                error.message = format!(
                                    "{} Remote cancellation failed: {}",
                                    error.message, cleanup.message
                                )
                                .into()
                            }
                            Err(_) => {
                                error.message = format!(
                                    "{} Remote cancellation timed out; remote outcome is unknown.",
                                    error.message
                                )
                                .into()
                            }
                        }
                    }
                }
                result
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaJobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Expired,
}
impl MediaJobStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaJobSnapshot {
    pub id: String,
    pub status: MediaJobStatus,
    pub output_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expiry_is_estimate: bool,
    pub outputs_retrieved: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct MediaPollPolicy {
    pub interval_ms: u64,
    pub retry_delay_ms: u64,
    pub max_retry_delay_ms: u64,
    pub max_retries: u32,
}
impl MediaPollPolicy {
    pub fn validate(self) -> Result<(), EngineAdapterRequestError> {
        if self.interval_ms == 0
            || self.retry_delay_ms == 0
            || self.max_retry_delay_ms < self.retry_delay_ms
        {
            return Err(EngineAdapterRequestError::new(
                "media",
                "Polling intervals must be positive and retry bounds ordered.",
                crate::ErrorKind::InvalidRequest,
            ));
        }
        Ok(())
    }
}
/// Raw signed callback. Hosts own HTTP routing and pass the unmodified request body.
pub struct MediaWebhook<'a> {
    pub id: &'a str,
    pub timestamp: &'a str,
    pub signature: &'a str,
    pub body: &'a [u8],
}
