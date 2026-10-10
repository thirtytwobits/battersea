//! Explicit provider and tool registration. Construction and admission perform no provider I/O.
use crate::{adapter::*, media::*, EngineAdapterRequestError, ErrorKind};
use async_trait::async_trait;
use std::{collections::BTreeMap, sync::Arc};

type ChatFactory = dyn Fn(
        EngineBackendConfig,
        Option<Arc<dyn EngineAdapterLogger>>,
    ) -> Result<Arc<dyn EngineAdapter>, EngineAdapterRequestError>
    + Send
    + Sync;
type MediaFactory = dyn Fn(MediaBackendConfig) -> Result<Arc<dyn MediaGenerationAdapter>, EngineAdapterRequestError>
    + Send
    + Sync;
type MediaPrepare = dyn Fn(
        &MediaBackendConfig,
        MediaRenderRequest,
    ) -> Result<PreparedMediaRequest, EngineAdapterRequestError>
    + Send
    + Sync;

pub struct ChatProvider {
    pub content: crate::ContentCapabilities,
    pub factory: Arc<ChatFactory>,
    pub parameters: Vec<String>,
    pub background_modes: Vec<String>,
}
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
fn invalid(provider: &str, message: impl Into<String>) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new(provider, message, ErrorKind::InvalidRequest)
}
impl ProviderRegistry {
    pub fn register_chat(
        &mut self,
        id: impl Into<String>,
        provider: ChatProvider,
    ) -> Result<(), EngineAdapterRequestError> {
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
    ) -> Result<(), EngineAdapterRequestError> {
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
    pub fn chat_provider(&self, id: &str) -> Result<&ChatProvider, EngineAdapterRequestError> {
        self.chat
            .get(id)
            .ok_or_else(|| invalid(id, "Chat provider is not registered."))
    }
    pub fn media_provider(&self, id: &str) -> Result<&MediaProvider, EngineAdapterRequestError> {
        self.media
            .get(id)
            .ok_or_else(|| invalid(id, "Media provider is not registered."))
    }
    pub fn validate_chat_backend(
        &self,
        backend: &EngineBackendConfig,
    ) -> Result<(), EngineAdapterRequestError> {
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
    pub fn create_chat(
        &self,
        backend: EngineBackendConfig,
        logger: Option<Arc<dyn EngineAdapterLogger>>,
    ) -> Result<Arc<dyn EngineAdapter>, EngineAdapterRequestError> {
        self.validate_chat_backend(&backend)?;
        let inner = (self.chat_provider(&backend.provider)?.factory)(backend.clone(), logger)?;
        Ok(Arc::new(AdmittedAdapter { backend, inner }))
    }
    pub fn validate_media_backend(
        &self,
        backend: &MediaBackendConfig,
    ) -> Result<(), EngineAdapterRequestError> {
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
    pub fn create_media(
        &self,
        backend: MediaBackendConfig,
    ) -> Result<Arc<dyn MediaGenerationAdapter>, EngineAdapterRequestError> {
        self.validate_media_backend(&backend)?;
        (self.media_provider(&backend.provider)?.factory)(backend)
    }
    pub fn prepare_media(
        &self,
        backend: &MediaBackendConfig,
        request: MediaRenderRequest,
    ) -> Result<PreparedMediaRequest, EngineAdapterRequestError> {
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

struct RegisteredTool {
    definition: EngineLocalToolDefinition,
    validator: jsonschema::Validator,
    executor: Arc<dyn EngineLocalToolExecutor>,
}
#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, RegisteredTool>,
}
impl ToolRegistry {
    pub fn register(
        &mut self,
        mut definition: EngineLocalToolDefinition,
        executor: Arc<dyn EngineLocalToolExecutor>,
    ) -> Result<(), EngineAdapterRequestError> {
        definition.name = local_tool_name(&definition.name);
        if definition.name.trim().is_empty() || self.tools.contains_key(&definition.name) {
            return Err(invalid(
                "tools",
                "Duplicate, blank or colliding tool identifier.",
            ));
        }
        let validator = jsonschema::validator_for(&definition.input_schema)
            .map_err(|e| invalid("tools", e.to_string()))?;
        self.tools.insert(
            definition.name.clone(),
            RegisteredTool {
                definition,
                validator,
                executor,
            },
        );
        Ok(())
    }
    pub fn definitions(&self) -> Vec<EngineLocalToolDefinition> {
        self.tools.values().map(|t| t.definition.clone()).collect()
    }
}
#[async_trait]
impl EngineLocalToolExecutor for ToolRegistry {
    async fn call(
        &self,
        mut call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        call.name = local_tool_name(&call.name);
        let tool = self
            .tools
            .get(&call.name)
            .ok_or_else(|| invalid("tools", "Tool is not registered."))?;
        tool.validator
            .validate(&call.arguments)
            .map_err(|e| invalid("tools", e.to_string()))?;
        tool.executor.call(call).await
    }
}

struct AdmittedAdapter {
    backend: EngineBackendConfig,
    inner: Arc<dyn EngineAdapter>,
}
#[async_trait]
impl EngineAdapter for AdmittedAdapter {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        self.inner.describe_debug_context()
    }
    async fn count_text_stream_input_tokens(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<u64, EngineAdapterRequestError> {
        validate_chat_request(&self.backend, &request)?;
        let call = self.inner.count_text_stream_input_tokens(request);
        match self.backend.options.timeout_ms {
            Some(ms) => tokio::time::timeout(std::time::Duration::from_millis(ms), call)
                .await
                .map_err(|_| {
                    EngineAdapterRequestError::new(
                        &self.backend.provider,
                        "Token counting deadline exceeded.",
                        ErrorKind::Timeout,
                    )
                })?,
            None => call.await,
        }
    }
    async fn stream_text(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        validate_chat_request(&self.backend, &request)?;
        let deadline = self
            .backend
            .options
            .timeout_ms
            .map(|ms| tokio::time::Instant::now() + std::time::Duration::from_millis(ms));
        let response_format = request.shared.chat.response_format.clone();
        let output_modalities = request
            .shared
            .chat
            .output_modalities
            .clone()
            .unwrap_or_else(|| vec![crate::Modality::Text]);
        let call = self.inner.stream_text(request);
        let stream = match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, call)
                .await
                .map_err(|_| {
                    EngineAdapterRequestError::new(
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
        Ok(Box::pin(futures_util::stream::unfold(
            Some((stream, deadline, provider)),
            |state| async move {
                use futures_util::StreamExt;
                let (mut stream, deadline, provider) = state?;
                let next = match deadline {
                    Some(deadline) => {
                        match tokio::time::timeout_at(deadline, stream.next()).await {
                            Ok(next) => next,
                            Err(_) => {
                                return Some((
                                    Err(EngineAdapterRequestError::new(
                                        provider,
                                        "Request deadline exceeded.",
                                        ErrorKind::Timeout,
                                    )),
                                    None,
                                ))
                            }
                        }
                    }
                    None => stream.next().await,
                };
                next.map(|event| (event, Some((stream, deadline, provider))))
            },
        )))
    }
}
