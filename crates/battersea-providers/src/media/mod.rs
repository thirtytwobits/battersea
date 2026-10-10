//! Copyright (c) Scott A Dixon
//!
//! Provider implementations and shared media transport helpers.

#[cfg(feature = "google")]
pub(crate) mod google;
#[cfg(feature = "mock")]
pub(crate) mod mock;
#[cfg(feature = "openai")]
pub(crate) mod openai;
#[cfg(all(
    test,
    feature = "openai",
    feature = "google",
    feature = "runway",
    feature = "mock"
))]
mod request_preparation_tests;
#[cfg(feature = "runway")]
pub(crate) mod runway;

use crate::adapter::error::EngineAdapterRequestError;
#[cfg(test)]
use crate::adapter::{EngineAuthConfig, EngineBackendOptions};

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[cfg(feature = "google")]
pub(crate) use google::create_google_media_generator;
#[cfg(feature = "mock")]
pub(crate) use mock::create_mock_media_generator;
#[cfg(feature = "openai")]
pub(crate) use openai::create_openai_media_generator;
#[cfg(feature = "runway")]
pub(crate) use runway::create_runway_media_generator;

/// Represents one configured media backend from the runtime config.
pub use battersea_model::media::*;

pub type MediaRenderTiming = MediaProvenanceTiming;

#[derive(Debug)]
pub struct MediaTimingRecorder {
    started_at: chrono::DateTime<chrono::Utc>,
    started_instant: Instant,
    phases: Vec<MediaProvenanceTimingPhase>,
}

impl MediaTimingRecorder {
    pub fn start() -> Self {
        Self {
            started_at: chrono::Utc::now(),
            started_instant: Instant::now(),
            phases: Vec::new(),
        }
    }

    pub fn record_phase(
        &mut self,
        name: impl Into<String>,
        category: impl Into<String>,
        included_in_model_processing: bool,
        started_at: chrono::DateTime<chrono::Utc>,
        started_instant: Instant,
    ) {
        let completed_at = chrono::Utc::now();
        self.phases.push(MediaProvenanceTimingPhase {
            name: name.into(),
            started_at: started_at.to_rfc3339(),
            completed_at: completed_at.to_rfc3339(),
            duration_ms: duration_ms(started_instant.elapsed()),
            category: category.into(),
            included_in_model_processing,
        });
    }

    pub fn finish_client_estimate(self) -> MediaRenderTiming {
        let completed_at = chrono::Utc::now();
        let elapsed_ms = duration_ms(self.started_instant.elapsed());
        let included_phases = self
            .phases
            .iter()
            .filter(|phase| phase.included_in_model_processing)
            .collect::<Vec<_>>();
        let included_sum = included_phases
            .iter()
            .map(|phase| phase.duration_ms)
            .sum::<u64>();
        let model_processing_ms = if !included_phases.is_empty() {
            Some(included_sum)
        } else if self.phases.is_empty() {
            Some(elapsed_ms)
        } else {
            None
        };
        MediaProvenanceTiming {
            started_at: self.started_at.to_rfc3339(),
            completed_at: completed_at.to_rfc3339(),
            elapsed_ms,
            model_processing_ms,
            model_processing_source: if model_processing_ms.is_some() {
                MediaProvenanceTimingSource::ClientEstimate
            } else {
                MediaProvenanceTimingSource::Unavailable
            },
            confidence: if model_processing_ms.is_some() {
                MediaProvenanceTimingConfidence::BestEffort
            } else {
                MediaProvenanceTimingConfidence::NotAvailable
            },
            phases: self.phases,
        }
    }
}

