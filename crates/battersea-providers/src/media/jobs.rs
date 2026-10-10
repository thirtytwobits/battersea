//! Shared remote media lifecycle. Provider transports only encode protocol operations.
use super::*;
use async_trait::async_trait;
use base64::Engine as _;
use battersea_model::{adapter::error::DispatchState, ErrorKind};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use reqwest::{header::CONTENT_TYPE, Url};

pub(crate) struct RemoteUpdate {
    pub id: String,
    pub status: MediaJobStatus,
    pub assets: Vec<MediaAsset>,
    pub raw: Value,
    pub expires_at: Option<DateTime<Utc>>,
    pub expiry_is_estimate: bool,
    pub failure: Option<String>,
    pub progress: Option<f32>,
}
#[allow(clippy::double_must_use)] // async_trait annotates its boxed futures.
#[async_trait]
pub(crate) trait JobTransport: Send + Sync {
    fn provider(&self) -> &str;
    fn backend(&self) -> &MediaBackendConfig;
    fn decode(
        &self,
        raw: Value,
        submission: bool,
    ) -> Result<RemoteUpdate, EngineAdapterRequestError>;
    async fn poll(&self, id: &str) -> Result<RemoteUpdate, EngineAdapterRequestError>;
    async fn cancel(&self, id: &str) -> Result<MediaJobStatus, EngineAdapterRequestError>;
    fn asset_auth(&self, _url: &Url) -> Option<&str> {
        None
    }
    fn webhook(
        &self,
        _webhook: MediaWebhook<'_>,
        _secret: &str,
        _now: DateTime<Utc>,
    ) -> Result<RemoteUpdate, EngineAdapterRequestError> {
        Err(EngineAdapterRequestError::new(
            self.provider(),
            "Provider does not support webhooks.",
            ErrorKind::InvalidRequest,
        ))
    }
}

pub(crate) fn poll_policy(
    backend: &MediaBackendConfig,
) -> Result<MediaPollPolicy, EngineAdapterRequestError> {
    let option = |key: &str| {
        backend
            .options
            .extra
            .get(key)
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
    };
    let missing = |key| {
        EngineAdapterRequestError::new(
            &backend.provider,
            format!("Media backend must configure options.{key}."),
            ErrorKind::InvalidRequest,
        )
    };
    let interval_ms = option("pollIntervalMs").ok_or_else(|| missing("pollIntervalMs"))?;
    let retry_delay_ms = option("pollRetryDelayMs").unwrap_or(interval_ms);
    let policy = MediaPollPolicy {
        interval_ms,
        retry_delay_ms,
        max_retry_delay_ms: option("maxPollRetryDelayMs")
            .unwrap_or(retry_delay_ms.saturating_mul(16)),
        max_retries: u32::try_from(
            option("maxPollRetryCount").ok_or_else(|| missing("maxPollRetryCount"))?,
        )
        .map_err(|_| missing("maxPollRetryCount"))?,
    };
    policy.validate()?;
    if let Some(value) = backend.options.extra.get("maxOutputBytes") {
        if !value.as_u64().is_some_and(|bytes| {
            bytes > 0 && bytes <= battersea_model::adapter::payload::PROVIDER_PAYLOAD_BYTES as u64
        }) {
            return Err(missing(
                "maxOutputBytes (positive and within the provider payload bound)",
            ));
        }
    }
    if backend.options.timeout_ms.filter(|ms| *ms > 0).is_none() {
        return Err(missing("timeoutMs"));
    }
    Ok(policy)
}

