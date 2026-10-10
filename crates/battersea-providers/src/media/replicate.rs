//! Replicate prediction jobs with model-specific input mapping and signed callbacks.
use super::{jobs::*, *};
use async_trait::async_trait;
use base64::Engine as _;
use battersea_model::{adapter::error::DispatchState, ErrorKind};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;

#[derive(Clone)]
pub(crate) struct Replicate {
    backend: MediaBackendConfig,
    client: reqwest::Client,
    base: reqwest::Url,
    kind: MediaKind,
}
fn invalid(message: impl Into<String>) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new("replicate", message, ErrorKind::InvalidRequest)
}
pub(crate) fn create(
    backend: MediaBackendConfig,
) -> Result<Arc<dyn MediaGenerationAdapter>, EngineAdapterRequestError> {
    poll_policy(&backend)?;
    let base = reqwest::Url::parse(&backend.endpoint)
        .map_err(|_| invalid("Invalid Replicate endpoint."))?;
    if !matches!(base.scheme(), "http" | "https")
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err(invalid("Replicate endpoint must be an HTTP(S) base URL without credentials, query or fragment."));
    }
    if backend
        .auth
        .api_key
        .as_deref()
        .is_none_or(|v| v.trim().is_empty())
    {
        return Err(invalid("Replicate requires auth.apiKey."));
    }
    let kind = match backend.capability {
        MediaCapability::ImageGeneration => MediaKind::Image,
        MediaCapability::VideoGeneration => MediaKind::Video,
        MediaCapability::AudioGeneration => MediaKind::Audio,
        _ => return Err(invalid("Unsupported media capability.")),
    };
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(backend.options.timeout_ms.unwrap()))
        .build()
        .map_err(|error| invalid(error.to_string()))?;
    Ok(Arc::new(Replicate {
        backend,
        client,
        base,
        kind,
    }))
}
pub(crate) fn prepare(
    backend: &MediaBackendConfig,
    request: MediaRenderRequest,
) -> Result<PreparedMediaRequest, EngineAdapterRequestError> {
    poll_policy(backend)?;
    // Versioned models use a pinned version; official models use owner/name.
    let (path, version) = if let Some((model, version)) = backend.model.split_once(':') {
        validate_model(model)?;
        if version.is_empty() || !version.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return Err(invalid("Invalid Replicate model version."));
        }
        ("predictions".to_owned(), Some(version))
    } else {
        validate_model(&backend.model)?;
        (format!("models/{}/predictions", backend.model), None)
    };
    let mut input = backend
        .options
        .extra
        .get("input")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let object = input
        .as_object_mut()
        .ok_or_else(|| invalid("options.input must be an object."))?;
    let fields = backend
        .options
        .extra
        .get("inputFields")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("Configure options.inputFields for the selected model."))?;
    let mut used = std::collections::HashSet::new();
    for name in fields.values() {
        let name = name
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| invalid("Input field names must be non-empty strings."))?;
        if !used.insert(name) {
            return Err(invalid("Input field mappings must be distinct."));
        }
    }
    let mut put = |key: &str, value: Option<Value>| -> Result<(), EngineAdapterRequestError> {
        if let Some(value) = value {
            let field = fields
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(format!("Configure options.inputFields.{key}.")))?;
            if object.contains_key(field) {
                return Err(invalid(format!(
                    "Static input conflicts with mapped field {field}."
                )));
            }
            object.insert(field.into(), value);
        }
        Ok(())
    };
    put("prompt", Some(json!(request.prompt_text)))?;
    put(
        "negative_prompt",
        request.negative_prompt.as_ref().map(|v| json!(v)),
    )?;
    put(
        "references",
        (!request.references.is_empty()).then(|| {
            json!(request
                .references
                .iter()
                .map(|r| &r.url)
                .collect::<Vec<_>>())
        }),
    )?;
    put("count", request.options.count.map(|v| json!(v)))?;
    put("size", request.options.size.as_ref().map(|v| json!(v)))?;
    put(
        "aspect_ratio",
        request.options.aspect_ratio.as_ref().map(|v| json!(v)),
    )?;
    put("seed", request.options.seed.map(|v| json!(v)))?;
    put(
        "duration_seconds",
        request.options.duration_seconds.map(|v| json!(v)),
    )?;
    let mut body = json!({"input": input});
    if let Some(version) = version {
        body["version"] = json!(version);
    }
    if let Some(webhook) = backend.options.extra.get("webhookUrl") {
        let url = webhook
            .as_str()
            .and_then(|s| reqwest::Url::parse(s).ok())
            .filter(|u| u.scheme() == "https" && u.username().is_empty() && u.password().is_none())
            .ok_or_else(|| invalid("webhookUrl must be an HTTPS URL."))?;
        body["webhook"] = json!(url.as_str());
        body["webhook_events_filter"] = json!(["completed"]);
    }
    if let Some(pointer) = backend.options.extra.get("outputPointer") {
        if !pointer
            .as_str()
            .is_some_and(|s| s.is_empty() || s.starts_with('/'))
        {
            return Err(invalid(
                "outputPointer must be a JSON pointer relative to output.",
            ));
        }
    }
    battersea_model::adapter::payload::check_payload(&body, "replicate")?;
    Ok(PreparedMediaRequest {
        path,
        body,
        prompt_text: request.prompt_text.clone(),
        render_input: request,
    })
}
fn validate_model(model: &str) -> Result<(), EngineAdapterRequestError> {
    let parts: Vec<_> = model.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|s| {
            s.is_empty()
                || !s
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        })
    {
        return Err(invalid(
            "Replicate model must be owner/name, optionally followed by :version.",
        ));
    }
    Ok(())
}
impl Replicate {
    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base.as_str().trim_end_matches('/'), path)
    }
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, self.url(path))
            .bearer_auth(self.backend.auth.api_key.as_deref().unwrap())
    }
    fn decode(&self, raw: Value) -> Result<RemoteUpdate, EngineAdapterRequestError> {
        let bad = |message| EngineAdapterRequestError::invalid_response("replicate", message);
        let id = raw["id"]
            .as_str()
            .ok_or_else(|| bad("Prediction has no identity."))?
            .to_owned();
        validate_id("replicate", &id)?;
        let status = match raw["status"].as_str() {
            Some("starting") => MediaJobStatus::Queued,
            Some("processing") => MediaJobStatus::Running,
            Some("succeeded") => MediaJobStatus::Succeeded,
            Some("failed") => MediaJobStatus::Failed,
            Some("canceled") => MediaJobStatus::Cancelled,
            _ => return Err(bad("Unknown prediction status.")),
        };
        let removed = raw["data_removed"].as_bool().unwrap_or(false);
        let expires_at = match raw["completed_at"].as_str() {
            Some(value) => Some(
                DateTime::parse_from_rfc3339(value)
                    .map_err(|_| bad("Invalid completion timestamp."))?
                    .with_timezone(&Utc)
                    + chrono::Duration::hours(1),
            ),
            None => None,
        };
        let mut assets = Vec::new();
        if status == MediaJobStatus::Succeeded && !removed {
            let output = &raw["output"];
            let output = match self
                .backend
                .options
                .extra
                .get("outputPointer")
                .and_then(Value::as_str)
            {
                Some(pointer) => output
                    .pointer(pointer)
                    .ok_or_else(|| bad("Configured output pointer is absent."))?,
                None => output,
            };
            let urls: Vec<&Value> = match output {
                Value::Array(values) => values.iter().collect(),
                Value::String(_) => vec![output],
                _ => {
                    return Err(bad(
                        "Prediction output must contain a media URL or array of URLs.",
                    ))
                }
            };
            if urls.is_empty() {
                return Err(bad("Succeeded prediction has no media outputs."));
            }
            for value in urls {
                let url = value
                    .as_str()
                    .ok_or_else(|| bad("Media output is not a URL."))?;
                reqwest::Url::parse(url).map_err(|_| bad("Invalid media output URL."))?;
                assets.push(MediaAsset {
                    url: url.into(),
                    mime_type: None,
                    media_type: match self.kind {
                        MediaKind::Image => MediaRenderType::Image,
                        MediaKind::Video => MediaRenderType::Video,
                        MediaKind::Audio => MediaRenderType::Audio,
                    },
                    width: None,
                    height: None,
                    duration_seconds: None,
                    provider_asset_id: Some(id.clone()),
                });
            }
        }
        Ok(RemoteUpdate {
            id,
            status: if removed && status == MediaJobStatus::Succeeded {
                MediaJobStatus::Expired
            } else {
                status
            },
            assets,
            expires_at,
            expiry_is_estimate: true,
            failure: raw["error"].as_str().map(str::to_owned),
            progress: None,
            raw,
        })
    }
}
#[async_trait]
impl MediaGenerationAdapter for Replicate {
    fn backend(&self) -> &MediaBackendConfig {
        &self.backend
    }
    fn prepare(
        &self,
        request: MediaRenderRequest,
    ) -> Result<PreparedMediaRequest, EngineAdapterRequestError> {
        super::prepare_media_request(&self.backend, request)
    }
    async fn submit(
        &self,
        prepared: PreparedMediaRequest,
        cancellation: CancellationToken,
        reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
        _batches: Option<Arc<dyn MediaGenerationBatchReporter>>,
    ) -> Result<MediaSubmission, EngineAdapterRequestError> {
        let timing = MediaTimingRecorder::start();
        let work = async {
            let response = self
                .request(reqwest::Method::POST, &prepared.path)
                .json(&prepared.body)
                .send()
                .await
                .map_err(|error| {
                    EngineAdapterRequestError::transport("replicate", error.to_string())
                        .with_dispatch(if error.is_connect() {
                            DispatchState::NotSent
                        } else {
                            DispatchState::Unknown
                        })
                })?;
            let raw = json_response("replicate", response).await?;
            let job = RemoteJob::new(
                Arc::new(self.clone()),
                raw,
                prepared.body,
                reporter,
                prepared.render_input.options.count.unwrap_or(1),
                timing,
            )?;
            job.announce(cancellation.clone()).await?;
            Ok(MediaSubmission::Pending(Box::new(job)))
        };
        tokio::select! { biased; _ = cancellation.cancelled() => Err(EngineAdapterRequestError::new("replicate", "Media submission cancelled.", ErrorKind::Cancelled)), result = work => result }
    }
    async fn cancel(&self, id: &str) -> Result<(), EngineAdapterRequestError> {
        validate_id("replicate", id)?;
        match JobTransport::cancel(self, id).await? {
            MediaJobStatus::Cancelled => Ok(()),
            _ => Err(EngineAdapterRequestError::new(
                "replicate",
                "Provider has not confirmed cancellation.",
                ErrorKind::Provider,
            )
            .with_request_id(Some(id.into()))
            .with_dispatch(DispatchState::Accepted)),
        }
    }
}
#[async_trait]
impl JobTransport for Replicate {
    fn provider(&self) -> &str {
        "replicate"
    }
    fn backend(&self) -> &MediaBackendConfig {
        &self.backend
    }
    fn decode(
        &self,
        raw: Value,
        _submission: bool,
    ) -> Result<RemoteUpdate, EngineAdapterRequestError> {
        self.decode(raw)
    }
    async fn poll(&self, id: &str) -> Result<RemoteUpdate, EngineAdapterRequestError> {
        let response = self
            .request(reqwest::Method::GET, &format!("predictions/{id}"))
            .send()
            .await
            .map_err(|e| EngineAdapterRequestError::transport("replicate", e.to_string()))?;
        self.decode(json_response("replicate", response).await?)
    }
    async fn cancel(&self, id: &str) -> Result<MediaJobStatus, EngineAdapterRequestError> {
        let response = self
            .request(reqwest::Method::POST, &format!("predictions/{id}/cancel"))
            .send()
            .await
            .map_err(|e| EngineAdapterRequestError::transport("replicate", e.to_string()))?;
        let update = self.decode(json_response("replicate", response).await?)?;
        if update.id != id {
            return Err(EngineAdapterRequestError::invalid_response(
                "replicate",
                "Cancellation returned a different prediction.",
            ));
        }
        Ok(update.status)
    }
    fn asset_auth(&self, url: &reqwest::Url) -> Option<&str> {
        let host = url.host_str().unwrap_or("");
        if url.origin() == self.base.origin()
            || (url.scheme() == "https"
                && (host == "replicate.delivery" || host.ends_with(".replicate.delivery")))
        {
            self.backend.auth.api_key.as_deref()
        } else {
            None
        }
    }
    fn webhook(
        &self,
        webhook: MediaWebhook<'_>,
        secret: &str,
        now: DateTime<Utc>,
    ) -> Result<RemoteUpdate, EngineAdapterRequestError> {
        verify_webhook(&webhook, secret, now)?;
        self.decode(
            serde_json::from_slice(webhook.body).map_err(|e| {
                EngineAdapterRequestError::invalid_response("replicate", e.to_string())
            })?,
        )
    }
}
fn verify_webhook(
    webhook: &MediaWebhook<'_>,
    secret: &str,
    now: DateTime<Utc>,
) -> Result<(), EngineAdapterRequestError> {
    let fail = || {
        EngineAdapterRequestError::new(
            "replicate",
            "Invalid or stale webhook signature.",
            ErrorKind::Authentication,
        )
    };
    if webhook.body.len() > battersea_model::adapter::payload::PROVIDER_PAYLOAD_BYTES
        || webhook.id.is_empty()
        || webhook.id.len() > 256
        || webhook.signature.len() > 4096
    {
        return Err(fail());
    }
    let timestamp: i64 = webhook.timestamp.parse().map_err(|_| fail())?;
    if now.timestamp().abs_diff(timestamp) > 300 {
        return Err(fail());
    }
    let key = base64::engine::general_purpose::STANDARD
        .decode(secret.strip_prefix("whsec_").ok_or_else(fail)?)
        .map_err(|_| fail())?;
    if key.is_empty() {
        return Err(fail());
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| fail())?;
    mac.update(webhook.id.as_bytes());
    mac.update(b".");
    mac.update(webhook.timestamp.as_bytes());
    mac.update(b".");
    mac.update(webhook.body);
    for signature in webhook
        .signature
        .split_whitespace()
        .filter_map(|s| s.strip_prefix("v1,"))
    {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(signature) {
            if mac.clone().verify_slice(&bytes).is_ok() {
                return Ok(());
            }
        }
    }
    Err(fail())
}
