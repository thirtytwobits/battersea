//! Copyright (c) Scott A Dixon
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MediaCapability {
    PromptPlanner,
    ImageGeneration,
    VideoGeneration,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaRenderType {
    Image,
    Video,
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
#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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
pub trait MediaGenerationActivityReporter: Send + Sync {
    fn report_activity(&self, update: MediaGenerationActivityUpdate);
}

/// Receives staged partial media batches while a generation call is still running.
pub trait MediaGenerationBatchReporter: Send + Sync {
    fn report_batch(&self, result: MediaRenderResult);
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
        let submission = match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, submit)
                .await
                .map_err(|_| {
                    EngineAdapterRequestError::new(
                        &self.backend().provider,
                        "Media submission deadline exceeded.",
                        crate::ErrorKind::Timeout,
                    )
                })??,
            None => submit.await?,
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
    ) -> Result<MediaRenderResult, EngineAdapterRequestError>;
    async fn cancel(&self) -> Result<(), EngineAdapterRequestError>;
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
                            Some(deadline) => tokio::time::timeout_at(deadline, job.wait(cancellation.clone())).await
                                .map_err(|_| EngineAdapterRequestError::new("media", "Media job deadline exceeded.", crate::ErrorKind::Timeout))?,
                            None => job.wait(cancellation.clone()).await,
                        }
                    } => result,
                };
                if result.as_ref().is_err_and(|error| {
                    matches!(
                        error.classification,
                        crate::ErrorKind::Timeout | crate::ErrorKind::Cancelled
                    )
                }) {
                    // An expired generation deadline cannot also be the cancellation deadline.
                    let _ =
                        tokio::time::timeout(std::time::Duration::from_secs(5), job.cancel()).await;
                }
                result
            }
        }
    }
}