pub fn duration_ms(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Returns true when the resolved (provider, model) adapter accepts
/// `value` as an `aspect_ratio` hint at dispatch time. Used by
/// engine-core's media-parameters vocabulary parity tests so the
/// option lists exposed to the UI can never drift from what the
/// adapter actually parses.
pub fn accepts_aspect_ratio(provider: &str, model: &str, value: &str) -> bool {
    crate::builtin_registry()
        .media_provider(provider)
        .is_ok_and(|p| (p.accepts_aspect_ratio)(model, value))
}

/// Returns true when the resolved (provider, model) adapter accepts
/// `value` as an explicit `size` hint. Same role as
/// `accepts_aspect_ratio`.
pub fn accepts_size(provider: &str, model: &str, value: &str) -> bool {
    crate::builtin_registry()
        .media_provider(provider)
        .is_ok_and(|p| (p.accepts_size)(model, value))
}

/// Runs estimate-backed progress for the lifetime of a media request. Reporter
/// failure interrupts the request; completion drops any pending progress send.
pub async fn with_estimated_generation_progress<T>(
    backend: &MediaBackendConfig,
    slot_count: usize,
    activity_reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
    cancellation: CancellationToken,
    work: impl std::future::Future<Output = Result<T, EngineAdapterRequestError>>,
) -> Result<T, EngineAdapterRequestError> {
    let progress = async {
        let Some(reporter) = activity_reporter else {
            return futures_util::future::pending().await;
        };
        let addressable_slots = slot_count.min(u8::MAX as usize);
        if addressable_slots == 0 {
            return futures_util::future::pending().await;
        }
        emit_estimated_generation_progress_ticks(
            reporter,
            addressable_slots,
            resolve_estimated_generation_ms(backend),
            read_estimated_progress_interval_ms(&backend.options.extra),
            cancellation.clone(),
        )
        .await
    };
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(EngineAdapterRequestError::new(&backend.provider, "Media generation cancelled.", "cancelled")),
        result = progress => {
            result?;
            Err(EngineAdapterRequestError::new(&backend.provider, "Media generation cancelled.", "cancelled"))
        },
        result = work => result,
    }
}

/// Normalises a configured base URL before provider-specific suffix trimming.
///
/// This helper strips only trailing `/` characters and otherwise preserves the input verbatim.
#[cfg(any(test, feature = "openai", feature = "runway"))]
pub(crate) fn normalize_base_url(value: &str) -> String {
    value.trim_end_matches('/').to_string()
}

/// Reduces OpenAI-style endpoint URLs to a reusable API base URL.
///
/// For parseable URLs this removes `/responses`, `/images/generations`, or `/images` suffixes and
/// clears query and fragment data. Unparseable inputs fall back to plain suffix trimming.
#[cfg(any(test, feature = "openai"))]
pub(crate) fn resolve_openai_base_url(endpoint: &str) -> String {
    if let Ok(mut url) = reqwest::Url::parse(endpoint) {
        for suffix in ["/responses", "/images/generations", "/images"] {
            if url.path().ends_with(suffix) {
                let next = url.path().trim_end_matches(suffix).to_string();
                url.set_path(if next.is_empty() { "/" } else { &next });
                break;
            }
        }
        url.set_query(None);
        url.set_fragment(None);
        return normalize_base_url(url.as_str());
    }

    normalize_base_url(
        endpoint
            .trim_end_matches("/responses")
            .trim_end_matches("/images/generations")
            .trim_end_matches("/images"),
    )
}

/// Reads a string option from the backend options map while honouring a fallback value.
///
/// Blank, missing, and non-string values all fall back.
#[cfg(any(test, feature = "mock"))]
pub(crate) fn read_string_option(
    options: &BTreeMap<String, Value>,
    key: &str,
    fallback: &str,
) -> String {
    options
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// Reads an integer-like option from the backend options map, accepting unsigned integers,
/// positive signed integers, and numeric strings before falling back.
pub(crate) fn read_u64_option(options: &BTreeMap<String, Value>, key: &str, fallback: u64) -> u64 {
    options
        .get(key)
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().and_then(|inner| u64::try_from(inner).ok()))
                .or_else(|| value.as_str().and_then(|inner| inner.parse::<u64>().ok()))
        })
        .unwrap_or(fallback)
}

/// Reads a string option with NO fallback. Returns `None` when the key is
/// absent, blank, or non-string so callers can treat a missing value as a
/// hard error rather than silently substituting a default.
#[cfg(any(test, feature = "google", feature = "runway"))]
pub(crate) fn read_string_option_opt(
    options: &BTreeMap<String, Value>,
    key: &str,
) -> Option<String> {
    options
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// Reads an integer-like option with NO fallback. Returns `None` when the
/// key is absent or unparsable so callers can treat a missing value as a
/// hard error rather than silently substituting a default.
#[cfg(any(test, feature = "openai", feature = "runway"))]
pub(crate) fn read_u64_option_opt(options: &BTreeMap<String, Value>, key: &str) -> Option<u64> {
    options.get(key).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|inner| u64::try_from(inner).ok()))
            .or_else(|| value.as_str().and_then(|inner| inner.parse::<u64>().ok()))
    })
}