pub(crate) struct RemoteJob {
    transport: Arc<dyn JobTransport>,
    snapshot: MediaJobSnapshot,
    update: RemoteUpdate,
    submit_body: Value,
    policy: MediaPollPolicy,
    reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
    slot_count: u8,
    timing: Option<MediaTimingRecorder>,
    completed_timing: Option<MediaProvenanceTiming>,
    result: Option<MediaRenderResult>,
    cancel_attempted: bool,
}
impl RemoteJob {
    pub fn new(
        transport: Arc<dyn JobTransport>,
        raw: Value,
        submit_body: Value,
        reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
        slot_count: u8,
        timing: MediaTimingRecorder,
    ) -> Result<Self, EngineAdapterRequestError> {
        let id = raw["id"]
            .as_str()
            .ok_or_else(|| {
                EngineAdapterRequestError::invalid_response(
                    transport.provider(),
                    "Submission has no job identity.",
                )
            })?
            .to_owned();
        validate_id(transport.provider(), &id)?;
        let update = RemoteUpdate {
            id,
            status: MediaJobStatus::Queued,
            assets: vec![],
            raw: Value::Null,
            expires_at: None,
            expiry_is_estimate: false,
            failure: None,
            progress: None,
        };
        let policy = poll_policy(transport.backend())?;
        let mut job = Self {
            snapshot: MediaJobSnapshot {
                id: update.id.clone(),
                status: update.status,
                output_expires_at: update.expires_at,
                expiry_is_estimate: update.expiry_is_estimate,
                outputs_retrieved: false,
            },
            transport,
            update,
            submit_body,
            policy,
            reporter,
            slot_count,
            timing: Some(timing),
            completed_timing: None,
            result: None,
            cancel_attempted: false,
        };
        // Acquire cleanup ownership before interpreting the rest of an accepted response.
        let update = job.transport.decode(raw, true).map_err(|error| {
            error
                .with_request_id(Some(job.snapshot.id.clone()))
                .with_dispatch(DispatchState::Accepted)
        })?;
        let _ = job.accept(update);
        Ok(job)
    }
    fn error(&self, kind: ErrorKind, message: impl Into<String>) -> EngineAdapterRequestError {
        EngineAdapterRequestError::new(self.transport.provider(), message, kind)
            .with_request_id(Some(self.snapshot.id.clone()))
            .with_dispatch(DispatchState::Accepted)
    }
    fn accept(
        &mut self,
        update: RemoteUpdate,
    ) -> Result<MediaJobStatus, EngineAdapterRequestError> {
        if update.id != self.snapshot.id {
            return Err(self.error(
                ErrorKind::InvalidResponse,
                "Provider returned a different job identity.",
            ));
        }
        if self.snapshot.status.is_terminal() {
            return self.checked_status();
        }
        if self.snapshot.status == MediaJobStatus::Running
            && update.status == MediaJobStatus::Queued
        {
            return Ok(self.snapshot.status);
        }
        self.snapshot.status = update.status;
        if update.status.is_terminal() {
            self.completed_timing = self
                .timing
                .take()
                .map(MediaTimingRecorder::finish_client_estimate);
        }
        self.snapshot.output_expires_at = update.expires_at;
        self.snapshot.expiry_is_estimate = update.expiry_is_estimate;
        self.update = update;
        self.checked_status()
    }
    fn checked_status(&self) -> Result<MediaJobStatus, EngineAdapterRequestError> {
        match self.snapshot.status {
            MediaJobStatus::Failed => Err(self.error(
                ErrorKind::Provider,
                self.update
                    .failure
                    .as_deref()
                    .unwrap_or("Media job failed."),
            )),
            MediaJobStatus::Cancelled => {
                Err(self.error(ErrorKind::Cancelled, "Provider cancelled media job."))
            }
            MediaJobStatus::Expired => {
                Err(self.error(ErrorKind::Expired, "Media job outputs expired."))
            }
            status => Ok(status),
        }
    }
    pub(crate) async fn announce(
        &self,
        cancellation: CancellationToken,
    ) -> Result<(), EngineAdapterRequestError> {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(self.error(ErrorKind::Cancelled, "Media job reporting cancelled.")),
            result = self.report() => result,
        }
    }
    async fn report(&self) -> Result<(), EngineAdapterRequestError> {
        let Some(reporter) = &self.reporter else {
            return Ok(());
        };
        for slot_index in 0..self.slot_count.max(1) {
            reporter
                .report_activity(MediaGenerationActivityUpdate {
                    state: if self.snapshot.status == MediaJobStatus::Queued {
                        ControllerActivityState::Waiting
                    } else {
                        ControllerActivityState::Working
                    },
                    event: None,
                    message: String::new(),
                    provider_job_id: Some(self.snapshot.id.clone()),
                    error_code: None,
                    slot_id: None,
                    slot_index: Some(slot_index),
                    progress: self.update.progress,
                    eta_ms: None,
                    preview_asset: None,
                    partial_index: None,
                })
                .await?;
        }
        Ok(())
    }
    async fn retrieve_outputs(&mut self) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        if let Some(result) = &self.result {
            return Ok(result.clone());
        }
        self.checked_status()?;
        if self.snapshot.status != MediaJobStatus::Succeeded {
            return Err(self.error(ErrorKind::InvalidRequest, "Media job has not succeeded."));
        }
        if self.update.assets.is_empty() {
            return Err(self.error(
                ErrorKind::InvalidResponse,
                "Succeeded media job returned no outputs.",
            ));
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(
                self.transport.backend().options.timeout_ms.unwrap(),
            ))
            .build()
            .map_err(|error| self.error(ErrorKind::Retrieval, error.to_string()))?;
        let mut remaining = self
            .transport
            .backend()
            .options
            .extra
            .get("maxOutputBytes")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(battersea_model::adapter::payload::PROVIDER_PAYLOAD_BYTES);
        let mut assets = Vec::new();
        for asset in &self.update.assets {
            if self
                .snapshot
                .output_expires_at
                .is_some_and(|at| at <= Utc::now())
            {
                return Err(self.error(
                    ErrorKind::Expired,
                    "Media job outputs expired before retrieval.",
                ));
            }
            let mut url = Url::parse(&asset.url)
                .map_err(|_| self.error(ErrorKind::InvalidResponse, "Invalid media output URL."))?;
            let mut redirects = 0;
            let response = loop {
                if !matches!(url.scheme(), "https" | "http")
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(self.error(
                        ErrorKind::InvalidResponse,
                        "Media outputs require HTTP(S) URLs without embedded credentials.",
                    ));
                }
                let mut request = client.get(url.clone());
                if let Some(key) = self.transport.asset_auth(&url) {
                    request = request.bearer_auth(key);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|error| self.error(ErrorKind::Retrieval, error.to_string()))?;
                if response.status().is_redirection() {
                    if redirects >= 5 {
                        return Err(self.error(ErrorKind::Retrieval, "Too many output redirects."));
                    }
                    let location = response
                        .headers()
                        .get(reqwest::header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .ok_or_else(|| {
                            self.error(ErrorKind::Retrieval, "Output redirect has no location.")
                        })?;
                    let next = url.join(location).map_err(|_| {
                        self.error(ErrorKind::Retrieval, "Invalid output redirect.")
                    })?;
                    if url.scheme() == "https" && next.scheme() != "https" {
                        return Err(self.error(ErrorKind::Retrieval, "Insecure output redirect."));
                    }
                    url = next;
                    redirects += 1;
                    continue;
                }
                break response;
            };
            if !response.status().is_success() {
                return Err(self
                    .error(
                        if matches!(response.status().as_u16(), 404 | 410) {
                            ErrorKind::Expired
                        } else {
                            ErrorKind::Retrieval
                        },
                        format!(
                            "Media output retrieval failed with HTTP {}.",
                            response.status().as_u16()
                        ),
                    )
                    .with_status_code(response.status().as_u16()));
            }
            let mime = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .map(str::to_owned)
                .or_else(|| asset.mime_type.clone())
                .ok_or_else(|| {
                    self.error(ErrorKind::InvalidResponse, "Media output has no MIME type.")
                })?;
            let prefix = match asset.media_type {
                MediaRenderType::Image => "image/",
                MediaRenderType::Video => "video/",
                MediaRenderType::Audio => "audio/",
            };
            if !mime.starts_with(prefix) {
                return Err(self.error(
                    ErrorKind::InvalidResponse,
                    "Media output MIME type conflicts with requested kind.",
                ));
            }
            let mut bytes = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk =
                    chunk.map_err(|error| self.error(ErrorKind::Retrieval, error.to_string()))?;
                // Reserve enough space for the returned base64 representation before retaining bytes.
                if bytes.len().saturating_add(chunk.len()).saturating_add(2) / 3 * 4
                    > remaining.saturating_sub(mime.len() + 13)
                {
                    return Err(self.error(
                        ErrorKind::Retrieval,
                        "Media outputs exceed the aggregate byte limit.",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            if bytes.is_empty() {
                return Err(self.error(ErrorKind::Retrieval, "Media output is empty."));
            }
            let mut retained = asset.clone();
            retained.url = format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
            retained.mime_type = Some(mime);
            remaining -= retained.url.len();
            assets.push(retained);
        }
        if self
            .snapshot
            .output_expires_at
            .is_some_and(|at| at <= Utc::now())
        {
            return Err(self.error(
                ErrorKind::Expired,
                "Media job outputs expired during retrieval.",
            ));
        }
        let result = MediaRenderResult {
            provider_job_id: Some(self.snapshot.id.clone()),
            assets,
            provider_request: Some(self.submit_body.clone()),
            provider_response: Some(self.update.raw.clone()),
            timing: self.completed_timing.clone(),
        };
        battersea_model::adapter::payload::check_payload(&result, self.transport.provider())?;
        self.snapshot.outputs_retrieved = true;
        self.result = Some(result.clone());
        Ok(result)
    }
}
#[async_trait]
impl MediaJob for RemoteJob {
    fn id(&self) -> &str {
        &self.snapshot.id
    }
    fn snapshot(&self) -> MediaJobSnapshot {
        self.snapshot.clone()
    }
    fn poll_policy(&self) -> MediaPollPolicy {
        self.policy
    }
    async fn poll(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<MediaJobStatus, EngineAdapterRequestError> {
        if self.snapshot.status.is_terminal() {
            return self.checked_status();
        }
        let update = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(self.error(ErrorKind::Cancelled, "Media polling cancelled.")),
            result = self.transport.poll(&self.snapshot.id) => match result {
                Ok(update) => update,
                Err(error) => {
                    if error.classification == ErrorKind::Expired { self.snapshot.status = MediaJobStatus::Expired; }
                    return Err(error.with_request_id(Some(self.snapshot.id.clone())).with_dispatch(DispatchState::Accepted));
                }
            },
        };
        let status = self.accept(update)?;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(self.error(ErrorKind::Cancelled, "Media reporting cancelled.")),
            result = self.report() => result?,
        }
        Ok(status)
    }
    async fn retrieve(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(self.error(ErrorKind::Cancelled, "Media retrieval cancelled.")),
            result = self.retrieve_outputs() => result,
        }
    }
    async fn cancel(&mut self) -> Result<(), EngineAdapterRequestError> {
        if self.snapshot.status.is_terminal() {
            return Ok(());
        }
        self.cancel_attempted = true;
        let status = self.transport.cancel(&self.snapshot.id).await?;
        if status == MediaJobStatus::Cancelled {
            self.snapshot.status = status;
            Ok(())
        } else {
            Err(self.error(
                ErrorKind::Provider,
                "Provider has not confirmed cancellation; reconcile the accepted job.",
            ))
        }
    }
    async fn webhook(
        &mut self,
        webhook: MediaWebhook<'_>,
        secret: &str,
        now: DateTime<Utc>,
    ) -> Result<MediaJobStatus, EngineAdapterRequestError> {
        let update = self.transport.webhook(webhook, secret, now)?;
        self.accept(update)
    }
}
impl Drop for RemoteJob {
    fn drop(&mut self) {
        if self.snapshot.status.is_terminal() || self.cancel_attempted {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let transport = self.transport.clone();
            let id = self.snapshot.id.clone();
            runtime.spawn(async move {
                if !matches!(tokio::time::timeout(Duration::from_secs(5), transport.cancel(&id)).await, Ok(Ok(MediaJobStatus::Cancelled))) {
                    tracing::warn!(provider = transport.provider(), job_id = %id, "Dropped media job cancellation was not confirmed");
                }
            });
        }
    }
}

