//! Explicit provider registration. Construction and admission perform no provider I/O.
use crate::{adapter::*, media::*, Clock, ErrorKind, Failure};
use async_trait::async_trait;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

type ChatFactory = dyn Fn(
        EngineBackendConfig,
        Option<Arc<dyn EngineAdapterLogger>>,
        Arc<dyn Clock>,
    ) -> Result<Arc<dyn EngineAdapter>, Failure>
    + Send
    + Sync;
type MediaFactory = dyn Fn(MediaBackendConfig, Arc<dyn Clock>) -> Result<Arc<dyn MediaGenerationAdapter>, Failure>
    + Send
    + Sync;
type MediaPrepare = dyn Fn(&MediaBackendConfig, MediaRenderRequest) -> Result<PreparedMediaRequest, Failure>
    + Send
    + Sync;

#[derive(Clone)]
pub struct ChatProvider {
    pub content: crate::ContentCapabilities,
    pub factory: Arc<ChatFactory>,
    pub parameters: Vec<String>,
    pub background_modes: Vec<String>,
}
#[derive(Clone)]
pub struct MediaProvider {
    pub factory: Arc<MediaFactory>,
    pub prepare: Arc<MediaPrepare>,
    pub parameters: Vec<String>,
    pub capabilities: MediaProviderCapabilities,
    pub accepts_aspect_ratio: fn(&str, &str) -> bool,
    pub accepts_size: fn(&str, &str) -> bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaProviderCapabilities {
    pub kinds: Vec<MediaKind>,
    pub references: bool,
    pub negative_prompt: bool,
    pub partial_images: bool,
    pub remote_jobs: bool,
    pub remote_cancellation: bool,
    pub webhooks: bool,
}

#[derive(Default)]
pub struct ProviderRegistry {
    chat: BTreeMap<String, ChatProvider>,
    media: BTreeMap<String, MediaProvider>,
}
fn invalid(provider: &str, message: impl Into<String>) -> Failure {
    Failure::new(provider, message, ErrorKind::InvalidRequest)
}
impl ProviderRegistry {
    pub fn register_chat(
        &mut self,
        id: impl Into<String>,
        provider: ChatProvider,
    ) -> Result<(), Failure> {
        let id = id.into();
        if id.trim().is_empty() || self.chat.contains_key(&id) {
            return Err(invalid(&id, "Duplicate or blank chat provider identifier."));
        }
        self.chat.insert(id, provider);
        Ok(())
    }
    pub fn register_media(
        &mut self,
        id: impl Into<String>,
        provider: MediaProvider,
    ) -> Result<(), Failure> {
        let id = id.into();
        if id.trim().is_empty() || self.media.contains_key(&id) {
            return Err(invalid(
                &id,
                "Duplicate or blank media provider identifier.",
            ));
        }
        self.media.insert(id, provider);
        Ok(())
    }
    /// The identifier of every registered chat provider, in order.
    pub fn chat_provider_ids(&self) -> impl Iterator<Item = &str> {
        self.chat.keys().map(String::as_str)
    }
    pub fn chat_provider(&self, id: &str) -> Result<&ChatProvider, Failure> {
        self.chat
            .get(id)
            .ok_or_else(|| invalid(id, "Chat provider is not registered."))
    }
    pub fn media_provider(&self, id: &str) -> Result<&MediaProvider, Failure> {
        self.media
            .get(id)
            .ok_or_else(|| invalid(id, "Media provider is not registered."))
    }
    pub fn validate_chat_backend(&self, backend: &EngineBackendConfig) -> Result<(), Failure> {
        let provider = self.chat_provider(&backend.provider)?;
        backend
            .capabilities
            .content
            .validate_subset(&provider.content)
            .map_err(|message| invalid(&backend.provider, message))?;
        if backend.options.background == Some(true)
            && !backend
                .options
                .background_mode
                .as_ref()
                .is_some_and(|mode| provider.background_modes.contains(mode))
        {
            return Err(invalid(
                &backend.provider,
                "Provider does not implement the configured background mode.",
            ));
        }
        if !backend.enabled || backend.id.trim().is_empty() || backend.model.trim().is_empty() {
            return Err(invalid(
                &backend.provider,
                "Backend must be enabled with an explicit identity and model.",
            ));
        }
        if backend.options.timeout_ms == Some(0)
            || backend.options.stream_idle_timeout_ms == Some(0)
        {
            return Err(invalid(
                &backend.provider,
                "Request deadlines and stream watchdogs must be positive.",
            ));
        }
        for parameter in &backend.capabilities.supported_chat_parameters {
            if !provider.parameters.contains(parameter) {
                return Err(invalid(
                    &backend.provider,
                    format!("Provider does not implement advertised parameter {parameter}."),
                ));
            }
        }
        if backend
            .capabilities
            .supported_tool_execution_modes
            .iter()
            .any(|mode| mode != "engine-orchestrated")
        {
            return Err(invalid(
                &backend.provider,
                "Provider does not implement the configured tool execution mode.",
            ));
        }
        backend
            .capabilities
            .validate_chat_parameters(&backend.chat)
            .map_err(|message| invalid(&backend.provider, message))?;
        if backend
            .capabilities
            .supported_chat_parameters
            .iter()
            .any(|name| name == "reasoningEffort")
        {
            for effort in &backend.capabilities.supported_reasoning_efforts {
                backend
                    .capabilities
                    .validate_reasoning_effort(Some(*effort))
                    .map_err(|message| invalid(&backend.provider, message))?;
            }
        }
        Ok(())
    }
    /// `clock` drives the adapter's deadlines, backoff, polling and idle watchdogs.
    pub fn create_chat(
        &self,
        backend: EngineBackendConfig,
        logger: Option<Arc<dyn EngineAdapterLogger>>,
        clock: Arc<dyn Clock>,
    ) -> Result<Arc<dyn EngineAdapter>, Failure> {
        self.validate_chat_backend(&backend)?;
        let inner = (self.chat_provider(&backend.provider)?.factory)(
            backend.clone(),
            logger.clone(),
            clock.clone(),
        )?;
        Ok(Arc::new(AdmittedAdapter {
            report: Arc::new(Report {
                context: inner.describe_debug_context(),
                backend,
                logger,
            }),
            inner,
            clock,
        }))
    }
    pub fn validate_media_backend(&self, backend: &MediaBackendConfig) -> Result<(), Failure> {
        let provider = self.media_provider(&backend.provider)?;
        let capabilities = &provider.capabilities;
        for parameter in &backend.capabilities.supported_media_parameters {
            if !provider.parameters.contains(parameter) {
                return Err(invalid(
                    &backend.provider,
                    format!("Provider does not implement advertised media parameter {parameter}."),
                ));
            }
        }
        let kind = match backend.capability {
            MediaCapability::ImageGeneration => MediaKind::Image,
            MediaCapability::VideoGeneration => MediaKind::Video,
            MediaCapability::AudioGeneration => MediaKind::Audio,
            MediaCapability::PromptPlanner => {
                return Err(invalid(&backend.provider, "Unsupported media capability."))
            }
        };
        if !backend.enabled
            || backend.id.trim().is_empty()
            || backend.model.trim().is_empty()
            || !capabilities.kinds.contains(&kind)
        {
            return Err(invalid(
                &backend.provider,
                "Backend is disabled, lacks a model or advertises an unsupported media kind.",
            ));
        }
        if backend.options.timeout_ms == Some(0)
            || backend.options.stream_idle_timeout_ms == Some(0)
        {
            return Err(invalid(
                &backend.provider,
                "Request deadlines and watchdogs must be positive.",
            ));
        }
        if backend.capabilities.supports_partial_image_streaming && !capabilities.partial_images {
            return Err(invalid(
                &backend.provider,
                "Provider does not implement partial image streaming.",
            ));
        }
        Ok(())
    }
    /// `clock` drives the adapter's deadlines, backoff and polling.
    pub fn create_media(
        &self,
        backend: MediaBackendConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Arc<dyn MediaGenerationAdapter>, Failure> {
        self.validate_media_backend(&backend)?;
        let inner = (self.media_provider(&backend.provider)?.factory)(backend.clone(), clock)?;
        Ok(Arc::new(AdmittedMedia {
            origin: Arc::new(MediaOrigin {
                provider: backend.provider,
                backend: backend.id,
                model: backend.model,
            }),
            inner,
        }))
    }
    pub fn prepare_media(
        &self,
        backend: &MediaBackendConfig,
        request: MediaRenderRequest,
    ) -> Result<PreparedMediaRequest, Failure> {
        self.validate_media_backend(backend)?;
        let provider = self.media_provider(&backend.provider)?;
        let configured_kind = match backend.capability {
            MediaCapability::ImageGeneration => MediaKind::Image,
            MediaCapability::AudioGeneration => MediaKind::Audio,
            _ => MediaKind::Video,
        };
        if request.kind != configured_kind
            || request.prompt_text.trim().is_empty()
            || (!request.references.is_empty() && !provider.capabilities.references)
            || (request.negative_prompt.is_some() && !provider.capabilities.negative_prompt)
        {
            return Err(invalid(
                &backend.provider,
                "Media request conflicts with provider or backend capabilities.",
            ));
        }
        for (name, present) in [
            ("count", request.options.count.is_some()),
            ("size", request.options.size.is_some()),
            ("aspect_ratio", request.options.aspect_ratio.is_some()),
            ("seed", request.options.seed.is_some()),
            (
                "duration_seconds",
                request.options.duration_seconds.is_some(),
            ),
        ] {
            if present
                && !backend
                    .capabilities
                    .supported_media_parameters
                    .iter()
                    .any(|p| p == name)
            {
                return Err(invalid(
                    &backend.provider,
                    format!("Backend does not support media parameter {name}."),
                ));
            }
        }
        if request.options.count == Some(0) || request.options.duration_seconds == Some(0) {
            return Err(invalid(
                &backend.provider,
                "Media count and duration must be positive.",
            ));
        }
        (provider.prepare)(backend, request)
    }
}

/// Names the operation a failure belongs to and passes the failure to the observer. Every
/// failure of an admitted adapter leaves through [`Report::failure`].
struct Report {
    backend: EngineBackendConfig,
    context: EngineAdapterDebugContext,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
}
impl Report {
    fn stamp(&self, operation: &str, failure: &mut Failure) {
        let origin = &mut failure.origin;
        origin.operation.get_or_insert_with(|| operation.into());
        origin.provider = Some(self.backend.provider.clone());
        origin.backend = Some(self.backend.id.clone());
        origin.model = Some(self.backend.model.clone());
    }
    async fn failure(&self, operation: &str, mut failure: Failure) -> Failure {
        self.stamp(operation, &mut failure);
        crate::adapter::emit_error(
            self.logger.as_ref(),
            &self.context,
            EngineOperation::FlowTextStream,
            &failure,
        )
        .await;
        failure
    }
    async fn result<T>(&self, operation: &str, result: Result<T, Failure>) -> Result<T, Failure> {
        match result {
            Ok(value) => Ok(value),
            Err(failure) => Err(self.failure(operation, failure).await),
        }
    }
    fn stamped<T>(&self, operation: &str, result: Result<T, Failure>) -> Result<T, Failure> {
        result.map_err(|mut failure| {
            self.stamp(operation, &mut failure);
            failure
        })
    }
}

struct AdmittedAdapter {
    report: Arc<Report>,
    inner: Arc<dyn EngineAdapter>,
    clock: Arc<dyn Clock>,
}
impl std::ops::Deref for AdmittedAdapter {
    type Target = Report;
    fn deref(&self) -> &Report {
        &self.report
    }
}
impl AdmittedAdapter {
    async fn validate_conversation_output(
        &self,
        request: &crate::ConversationRequest,
        turn: &mut crate::ConversationTurn,
    ) -> Result<(), Failure> {
        let format = request.shared.chat.response_format.clone();
        let modalities = request
            .shared
            .chat
            .output_modalities
            .clone()
            .unwrap_or_else(|| vec![crate::Modality::Text]);
        if turn.calls.is_empty() {
            use futures_util::StreamExt;
            let stream = Box::pin(futures_util::stream::iter(
                std::mem::take(&mut turn.events).into_iter().map(Ok),
            ));
            let mut validated = crate::adapter::output::validate_output(
                stream,
                self.backend.provider.clone(),
                format,
                modalities,
            );
            let mut events = Vec::new();
            while let Some(event) = validated.next().await {
                events.push(event?);
            }
            turn.events = events;
        }
        crate::adapter::payload::check_payload(turn, &self.backend.provider)?;
        Ok(())
    }
}

#[async_trait]
impl EngineAdapter for AdmittedAdapter {
    fn validate_conversation(
        &self,
        request: &crate::ConversationRequest,
        state: Option<&Value>,
    ) -> Result<crate::ConversationBinding, Failure> {
        self.stamped("validate_conversation", {
            validate_chat_request(&self.backend, &request.to_request())
                .and_then(|()| self.inner.validate_conversation(request, state))
        })
    }
    fn validate_conversation_turn(
        &self,
        request: &crate::ConversationRequest,
        previous: Option<&Value>,
        results: &[crate::ConversationToolResult],
        turn: &crate::ConversationTurn,
    ) -> Result<crate::ConversationBinding, Failure> {
        self.stamped("validate_conversation_turn", {
            validate_chat_request(&self.backend, &request.to_request()).and_then(|()| {
                self.inner
                    .validate_conversation_turn(request, previous, results, turn)
            })
        })
    }
    fn validate_conversation_step(
        &self,
        request: &crate::ConversationRequest,
        previous: Option<&Value>,
        results: &[crate::ConversationToolResult],
        command: &crate::ConversationCommand,
        step: &crate::ConversationStep,
    ) -> Result<crate::ConversationBinding, Failure> {
        self.stamped("validate_conversation_step", {
            validate_chat_request(&self.backend, &request.to_request()).and_then(|()| {
                self.inner
                    .validate_conversation_step(request, previous, results, command, step)
            })
        })
    }
    async fn conversation_step(
        &self,
        request: crate::ConversationRequest,
        previous: Option<Value>,
        results: Vec<crate::ConversationToolResult>,
        command: crate::ConversationCommand,
    ) -> Result<crate::ConversationStep, Failure> {
        let step = self.step(request, previous, results, command).await;
        self.result("conversation_step", step).await
    }
    async fn conversation_turn(
        &self,
        request: crate::ConversationRequest,
        state: Option<Value>,
        results: Vec<crate::ConversationToolResult>,
    ) -> Result<crate::ConversationTurn, Failure> {
        let turn = self.turn(request, state, results).await;
        self.result("conversation_turn", turn).await
    }

    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        self.inner.describe_debug_context()
    }
    async fn count_text_stream_input_tokens(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<u64, Failure> {
        let count = self.count(request).await;
        self.result("count_tokens", count).await
    }
    async fn stream_text(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, Failure> {
        let stream = self.stream(request).await;
        self.result(STREAM_TEXT, stream).await
    }
}

const STREAM_TEXT: &str = "stream_text";

impl AdmittedAdapter {
    async fn step(
        &self,
        request: crate::ConversationRequest,
        previous: Option<Value>,
        results: Vec<crate::ConversationToolResult>,
        command: crate::ConversationCommand,
    ) -> Result<crate::ConversationStep, Failure> {
        validate_chat_request(&self.backend, &request.to_request())?;
        self.inner
            .validate_conversation(&request, previous.as_ref())?;
        let call = self
            .inner
            .conversation_step(request.clone(), previous, results, command);
        let mut step = match self.backend.options.timeout_ms {
            Some(ms) => crate::clock::timeout(
                self.clock.as_ref(),
                std::time::Duration::from_millis(ms),
                call,
            )
            .await
            .map_err(|_| {
                Failure::new(
                    &self.backend.provider,
                    "Provider operation deadline exceeded.",
                    ErrorKind::Timeout,
                )
            })??,
            None => call.await?,
        };
        if let crate::ConversationStep::Completed { turn } = &mut step {
            self.validate_conversation_output(&request, turn).await?;
        }
        crate::adapter::payload::check_payload(&step, &self.backend.provider)?;
        Ok(step)
    }
    async fn turn(
        &self,
        request: crate::ConversationRequest,
        state: Option<Value>,
        results: Vec<crate::ConversationToolResult>,
    ) -> Result<crate::ConversationTurn, Failure> {
        validate_chat_request(&self.backend, &request.to_request())?;
        self.inner.validate_conversation(&request, state.as_ref())?;
        let request_for_output = request.clone();
        let call = self.inner.conversation_turn(request, state, results);
        let mut turn = match self.backend.options.timeout_ms {
            Some(ms) => crate::clock::timeout(
                self.clock.as_ref(),
                std::time::Duration::from_millis(ms),
                call,
            )
            .await
            .map_err(|_| {
                Failure::new(
                    &self.backend.provider,
                    "Provider turn deadline exceeded.",
                    ErrorKind::Timeout,
                )
            })??,
            None => call.await?,
        };
        self.validate_conversation_output(&request_for_output, &mut turn)
            .await?;
        Ok(turn)
    }

    async fn count(&self, request: EngineTextStreamRequest) -> Result<u64, Failure> {
        validate_chat_request(&self.backend, &request)?;
        let call = self.inner.count_text_stream_input_tokens(request);
        match self.backend.options.timeout_ms {
            Some(ms) => crate::clock::timeout(
                self.clock.as_ref(),
                std::time::Duration::from_millis(ms),
                call,
            )
            .await
            .map_err(|_| {
                Failure::new(
                    &self.backend.provider,
                    "Token counting deadline exceeded.",
                    ErrorKind::Timeout,
                )
            })?,
            None => call.await,
        }
    }
    async fn stream(&self, request: EngineTextStreamRequest) -> Result<EngineTextStream, Failure> {
        validate_chat_request(&self.backend, &request)?;
        let deadline = self.backend.options.timeout_ms.and_then(|ms| {
            crate::clock::deadline_after(self.clock.as_ref(), std::time::Duration::from_millis(ms))
        });
        let response_format = request.shared.chat.response_format.clone();
        let output_modalities = request
            .shared
            .chat
            .output_modalities
            .clone()
            .unwrap_or_else(|| vec![crate::Modality::Text]);
        let call = self.inner.stream_text(request);
        let stream = match deadline {
            Some(deadline) => crate::clock::timeout_at(self.clock.as_ref(), deadline, call)
                .await
                .map_err(|_| {
                    Failure::new(
                        &self.backend.provider,
                        "Request deadline exceeded.",
                        ErrorKind::Timeout,
                    )
                })??,
            None => call.await?,
        };
        let provider = self.backend.provider.clone();
        let stream = match self.backend.options.stream_idle_timeout_ms {
            Some(ms) => with_stream_idle_watchdog(
                stream,
                std::time::Duration::from_millis(ms),
                provider.clone(),
                self.clock.clone(),
            ),
            None => stream,
        };
        let validated = super::adapter::output::validate_output(
            stream,
            provider.clone(),
            response_format,
            output_modalities,
        );
        let stream = validated;
        struct Open {
            stream: EngineTextStream,
            deadline: Option<tokio::time::Instant>,
            tool_call: Option<crate::ToolCallEvidence>,
            clock: Arc<dyn Clock>,
            report: Arc<Report>,
            request_id: Option<String>,
            response_id: Option<String>,
        }
        Ok(Box::pin(futures_util::stream::unfold(
            Some(Open {
                stream,
                deadline,
                tool_call: None,
                clock: self.clock.clone(),
                report: self.report.clone(),
                request_id: None,
                response_id: None,
            }),
            |state| async move {
                use futures_util::StreamExt;
                let mut open = state?;
                loop {
                    let next = match open.deadline {
                        Some(deadline) => crate::clock::timeout_at(
                            open.clock.as_ref(),
                            deadline,
                            open.stream.next(),
                        )
                        .await
                        .unwrap_or_else(|_| {
                            let mut error = Failure::new(
                                &open.report.backend.provider,
                                "Request deadline exceeded.",
                                ErrorKind::Timeout,
                            );
                            error.tool_call = open.tool_call.clone().map(Box::new);
                            Some(Err(error))
                        }),
                        None => open.stream.next().await,
                    };
                    match next? {
                        Ok(EngineTextStreamEvent::ResponseOpened {
                            request_id,
                            response_id,
                        }) => {
                            open.request_id = request_id;
                            open.response_id = response_id;
                        }
                        Ok(event) => {
                            super::adapter::retain_tool_evidence(&mut open.tool_call, &event);
                            return Some((Ok(event), Some(open)));
                        }
                        Err(mut failure) => {
                            let exchange = &mut failure.exchange;
                            if exchange.request_id.is_none() {
                                exchange.request_id = open.request_id.take();
                            }
                            if exchange.response_id.is_none() {
                                exchange.response_id = open.response_id.take();
                            }
                            let failure = open.report.failure(STREAM_TEXT, failure).await;
                            return Some((Err(failure), None));
                        }
                    }
                }
            },
        )))
    }
}

/// Names the media operation a failure belongs to.
struct MediaOrigin {
    provider: String,
    backend: String,
    model: String,
}
impl MediaOrigin {
    fn stamp(&self, operation: &str, mut failure: Failure) -> Failure {
        let origin = &mut failure.origin;
        origin.operation.get_or_insert_with(|| operation.into());
        origin.provider = Some(self.provider.clone());
        origin.backend = Some(self.backend.clone());
        origin.model = Some(self.model.clone());
        failure
    }
}

struct AdmittedMedia {
    origin: Arc<MediaOrigin>,
    inner: Arc<dyn MediaGenerationAdapter>,
}
#[async_trait]
impl MediaGenerationAdapter for AdmittedMedia {
    fn backend(&self) -> &MediaBackendConfig {
        self.inner.backend()
    }
    fn clock(&self) -> Arc<dyn Clock> {
        self.inner.clock()
    }
    fn prepare(&self, request: MediaRenderRequest) -> Result<PreparedMediaRequest, Failure> {
        self.inner
            .prepare(request)
            .map_err(|failure| self.origin.stamp("media_prepare", failure))
    }
    async fn submit(
        &self,
        prepared: PreparedMediaRequest,
        cancellation: tokio_util::sync::CancellationToken,
        activity: Option<Arc<dyn crate::media::MediaGenerationActivityReporter>>,
        batch: Option<Arc<dyn crate::media::MediaGenerationBatchReporter>>,
    ) -> Result<crate::media::MediaSubmission, Failure> {
        match self
            .inner
            .submit(prepared, cancellation, activity, batch)
            .await
        {
            Ok(crate::media::MediaSubmission::Pending(inner)) => Ok(
                crate::media::MediaSubmission::Pending(Box::new(AdmittedJob {
                    origin: self.origin.clone(),
                    inner,
                })),
            ),
            Ok(complete) => Ok(complete),
            Err(failure) => Err(self.origin.stamp("media_submit", failure)),
        }
    }
    async fn cancel(&self, id: &str) -> Result<(), Failure> {
        self.inner
            .cancel(id)
            .await
            .map_err(|failure| self.origin.stamp("media_cancel", failure))
    }
}

struct AdmittedJob {
    origin: Arc<MediaOrigin>,
    inner: Box<dyn crate::media::MediaJob>,
}
#[async_trait]
impl crate::media::MediaJob for AdmittedJob {
    fn provider(&self) -> &str {
        self.inner.provider()
    }
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn clock(&self) -> Arc<dyn Clock> {
        self.inner.clock()
    }
    fn snapshot(&self) -> crate::media::MediaJobSnapshot {
        self.inner.snapshot()
    }
    fn poll_policy(&self) -> crate::media::MediaPollPolicy {
        self.inner.poll_policy()
    }
    async fn wait(
        &mut self,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<crate::media::MediaRenderResult, Failure> {
        self.inner
            .wait(cancellation)
            .await
            .map_err(|failure| self.origin.stamp("media_wait", failure))
    }
    async fn poll(
        &mut self,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<crate::media::MediaJobStatus, Failure> {
        self.inner
            .poll(cancellation)
            .await
            .map_err(|failure| self.origin.stamp("media_poll", failure))
    }
    async fn retrieve(
        &mut self,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<crate::media::MediaRenderResult, Failure> {
        self.inner
            .retrieve(cancellation)
            .await
            .map_err(|failure| self.origin.stamp("media_retrieve", failure))
    }
    async fn cancel(&mut self) -> Result<(), Failure> {
        self.inner
            .cancel()
            .await
            .map_err(|failure| self.origin.stamp("media_cancel", failure))
    }
    async fn webhook(
        &mut self,
        webhook: crate::media::MediaWebhook<'_>,
        secret: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<crate::media::MediaJobStatus, Failure> {
        self.inner
            .webhook(webhook, secret, now)
            .await
            .map_err(|failure| self.origin.stamp("media_webhook", failure))
    }
}