pub fn resolve_estimated_generation_ms(backend: &MediaBackendConfig) -> u64 {
    let seconds = backend.capabilities.estimated_generation_seconds;
    if !seconds.is_finite() || seconds <= 0.0 {
        return 1;
    }
    ((seconds * 1_000.0).round() as u64).max(1)
}

fn read_estimated_progress_interval_ms(options: &BTreeMap<String, Value>) -> u64 {
    read_u64_option(options, "estimatedProgressIntervalMs", 1_000).clamp(1, 30_000)
}

async fn emit_estimated_generation_progress_ticks(
    reporter: Arc<dyn MediaGenerationActivityReporter>,
    slot_count: usize,
    estimated_ms: u64,
    interval_ms: u64,
    cancellation: CancellationToken,
) -> Result<(), crate::EngineAdapterRequestError> {
    const PROGRESS_CAP: f32 = 0.95;
    let started = Instant::now();
    let interval = Duration::from_millis(interval_ms);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => {
                break;
            }
            _ = tokio::time::sleep(interval) => {}
        }
        if cancellation.is_cancelled() {
            break;
        }
        let elapsed_ms = duration_ms(started.elapsed());
        let progress =
            ((elapsed_ms as f32) / (estimated_ms as f32) * PROGRESS_CAP).clamp(0.0, PROGRESS_CAP);
        let eta_ms = estimated_ms.saturating_sub(elapsed_ms);
        for index in 0..slot_count {
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
                    eta_ms: Some(eta_ms),
                    preview_asset: None,
                    partial_index: None,
                })
                .await?;
        }
    }
    Ok(())
}

/// Reads a boolean option from the backend options map, accepting boolean scalars and boolean
/// strings before falling back.
#[cfg(any(test, feature = "runway"))]
pub(crate) fn read_bool_option(
    options: &BTreeMap<String, Value>,
    key: &str,
    fallback: bool,
) -> bool {
    options
        .get(key)
        .and_then(|value| {
            value
                .as_bool()
                .or_else(|| value.as_str().and_then(|inner| inner.parse::<bool>().ok()))
        })
        .unwrap_or(fallback)
}

