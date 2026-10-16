//! Bind admission, continuation, observations and remote job lifetimes to an accepted service.
use async_trait::async_trait;
use battersea_model::media::*;
use battersea_model::*;
use futures_util::StreamExt;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Context {
    binding: ProviderBinding,
    secret: Option<SecretString>,
}
impl Context {
    fn error(&self, error: EngineAdapterRequestError) -> EngineAdapterRequestError {
        self.binding.error(error, self.secret.as_deref())
    }
    fn invalid(&self, message: &str) -> EngineAdapterRequestError {
        EngineAdapterRequestError::new(&self.binding.provider, message, ErrorKind::InvalidRequest)
    }
    fn admit(
        &self,
        request: &mut EngineTextStreamRequest,
    ) -> Result<(), EngineAdapterRequestError> {
        for message in &mut request.shared.messages {
            for block in &mut message.content {
                *block = match block.clone() {
                    ContentBlock::NativeV2 { origin, payload }
                        if origin == self.binding.origin() =>
                    {
                        ContentBlock::Native {
                            provider: origin.provider,
                            payload,
                        }
                    }
                    ContentBlock::ReasoningV2 {
                        origin,
                        text,
                        signature,
                    } if origin == self.binding.origin() => ContentBlock::Reasoning {
                        provider: origin.provider,
                        text,
                        signature,
                    },
                    ContentBlock::NativeV2 { .. }
                    | ContentBlock::ReasoningV2 { .. }
                    | ContentBlock::Native { .. }
                    | ContentBlock::Reasoning { .. } => return Err(self.invalid(
                        "Continuation belongs to another binding or requires an explicit upgrade.",
                    )),
                    block => block,
                };
            }
        }
        Ok(())
    }
    fn event(
        &self,
        event: EngineTextStreamEvent,
    ) -> Result<EngineTextStreamEvent, EngineAdapterRequestError> {
        Ok(match event {
            EngineTextStreamEvent::ContentBlock { block } => EngineTextStreamEvent::ContentBlock {
                block: match block {
                    ContentBlock::Native { payload, .. } => ContentBlock::NativeV2 {
                        origin: self.binding.origin(),
                        payload,
                    },
                    ContentBlock::Reasoning {
                        text, signature, ..
                    } => ContentBlock::ReasoningV2 {
                        origin: self.binding.origin(),
                        text,
                        signature,
                    },
                    ContentBlock::NativeV2 { ref origin, .. }
                    | ContentBlock::ReasoningV2 { ref origin, .. }
                        if origin != &self.binding.origin() =>
                    {
                        return Err(self.invalid("Adapter emitted content for another binding."))
                    }
                    block => block,
                },
            },
            event => event,
        })
    }
    fn debug(&self, context: &mut EngineAdapterDebugContext) {
        context.provider = self.binding.provider.clone();
        context.base_url = (!self.binding.connection.base_url.is_empty())
            .then(|| self.binding.connection.base_url.clone());
    }
    fn redact(&self, value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(s) => {
                if let Some(secret) = self.secret.as_deref().filter(|s| !s.is_empty()) {
                    *s = SecretString::from(secret).redact(s);
                }
            }
            serde_json::Value::Array(items) => {
                for v in items {
                    self.redact(v);
                }
            }
            serde_json::Value::Object(map) => {
                for v in map.values_mut() {
                    self.redact(v);
                }
            }
            _ => (),
        }
    }
}
fn context(backend: &EngineBackendConfig) -> Context {
    Context {
        binding: backend.binding.clone().expect("configured binding"),
        secret: backend.auth.api_key.clone(),
    }
}
pub(super) fn chat(
    backend: EngineBackendConfig,
    inner: Arc<dyn EngineAdapter>,
    counting: TokenCounting,
) -> Arc<dyn EngineAdapter> {
    Arc::new(Chat {
        context: context(&backend),
        inner,
        counting,
    })
}
struct Chat {
    context: Context,
    inner: Arc<dyn EngineAdapter>,
    counting: TokenCounting,
}
#[async_trait]
impl EngineAdapter for Chat {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        let mut result = self.inner.describe_debug_context();
        self.context.debug(&mut result);
        result
    }
    async fn count_text_stream_input_tokens(
        &self,
        mut request: EngineTextStreamRequest,
    ) -> Result<u64, EngineAdapterRequestError> {
        if self.counting == TokenCounting::Unavailable {
            return Err(self
                .context
                .invalid("This binding does not support input token counting."));
        }
        self.context.admit(&mut request)?;
        self.inner
            .count_text_stream_input_tokens(request)
            .await
            .map_err(|e| self.context.error(e))
    }
    async fn stream_text(
        &self,
        mut request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        self.context.admit(&mut request)?;
        let stream = self
            .inner
            .stream_text(request)
            .await
            .map_err(|e| self.context.error(e))?;
        let context = self.context.clone();
        Ok(Box::pin(stream.map(move |event| {
            event
                .and_then(|event| context.event(event))
                .map_err(|error| context.error(error))
        })))
    }
}
struct Logger {
    context: Context,
    inner: Arc<dyn EngineAdapterLogger>,
}
pub(super) fn logger(
    backend: &EngineBackendConfig,
    inner: Arc<dyn EngineAdapterLogger>,
) -> Arc<dyn EngineAdapterLogger> {
    Arc::new(Logger {
        context: context(backend),
        inner,
    })
}
#[async_trait]
impl EngineAdapterLogger for Logger {
    async fn on_request(&self, mut entry: EngineAdapterRequestLog) {
        self.context.debug(&mut entry.context);
        let mut detail = serde_json::to_value(&entry.detail).expect("log");
        self.context.redact(&mut detail);
        entry.detail = serde_json::from_value(detail).expect("redacted log");
        self.inner.on_request(entry).await;
    }
    async fn on_response(&self, mut entry: EngineAdapterResponseLog) {
        self.context.debug(&mut entry.context);
        let mut detail = serde_json::to_value(&entry.detail).expect("log");
        self.context.redact(&mut detail);
        entry.detail = serde_json::from_value(detail).expect("redacted log");
        self.inner.on_response(entry).await;
    }
    async fn on_stream_event(&self, mut entry: EngineAdapterStreamLog) {
        self.context.debug(&mut entry.context);
        self.context.redact(&mut entry.detail);
        self.inner.on_stream_event(entry).await;
    }
    async fn on_error(&self, mut entry: EngineAdapterErrorLog) {
        self.context.debug(&mut entry.context);
        let mut detail = serde_json::to_value(&entry.detail).expect("log");
        self.context.redact(&mut detail);
        entry.detail = serde_json::from_value(detail).expect("redacted log");
        self.inner.on_error(entry).await;
    }
    async fn on_retry(&self, mut entry: EngineAdapterRetryLog) {
        entry.provider = self.context.binding.provider.clone();
        if let Some(id) = &mut entry.request_id {
            let mut v = serde_json::Value::String(id.clone());
            self.context.redact(&mut v);
            *id = v.as_str().unwrap().into();
        }
        self.inner.on_retry(entry).await;
    }
}
pub(super) fn media(
    backend: MediaBackendConfig,
    inner: Arc<dyn MediaGenerationAdapter>,
    provider: MediaProvider,
) -> Arc<dyn MediaGenerationAdapter> {
    let context = Context {
        binding: backend.binding.clone().unwrap(),
        secret: backend.auth.api_key.clone(),
    };
    let mut registry = ProviderRegistry::default();
    registry
        .register_media(&backend.provider, provider)
        .expect("unique provider");
    let revision = configuration_digest(&serde_json::to_value(&backend).expect("safe backend"));
    Arc::new(Media {
        revision,
        backend,
        inner,
        context,
        registry,
    })
}
struct Media {
    revision: String,
    backend: MediaBackendConfig,
    inner: Arc<dyn MediaGenerationAdapter>,
    context: Context,
    registry: ProviderRegistry,
}
#[async_trait]
impl MediaGenerationAdapter for Media {
    fn backend(&self) -> &MediaBackendConfig {
        &self.backend
    }
    fn prepare(
        &self,
        request: MediaRenderRequest,
    ) -> Result<PreparedMediaRequest, EngineAdapterRequestError> {
        let mut result = self
            .registry
            .prepare_media(&self.backend, request)
            .map_err(|e| self.context.error(e))?;
        result.binding_revision = Some(self.revision.clone());
        Ok(result)
    }
    async fn submit(
        &self,
        prepared: PreparedMediaRequest,
        cancellation: CancellationToken,
        activity: Option<Arc<dyn MediaGenerationActivityReporter>>,
        batch: Option<Arc<dyn MediaGenerationBatchReporter>>,
    ) -> Result<MediaSubmission, EngineAdapterRequestError> {
        if prepared.binding_revision.as_deref() != Some(&self.revision) {
            return Err(self
                .context
                .invalid("Prepared media belongs to another binding."));
        }
        Ok(
            match self
                .inner
                .submit(prepared, cancellation, activity, batch)
                .await
                .map_err(|e| self.context.error(e))?
            {
                MediaSubmission::Pending(inner) => MediaSubmission::Pending(Box::new(Job {
                    inner,
                    context: self.context.clone(),
                })),
                complete => complete,
            },
        )
    }
    async fn cancel(&self, id: &str) -> Result<(), EngineAdapterRequestError> {
        self.inner
            .cancel(id)
            .await
            .map_err(|e| self.context.error(e))
    }
}
struct Job {
    inner: Box<dyn MediaJob>,
    context: Context,
}
#[async_trait]
impl MediaJob for Job {
    fn provider(&self) -> &str {
        &self.context.binding.provider
    }
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn snapshot(&self) -> MediaJobSnapshot {
        self.inner.snapshot()
    }
    fn poll_policy(&self) -> MediaPollPolicy {
        self.inner.poll_policy()
    }
    async fn wait(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        self.inner
            .wait(cancellation)
            .await
            .map_err(|e| self.context.error(e))
    }
    async fn poll(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<MediaJobStatus, EngineAdapterRequestError> {
        self.inner
            .poll(cancellation)
            .await
            .map_err(|e| self.context.error(e))
    }
    async fn retrieve(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        self.inner
            .retrieve(cancellation)
            .await
            .map_err(|e| self.context.error(e))
    }
    async fn cancel(&mut self) -> Result<(), EngineAdapterRequestError> {
        self.inner.cancel().await.map_err(|e| self.context.error(e))
    }
    async fn webhook(
        &mut self,
        webhook: MediaWebhook<'_>,
        secret: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<MediaJobStatus, EngineAdapterRequestError> {
        self.inner
            .webhook(webhook, secret, now)
            .await
            .map_err(|e| self.context.error(e))
    }
}