pub(crate) fn validate_id(provider: &str, id: &str) -> Result<(), EngineAdapterRequestError> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return Err(EngineAdapterRequestError::invalid_response(
            provider,
            "Invalid provider job identity.",
        ));
    }
    Ok(())
}
pub(crate) async fn json_response(
    provider: &str,
    response: reqwest::Response,
) -> Result<Value, EngineAdapterRequestError> {
    use crate::http_payload::BoundedResponse as _;
    let status = response.status();
    if !status.is_success() {
        let mut error = EngineAdapterRequestError::new(
            provider,
            format!("Media job operation failed with HTTP {}.", status.as_u16()),
            match status.as_u16() {
                401 => ErrorKind::Authentication,
                403 => ErrorKind::Permission,
                404 | 410 => ErrorKind::Expired,
                429 => ErrorKind::RateLimit,
                500..=599 => ErrorKind::Server,
                _ => ErrorKind::Request,
            },
        )
        .with_status_code(status.as_u16());
        if let Some(value) = response.headers().get(reqwest::header::RETRY_AFTER) {
            let delay = value
                .to_str()
                .ok()
                .and_then(|value| {
                    value
                        .parse::<u64>()
                        .ok()
                        .map(|seconds| seconds.saturating_mul(1000))
                        .or_else(|| {
                            DateTime::parse_from_rfc2822(value).ok().map(|at| {
                                at.timestamp_millis()
                                    .saturating_sub(Utc::now().timestamp_millis())
                                    .max(0) as u64
                            })
                        })
                })
                .ok_or_else(|| {
                    EngineAdapterRequestError::invalid_response(
                        provider,
                        "Invalid Retry-After header.",
                    )
                })?;
            error.retry_after_ms = Some(delay);
        }
        return Err(error);
    }
    response.bounded_json().await
}