/// Prepares a submit body without credentials, clients, activity or I/O.
pub fn prepare_media_request(
    backend: &MediaBackendConfig,
    request: MediaRenderRequest,
) -> Result<PreparedMediaRequest, EngineAdapterRequestError> {
    crate::builtin_registry().prepare_media(backend, request)
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_base_url, read_bool_option, read_string_option, read_u64_option,
        resolve_estimated_generation_ms, resolve_openai_base_url,
        with_estimated_generation_progress, MediaBackendConfig, MediaGenerationActivityReporter,
        MediaGenerationActivityUpdate, MediaTimingRecorder,
    };
    use battersea_model::media::{MediaProvenanceTimingConfidence, MediaProvenanceTimingSource};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;

    #[test]
    fn normalize_base_url_trims_only_trailing_slashes() {
        assert_eq!(
            normalize_base_url("https://example.test/api///"),
            "https://example.test/api"
        );
        assert_eq!(
            normalize_base_url(" https://example.test/api "),
            " https://example.test/api "
        );
    }

    #[test]
    fn resolve_openai_base_url_preserves_v1_for_image_endpoints() {
        assert_eq!(
            resolve_openai_base_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            resolve_openai_base_url("https://api.openai.com/v1/images/generations"),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            resolve_openai_base_url("https://api.openai.com/v1/images"),
            "https://api.openai.com/v1"
        );
    }

    #[test]
    fn resolve_openai_base_url_strips_provider_suffixes_and_query_data() {
        assert_eq!(
            resolve_openai_base_url("https://api.openai.com/v1/responses?foo=1#frag"),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            resolve_openai_base_url("custom-endpoint/responses"),
            "custom-endpoint"
        );
    }

    #[test]
    fn read_string_option_trims_strings_and_falls_back_otherwise() {
        let mut options = BTreeMap::new();
        options.insert("good".to_string(), json!("  value  "));
        options.insert("blank".to_string(), json!("   "));
        options.insert("wrong".to_string(), json!(12));

        assert_eq!(read_string_option(&options, "good", "fallback"), "value");
        assert_eq!(
            read_string_option(&options, "blank", "fallback"),
            "fallback"
        );
        assert_eq!(
            read_string_option(&options, "wrong", "fallback"),
            "fallback"
        );
        assert_eq!(
            read_string_option(&options, "missing", "fallback"),
            "fallback"
        );
    }

    #[test]
    fn read_u64_option_accepts_numeric_scalars_and_strings_then_falls_back() {
        let mut options = BTreeMap::new();
        options.insert("u64".to_string(), json!(42_u64));
        options.insert("i64".to_string(), json!(7_i64));
        options.insert("string".to_string(), json!("99"));
        options.insert("negative".to_string(), json!(-1));
        options.insert("wrong".to_string(), json!("oops"));

        assert_eq!(read_u64_option(&options, "u64", 5), 42);
        assert_eq!(read_u64_option(&options, "i64", 5), 7);
        assert_eq!(read_u64_option(&options, "string", 5), 99);
        assert_eq!(read_u64_option(&options, "negative", 5), 5);
        assert_eq!(read_u64_option(&options, "wrong", 5), 5);
        assert_eq!(read_u64_option(&options, "missing", 5), 5);
    }

    #[test]
    fn read_bool_option_accepts_bool_scalars_and_strings_then_falls_back() {
        let mut options = BTreeMap::new();
        options.insert("bool".to_string(), json!(true));
        options.insert("string".to_string(), json!("false"));
        options.insert("wrong".to_string(), json!("maybe"));

        assert!(read_bool_option(&options, "bool", false));
        assert!(!read_bool_option(&options, "string", true));
        assert!(read_bool_option(&options, "wrong", true));
        assert!(!read_bool_option(&options, "missing", false));
    }

    #[test]
    fn estimated_generation_resolver_uses_configured_backend_capability() {
        #[cfg(test)]
        use crate::adapter::{EngineAuthConfig, EngineBackendOptions};
        use battersea_model::media::{MediaBackendCapabilities, MediaCapability};

        fn blank_auth() -> EngineAuthConfig {
            EngineAuthConfig {
                auth_type: String::new(),
                api_key_env: String::new(),
                header: None,
                version_header: None,
                version: None,
                has_api_key: false,
                api_key: None,
            }
        }

        fn backend_with(estimated_generation_seconds: f64) -> MediaBackendConfig {
            MediaBackendConfig {
                id: "test".to_string(),
                provider: "synthetic".to_string(),
                capability: MediaCapability::ImageGeneration,
                label: "Test".to_string(),
                enabled: true,
                endpoint: "https://example.invalid".to_string(),
                model: "synthetic-media-model".to_string(),
                capabilities: MediaBackendCapabilities {
                    supported_media_parameters: Vec::new(),
                    aspect_ratio_options: Vec::new(),
                    size_options: Vec::new(),
                    supports_partial_image_streaming: false,
                    estimated_generation_seconds,
                },
                options: EngineBackendOptions::default(),
                auth: blank_auth(),
                short_description: String::new(),
                long_description: String::new(),
            }
        }

        assert_eq!(resolve_estimated_generation_ms(&backend_with(8.0)), 8_000);
        assert_eq!(resolve_estimated_generation_ms(&backend_with(1.5)), 1_500);
        assert_eq!(resolve_estimated_generation_ms(&backend_with(-3.0)), 1);
    }

    fn progress_test_backend() -> MediaBackendConfig {
        use crate::adapter::{EngineAuthConfig, EngineBackendOptions};
        use battersea_model::media::{MediaBackendCapabilities, MediaCapability};
        let mut options = EngineBackendOptions::default();
        options.extra.insert(
            "estimatedProgressIntervalMs".to_string(),
            serde_json::json!(5),
        );
        MediaBackendConfig {
            id: "google-image".to_string(),
            provider: "google".to_string(),
            capability: MediaCapability::ImageGeneration,
            label: "Google image".to_string(),
            enabled: true,
            endpoint: "https://example.invalid".to_string(),
            model: "synthetic-media-model".to_string(),
            capabilities: MediaBackendCapabilities {
                supported_media_parameters: Vec::new(),
                aspect_ratio_options: Vec::new(),
                size_options: Vec::new(),
                supports_partial_image_streaming: false,
                estimated_generation_seconds: 0.02,
            },
            options,
            auth: EngineAuthConfig {
                auth_type: String::new(),
                api_key_env: String::new(),
                header: None,
                version_header: None,
                version: None,
                has_api_key: false,
                api_key: None,
            },
            short_description: String::new(),
            long_description: String::new(),
        }
    }

    #[tokio::test]
    async fn progress_failure_and_cancellation_drop_the_owned_media_request() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct WorkGuard(Arc<AtomicBool>);
        impl Drop for WorkGuard {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        struct Reporter {
            block: bool,
            entered: Arc<tokio::sync::Notify>,
        }
        #[async_trait::async_trait]
        impl MediaGenerationActivityReporter for Reporter {
            async fn report_activity(
                &self,
                _: MediaGenerationActivityUpdate,
            ) -> Result<(), crate::EngineAdapterRequestError> {
                self.entered.notify_one();
                if self.block {
                    futures_util::future::pending::<()>().await;
                }
                Err(crate::EngineAdapterRequestError::invalid_response(
                    "test",
                    "consumer refused progress",
                ))
            }
        }
        for block in [false, true] {
            let dropped = Arc::new(AtomicBool::new(false));
            let guard = WorkGuard(dropped.clone());
            let entered = Arc::new(tokio::sync::Notify::new());
            let cancellation = CancellationToken::new();
            let reporter = Arc::new(Reporter {
                block,
                entered: entered.clone(),
            });
            let work = async move {
                let _guard = guard;
                futures_util::future::pending::<Result<(), crate::EngineAdapterRequestError>>()
                    .await
            };
            let backend = progress_test_backend();
            let run = with_estimated_generation_progress(
                &backend,
                1,
                Some(reporter),
                cancellation.clone(),
                work,
            );
            let cancel = async {
                entered.notified().await;
                if block {
                    cancellation.cancel();
                }
            };
            let (result, ()) =
                tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(run, cancel) })
                    .await
                    .expect("blocked reporter must be interruptible");
            assert!(result.is_err());
            assert!(
                dropped.load(Ordering::SeqCst),
                "owned request drops on reporter failure or cancellation"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn estimated_progress_ticker_reports_alive_request_progress_below_completion() {
        use std::sync::Mutex;

        struct CapturingReporter {
            updates: Mutex<Vec<MediaGenerationActivityUpdate>>,
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

        let reporter = Arc::new(CapturingReporter {
            updates: Mutex::new(Vec::new()),
        });
        let backend = progress_test_backend();
        let cancellation = CancellationToken::new();
        with_estimated_generation_progress(
            &backend,
            2,
            Some(reporter.clone()),
            cancellation,
            async {
                for _ in 0..20 {
                    if reporter.updates.lock().expect("updates lock").len() >= 4 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Ok(())
            },
        )
        .await
        .expect("progress delivery");
        let updates = reporter.updates.lock().expect("updates lock").clone();

        assert!(
            updates.len() >= 4,
            "expected ticks for both slots, got {updates:?}"
        );
        assert!(updates.iter().all(|update| update.progress.is_some()));
        assert!(updates.iter().all(|update| update.progress.unwrap() < 1.0));
        assert!(updates.iter().any(|update| update.slot_index == Some(0)));
        assert!(updates.iter().any(|update| update.slot_index == Some(1)));
    }

    #[test]
    fn timing_recorder_excludes_poll_and_retry_wait_from_model_processing() {
        let mut recorder = MediaTimingRecorder::start();
        recorder.record_phase(
            "poll wait",
            "poll-wait",
            false,
            chrono::Utc::now(),
            Instant::now(),
        );
        recorder.record_phase(
            "poll retry wait",
            "retry-wait",
            false,
            chrono::Utc::now(),
            Instant::now(),
        );

        let timing = recorder.finish_client_estimate();

        assert_eq!(timing.model_processing_ms, None);
        assert_eq!(
            timing.model_processing_source,
            MediaProvenanceTimingSource::Unavailable
        );
        assert_eq!(
            timing.confidence,
            MediaProvenanceTimingConfidence::NotAvailable
        );
        assert_eq!(timing.phases.len(), 2);
        assert!(timing
            .phases
            .iter()
            .all(|phase| !phase.included_in_model_processing));
    }
}
