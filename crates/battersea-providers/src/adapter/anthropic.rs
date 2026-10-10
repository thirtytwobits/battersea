use super::error::EngineAdapterRequestError;
use super::tool_loop::{
    run_tool_loop, send_event, ExecutedToolCall, ToolConversation, ToolEventSender,
};
use super::{
    emit_request, emit_response, emit_stream_event, missing_backend_option, EngineAdapter,
    EngineAdapterDebugContext, EngineAdapterLogger, EngineAdapterResponseDetail,
    EngineBackendConfig, EngineLocalToolCall, EngineTextStream, EngineTextStreamEvent,
    EngineTextStreamRequest, EngineTokenUsage,
};
use crate::http_payload::BoundedResponse as _;
use async_trait::async_trait;
use battersea_model::engine::{
    EngineChatToolChoiceMode, EngineReasoningEffort, EngineTemperatureDispatch,
    EngineThinkingVisibility,
};
use eventsource_stream::Eventsource;
use futures_util::{stream, StreamExt};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// Builds an Anthropic text adapter from backend config, deriving stable debug context fields and
/// validating any configured auth headers up front.
pub(crate) fn create_anthropic_adapter(
    backend: EngineBackendConfig,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
) -> Result<Arc<dyn EngineAdapter>, EngineAdapterRequestError> {
    let provider = "anthropic";
    let timeout_ms = backend
        .options
        .timeout_ms
        .ok_or_else(|| missing_backend_option(provider, "options.timeoutMs"))?;
    let max_retries = backend
        .options
        .max_retries
        .ok_or_else(|| missing_backend_option(provider, "options.maxRetries"))?;
    let stream_idle_timeout_ms = backend
        .options
        .stream_idle_timeout_ms
        .ok_or_else(|| missing_backend_option(provider, "options.streamIdleTimeoutMs"))?;
    let base_url = resolve_anthropic_base_url(&backend.endpoint);
    let model = backend.model.clone();
    let backend_id = backend.id.clone();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .default_headers(build_headers(&backend)?)
        .build()
        .map_err(|error| EngineAdapterRequestError::transport("anthropic", error.to_string()))?;
    Ok(Arc::new(AnthropicEngineAdapter {
        client,
        logger,
        backend,
        stream_idle_timeout_ms,
        context: EngineAdapterDebugContext {
            provider: "anthropic".to_string(),
            backend: backend_id,
            model,
            sdk: "reqwest".to_string(),
            base_url: Some(base_url),
            timeout_ms: Some(timeout_ms),
            max_retries: Some(max_retries),
        },
    }))
}

struct AnthropicEngineAdapter {
    client: reqwest::Client,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
    backend: EngineBackendConfig,
    stream_idle_timeout_ms: u64,
    context: EngineAdapterDebugContext,
}

#[async_trait]
impl EngineAdapter for AnthropicEngineAdapter {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        self.context.clone()
    }

    async fn stream_text(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        super::validate_chat_request(&self.backend, &request)?;
        if !request.local_tools.is_empty() {
            return self.stream_text_with_local_tools(request).await;
        }

        let body = build_stream_body(&self.backend, &request)?;
        battersea_model::adapter::payload::check_payload(&body, "anthropic")?;
        emit_request(
            self.logger.as_ref(),
            &self.context,
            &request.shared,
            None,
            Some(body.clone()),
        )
        .await;

        let response = self
            .client
            .post(self.backend.endpoint.clone())
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                EngineAdapterRequestError::transport("anthropic", error.to_string())
            })?;
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        if !response.status().is_success() {
            return Err(normalize_anthropic_http_error(
                status,
                request_id,
                response.bounded_text().await.ok(),
            ));
        }
        emit_response(
            self.logger.as_ref(),
            &self.context,
            request.shared.operation,
            status,
            EngineAdapterResponseDetail {
                ok: true,
                request_id: request_id.clone(),
                response_id: None,
                output_chars: None,
                usage: None,
                streaming: Some(true),
                stop_reason: None,
            },
        )
        .await;

        let logger = self.logger.clone();
        let context = self.context.clone();
        let operation = request.shared.operation;
        let stream = crate::http_payload::bounded_stream(response)
            .eventsource()
            .map(move |event| {
                let logger = logger.clone();
                let context = context.clone();
                async move {
                    match event {
                        Ok(event) => {
                            let parsed: AnthropicStreamEvent = serde_json::from_str(&event.data)
                                .map_err(|error| {
                                    EngineAdapterRequestError::invalid_response(
                                        "anthropic",
                                        error.to_string(),
                                    )
                                })?;
                            emit_stream_event(
                                logger.as_ref(),
                                &context,
                                operation,
                                if event.event.is_empty() {
                                    parsed.event_type()
                                } else {
                                    event.event.as_str()
                                },
                                serde_json::to_value(&parsed).unwrap_or_else(|_| json!({})),
                            )
                            .await;
                            let usage = parsed.token_usage();
                            match parsed {
                                AnthropicStreamEvent::ContentBlockDelta { delta, .. } => {
                                    let mut events = Vec::new();
                                    match delta.r#type.as_deref() {
                                        Some("text_delta") => {
                                            let text = delta.text.unwrap_or_default();
                                            if !text.is_empty() {
                                                events.push(EngineTextStreamEvent::TextDelta {
                                                    text,
                                                });
                                            }
                                        }
                                        Some("thinking_delta") => {
                                            // Extended-thinking content blocks carry the
                                            // delta in `thinking` rather than `text`.
                                            // Forward as the neutral ReasoningDelta event;
                                            // ChatAPI accumulates and routes to the
                                            // reasoning_stream port + controller output.
                                            let text = delta.thinking.unwrap_or_default();
                                            if !text.is_empty() {
                                                events.push(
                                                    EngineTextStreamEvent::ReasoningDelta { text },
                                                );
                                            }
                                        }
                                        // signature_delta and any other future delta types
                                        // are control frames we don't surface yet.
                                        _ => {}
                                    }
                                    if let Some(usage) = usage {
                                        events.push(EngineTextStreamEvent::TokenUsage { usage });
                                    }
                                    Ok(events)
                                }
                                AnthropicStreamEvent::Error { error } => {
                                    Err(EngineAdapterRequestError::transport(
                                        "anthropic",
                                        error.message.unwrap_or_else(|| {
                                            "Anthropic streaming transport failed.".to_string()
                                        }),
                                    ))
                                }
                                _ => Ok(usage
                                    .map(|usage| vec![EngineTextStreamEvent::TokenUsage { usage }])
                                    .unwrap_or_default()),
                            }
                        }
                        Err(error) => Err(EngineAdapterRequestError::transport(
                            "anthropic",
                            error.to_string(),
                        )),
                    }
                }
            })
            .buffer_unordered(1)
            .flat_map(|result| {
                let events = match result {
                    Ok(events) => events.into_iter().map(Ok).collect::<Vec<_>>(),
                    Err(error) => vec![Err(error)],
                };
                stream::iter(events)
            })
            .boxed();

        Ok(super::with_stream_idle_watchdog(
            Box::pin(stream),
            std::time::Duration::from_millis(self.stream_idle_timeout_ms),
            "anthropic",
        ))
    }

    async fn count_text_stream_input_tokens(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<u64, EngineAdapterRequestError> {
        super::validate_chat_request(&self.backend, &request)?;
        let body = build_input_token_count_body(&self.backend, &request);
        let response = self
            .client
            .post(format!(
                "{}/v1/messages/count_tokens",
                resolve_anthropic_base_url(&self.backend.endpoint)
            ))
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                EngineAdapterRequestError::transport("anthropic", error.to_string())
            })?;
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        if !response.status().is_success() {
            return Err(normalize_anthropic_http_error(
                status,
                request_id,
                response.bounded_text().await.ok(),
            ));
        }
        let payload: Value =
            read_anthropic_json(response, request_id.clone(), &self.context).await?;
        payload
            .get("input_tokens")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                EngineAdapterRequestError::invalid_response(
                    "anthropic",
                    "Anthropic token count response omitted input_tokens.",
                )
                .with_request_id(request_id)
            })
    }
}

impl AnthropicEngineAdapter {
    async fn stream_text_with_local_tools(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        let executor = request.local_tool_executor.clone().ok_or_else(|| {
            EngineAdapterRequestError::new(
                "anthropic",
                "Local engine tool executor is unavailable.",
                "request",
            )
        })?;

        let (tx, rx) =
            mpsc::channel::<Result<EngineTextStreamEvent, EngineAdapterRequestError>>(64);
        let client = self.client.clone();
        let backend = self.backend.clone();
        let logger = self.logger.clone();
        let context = self.context.clone();

        tokio::spawn(async move {
            let conversation = AnthropicToolConversation {
                client,
                backend,
                logger: logger.clone(),
                context: context.clone(),
                messages: super::text_messages(&request.shared),
            };
            run_tool_loop(conversation, request, executor, logger, context, tx).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

struct AnthropicToolConversation {
    client: reqwest::Client,
    backend: EngineBackendConfig,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
    context: EngineAdapterDebugContext,
    messages: Vec<Value>,
}

#[async_trait]
impl ToolConversation for AnthropicToolConversation {
    async fn next_turn(
        &mut self,
        request: &EngineTextStreamRequest,
        results: Vec<ExecutedToolCall>,
        events: &ToolEventSender,
    ) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
        if !results.is_empty() {
            self.messages.push(json!({"role": "user", "content": results.into_iter().map(|item| json!({
                "type": "tool_result", "tool_use_id": item.call.id, "content": item.result.content.to_string()
            })).collect::<Vec<_>>()}));
        }
        let body = build_local_tool_body(&self.backend, request, self.messages.clone())?;
        battersea_model::adapter::payload::check_payload(&body, "anthropic")?;
        emit_request(
            self.logger.as_ref(),
            &self.context,
            &request.shared,
            None,
            Some(body.clone()),
        )
        .await;
        let response = self
            .client
            .post(&self.backend.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                EngineAdapterRequestError::transport("anthropic", error.to_string())
            })?;
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        if !response.status().is_success() {
            return Err(normalize_anthropic_http_error(
                status,
                request_id,
                response.bounded_text().await.ok(),
            ));
        }
        let payload: AnthropicMessagesResponse =
            read_anthropic_json(response, request_id.clone(), &self.context).await?;
        if !matches!(
            payload.stop_reason.as_deref(),
            Some("end_turn" | "tool_use" | "stop_sequence")
        ) {
            return Err(EngineAdapterRequestError::invalid_response(
                "anthropic",
                format!(
                    "Anthropic turn did not complete: {:?}.",
                    payload.stop_reason
                ),
            ));
        }
        let calls = collect_anthropic_tool_calls(&payload.content)?;
        let text = payload
            .content
            .iter()
            .filter_map(anthropic_content_block_text)
            .collect::<Vec<_>>()
            .join("");
        let reasoning = payload
            .content
            .iter()
            .filter_map(anthropic_content_block_reasoning)
            .collect::<Vec<_>>()
            .join("");
        emit_response(
            self.logger.as_ref(),
            &self.context,
            request.shared.operation,
            status,
            EngineAdapterResponseDetail {
                ok: true,
                request_id,
                response_id: payload.id,
                output_chars: Some(text.chars().count()),
                usage: payload.usage.clone(),
                streaming: Some(false),
                stop_reason: payload.stop_reason,
            },
        )
        .await;
        if !reasoning.is_empty() {
            send_event(
                events,
                EngineTextStreamEvent::ReasoningDelta { text: reasoning },
            )
            .await?;
        }
        if !text.is_empty() {
            send_event(events, EngineTextStreamEvent::TextDelta { text }).await?;
        }
        if let Some(usage) = payload.usage.as_ref().and_then(parse_anthropic_usage) {
            send_event(events, EngineTextStreamEvent::TokenUsage { usage }).await?;
        }
        // Keep signed thinking and every provider content block intact.
        self.messages
            .push(json!({"role": "assistant", "content": payload.content}));
        Ok(calls)
    }
}

/// Builds the default Anthropic request headers, applying provider defaults when custom names or
/// version values are not configured.
fn build_headers(backend: &EngineBackendConfig) -> Result<HeaderMap, EngineAdapterRequestError> {
    let provider = "anthropic";
    let mut headers = HeaderMap::new();
    let header_name = backend
        .auth
        .header
        .as_deref()
        .ok_or_else(|| missing_backend_option(provider, "auth.header"))?;
    headers.insert(
        HeaderName::from_bytes(header_name.as_bytes()).map_err(|error| {
            EngineAdapterRequestError::transport("anthropic", error.to_string())
        })?,
        HeaderValue::from_str(backend.auth.api_key.as_deref().unwrap_or("")).map_err(|error| {
            EngineAdapterRequestError::transport("anthropic", error.to_string())
        })?,
    );
    let version_header = backend
        .auth
        .version_header
        .as_deref()
        .ok_or_else(|| missing_backend_option(provider, "auth.versionHeader"))?;
    let version = backend
        .auth
        .version
        .as_deref()
        .ok_or_else(|| missing_backend_option(provider, "auth.version"))?;
    headers.insert(
        HeaderName::from_bytes(version_header.as_bytes()).map_err(|error| {
            EngineAdapterRequestError::transport("anthropic", error.to_string())
        })?,
        HeaderValue::from_str(version).map_err(|error| {
            EngineAdapterRequestError::transport("anthropic", error.to_string())
        })?,
    );
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("accept", HeaderValue::from_static("application/json"));
    Ok(headers)
}

/// Builds the Anthropic streaming request body with streaming explicitly enabled.
fn build_stream_body(
    backend: &EngineBackendConfig,
    request: &EngineTextStreamRequest,
) -> Result<Value, EngineAdapterRequestError> {
    let mut body = json!({
        "model": backend.model,
        "max_tokens": require_anthropic_max_tokens(&request.shared.chat)?,
        "stream": true,
        "system": request.shared.text_for_role(battersea_model::Role::System),
        "messages": super::text_messages(&request.shared)
    });
    apply_anthropic_temperature(&mut body, backend, request.shared.chat.temperature);
    apply_anthropic_chat_parameters(&mut body, backend, &request.shared, true);
    Ok(body)
}

fn build_local_tool_body(
    backend: &EngineBackendConfig,
    request: &EngineTextStreamRequest,
    messages: Vec<Value>,
) -> Result<Value, EngineAdapterRequestError> {
    let mut shared = request.shared.clone();
    if let Some(tool_name) = shared.chat.tool_choice.tool_name.as_mut() {
        *tool_name = super::local_tool_name(tool_name);
    }
    let mut body = json!({
        "model": backend.model,
        "max_tokens": require_anthropic_max_tokens(&shared.chat)?,
        "system": shared.text_for_role(battersea_model::Role::System),
        "messages": messages,
        "tools": request.local_tools.iter().map(|tool| {
            json!({
                "name": super::local_tool_name(&tool.name),
                "description": tool.description,
                "input_schema": tool.input_schema,
                "strict": shared.chat.strict_tool_inputs.unwrap_or(false),
            })
        }).collect::<Vec<_>>(),
    });
    apply_anthropic_temperature(&mut body, backend, shared.chat.temperature);
    apply_anthropic_chat_parameters(&mut body, backend, &shared, true);
    Ok(body)
}

fn require_anthropic_max_tokens(
    chat: &battersea_model::engine::EngineChatParameters,
) -> Result<u32, EngineAdapterRequestError> {
    chat.max_output_tokens.ok_or_else(|| {
        EngineAdapterRequestError::new(
            "anthropic",
            "Anthropic requests require maxOutputTokens.",
            "request",
        )
    })
}

fn apply_anthropic_chat_parameters(
    body: &mut Value,
    backend: &EngineBackendConfig,
    request: &super::EngineAdapterRequest,
    include_tool_choice: bool,
) {
    let chat = &request.chat;
    if backend_supports_chat_parameter(backend, "topP") {
        if let Some(top_p) = chat.top_p {
            body["top_p"] = json!(top_p);
        }
    }
    if backend_supports_chat_parameter(backend, "topK") {
        if let Some(top_k) = chat.top_k {
            body["top_k"] = json!(top_k);
        }
    }
    if !chat.stop_sequences.is_empty() {
        body["stop_sequences"] = json!(chat.stop_sequences);
    }
    let has_tools = body
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| !tools.is_empty());
    if include_tool_choice && (has_tools || chat.tool_choice.mode != EngineChatToolChoiceMode::Auto)
    {
        apply_anthropic_tool_choice(
            body,
            chat.tool_choice.mode,
            chat.tool_choice.tool_name.as_deref(),
            chat.parallel_tool_calls,
        );
    }
    if backend_supports_chat_parameter(backend, "serviceTier") {
        if let Some(value) = chat.service_tier.as_deref() {
            body["service_tier"] = json!(value);
        }
    }
    let mut metadata = chat
        .request_metadata
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(value) = chat.safety_identifier.as_deref() {
        metadata.insert("user_id".to_string(), json!(value));
    }
    if !metadata.is_empty() {
        body["metadata"] = Value::Object(metadata);
    }
    let Some(thinking_capabilities) = backend.capabilities.anthropic_thinking.as_ref() else {
        return;
    };
    let manual_thinking_budget_tokens = chat
        .thinking_budget_tokens
        .filter(|_| thinking_capabilities.manual_budget_supported);
    if let Some(budget_tokens) = manual_thinking_budget_tokens {
        let mut thinking = serde_json::Map::from_iter([
            ("type".to_string(), json!("enabled")),
            ("budget_tokens".to_string(), json!(budget_tokens)),
        ]);
        apply_anthropic_thinking_display(&mut thinking, chat.thinking_visibility);
        body["thinking"] = Value::Object(thinking);
    } else if thinking_capabilities.mode == "adaptive" {
        if chat.reasoning_effort == Some(EngineReasoningEffort::None)
            && thinking_capabilities.disable_supported
        {
            body["thinking"] = json!({ "type": "disabled" });
        } else if let Some(effort) = chat.reasoning_effort {
            if effort != EngineReasoningEffort::None {
                let mut thinking =
                    serde_json::Map::from_iter([("type".to_string(), json!("adaptive"))]);
                apply_anthropic_thinking_display(&mut thinking, chat.thinking_visibility);
                body["thinking"] = Value::Object(thinking);
                apply_anthropic_output_effort(body, thinking_capabilities, effort);
            }
        } else if chat.thinking_visibility == Some(EngineThinkingVisibility::Omitted)
            && thinking_capabilities.disable_supported
        {
            body["thinking"] = json!({
                "type": "disabled",
            });
        }
    } else if let Some(effort) = chat.reasoning_effort {
        if effort == EngineReasoningEffort::None {
            if thinking_capabilities.disable_supported {
                body["thinking"] = json!({ "type": "disabled" });
            }
        } else if let Some(budget_tokens) = anthropic_extended_thinking_budget(
            thinking_capabilities,
            effort,
            chat.max_output_tokens,
        ) {
            let mut thinking = serde_json::Map::from_iter([
                ("type".to_string(), json!("enabled")),
                ("budget_tokens".to_string(), json!(budget_tokens)),
            ]);
            apply_anthropic_thinking_display(&mut thinking, chat.thinking_visibility);
            body["thinking"] = Value::Object(thinking);
        }
    } else if chat.thinking_visibility == Some(EngineThinkingVisibility::Omitted)
        && thinking_capabilities.disable_supported
    {
        body["thinking"] = json!({
            "type": "disabled",
        });
    }
}

fn backend_supports_chat_parameter(backend: &EngineBackendConfig, parameter: &str) -> bool {
    backend
        .capabilities
        .supported_chat_parameters
        .iter()
        .any(|candidate| candidate == parameter)
}

fn apply_anthropic_temperature(body: &mut Value, backend: &EngineBackendConfig, temperature: f64) {
    if matches!(
        backend.capabilities.temperature_dispatch,
        EngineTemperatureDispatch::Always
    ) {
        body["temperature"] = json!(temperature);
    }
}

fn apply_anthropic_thinking_display(
    thinking: &mut serde_json::Map<String, Value>,
    visibility: Option<EngineThinkingVisibility>,
) {
    match visibility {
        Some(EngineThinkingVisibility::Summarized) => {
            thinking.insert("display".to_string(), json!("summarized"));
        }
        Some(EngineThinkingVisibility::Omitted) => {
            thinking.insert("display".to_string(), json!("omitted"));
        }
        None => {}
    }
}

fn apply_anthropic_output_effort(
    body: &mut Value,
    capabilities: &battersea_model::engine::EngineAnthropicThinkingCapabilities,
    effort: EngineReasoningEffort,
) {
    let effort = match capabilities.output_effort.get(effort.as_str()) {
        Some(effort) => effort,
        None => return,
    };
    let mut output_config = body
        .get("output_config")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    output_config.insert("effort".to_string(), json!(effort));
    body["output_config"] = Value::Object(output_config);
}

fn anthropic_extended_thinking_budget(
    capabilities: &battersea_model::engine::EngineAnthropicThinkingCapabilities,
    effort: EngineReasoningEffort,
    max_output_tokens: Option<u32>,
) -> Option<u32> {
    let requested = *capabilities.budget_tokens_by_effort.get(effort.as_str())?;
    let Some(max_output_tokens) = max_output_tokens else {
        return Some(requested);
    };
    Some(requested.min(max_output_tokens.saturating_sub(1).max(1)))
}

async fn read_anthropic_json<T: DeserializeOwned>(
    response: reqwest::Response,
    request_id: Option<String>,
    context: &EngineAdapterDebugContext,
) -> Result<T, EngineAdapterRequestError> {
    let status = response.status().as_u16();
    let body = response.bounded_text().await.map_err(|error| {
        EngineAdapterRequestError::transport(
            "anthropic",
            format!(
                "Anthropic response body read failed for backend `{}` model `{}`{}: {}",
                context.backend,
                context.model,
                format_request_id_suffix(request_id.as_deref()),
                error
            ),
        )
        .with_status_code(status)
        .with_request_id(request_id.clone())
    })?;

    serde_json::from_str::<T>(&body)
        .map_err(|error| anthropic_decode_error(error, &body, request_id, context, status))
}

fn anthropic_decode_error(
    error: serde_json::Error,
    body: &str,
    request_id: Option<String>,
    context: &EngineAdapterDebugContext,
    status: u16,
) -> EngineAdapterRequestError {
    EngineAdapterRequestError::invalid_response(
        "anthropic",
        format!(
            "Anthropic response decode failed for backend `{}` model `{}`{}: {}. Body: {}",
            context.backend,
            context.model,
            format_request_id_suffix(request_id.as_deref()),
            error,
            summarise_response_body(body)
        ),
    )
    .with_status_code(status)
    .with_request_id(request_id)
}

fn format_request_id_suffix(request_id: Option<&str>) -> String {
    request_id
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!(" request `{}`", value.trim()))
        .unwrap_or_default()
}

fn summarise_response_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "empty body".to_string();
    }
    let preview = trimmed
        .chars()
        .take(300)
        .collect::<String>()
        .replace(char::is_control, " ");
    let suffix = if trimmed.chars().count() > 300 {
        "..."
    } else {
        ""
    };
    format!("{} bytes, preview `{preview}{suffix}`", body.len())
}

fn apply_anthropic_tool_choice(
    body: &mut Value,
    mode: EngineChatToolChoiceMode,
    tool_name: Option<&str>,
    parallel_tool_calls: Option<bool>,
) {
    let mut choice = match mode {
        EngineChatToolChoiceMode::Auto => json!({"type": "auto"}),
        EngineChatToolChoiceMode::None => json!({"type": "none"}),
        EngineChatToolChoiceMode::Required | EngineChatToolChoiceMode::Any => {
            json!({"type": "any"})
        }
        EngineChatToolChoiceMode::Tool => {
            let Some(tool_name) = tool_name else {
                return;
            };
            json!({"type": "tool", "name": tool_name})
        }
    };
    if let Some(parallel) = parallel_tool_calls {
        choice["disable_parallel_tool_use"] = json!(!parallel);
    }
    body["tool_choice"] = choice;
}

fn build_input_token_count_body(
    backend: &EngineBackendConfig,
    request: &EngineTextStreamRequest,
) -> Value {
    json!({
        "model": backend.model,
        "system": request.shared.text_for_role(battersea_model::Role::System),
        "messages": super::text_messages(&request.shared)
    })
}

/// Reduces an Anthropic endpoint URL to a reusable base URL by trimming provider-specific message
/// suffixes and clearing query and fragment data when parsing succeeds.
fn resolve_anthropic_base_url(endpoint: &str) -> String {
    if let Ok(mut url) = reqwest::Url::parse(endpoint) {
        let maybe_path = url
            .path()
            .strip_suffix("/v1/messages")
            .or_else(|| url.path().strip_suffix("/messages"))
            .or_else(|| url.path().strip_suffix("/v1"))
            .map(str::to_string);
        if let Some(path) = maybe_path {
            url.set_path(if path.is_empty() { "/" } else { &path });
        }
        url.set_query(None);
        url.set_fragment(None);
        return url.to_string().trim_end_matches('/').to_string();
    }
    endpoint
        .trim_end_matches("/v1/messages")
        .trim_end_matches("/messages")
        .trim_end_matches("/v1")
        .trim_end_matches('/')
        .to_string()
}

/// Normalises an Anthropic HTTP failure into the shared provider-neutral error shape.
///
/// Blank or malformed error bodies fall back to a generic provider message while preserving the
/// originating HTTP status and request id.
fn normalize_anthropic_http_error(
    status: u16,
    request_id: Option<String>,
    body: Option<String>,
) -> EngineAdapterRequestError {
    let message = body
        .and_then(|payload| {
            let trimmed = payload.trim();
            if trimmed.is_empty() {
                return None;
            }
            serde_json::from_str::<AnthropicErrorEnvelope>(&payload)
                .ok()
                .and_then(|value| value.error.message)
        })
        .unwrap_or_else(|| "Anthropic request failed.".to_string());
    EngineAdapterRequestError::new("anthropic", message, classify_status(status))
        .with_status_code(status)
        .with_request_id(request_id)
}

/// Classifies Anthropic HTTP status codes into the shared adapter error categories.
fn classify_status(status: u16) -> &'static str {
    if matches!(status, 401 | 403) {
        "auth"
    } else if status == 429 {
        "rate_limit"
    } else if status >= 500 {
        "server"
    } else {
        "request"
    }
}

#[derive(Debug, Deserialize)]
struct AnthropicMessagesResponse {
    #[serde(default)]
    id: Option<String>,
    // Content blocks are kept as raw `serde_json::Value`s rather than
    // deserialised into a typed enum. The engine-orchestrated tool loop
    // echoes the assistant turn back to Anthropic verbatim (thinking
    // blocks with their `signature`, tool_use blocks, and anything
    // else) to satisfy the extended-thinking + tool-use contract, and
    // Anthropic rejects a turn whose thinking blocks were modified. A
    // typed enum silently drops unmodelled fields on the round-trip AND
    // hard-fails deserialisation on any block `type` it doesn't know
    // (a new Anthropic block type, or `server_tool_use` /
    // `web_search_tool_result` when server tools are in play). Storing
    // raw values makes the round-trip byte-for-byte faithful and immune
    // to new block types; the accessors below classify blocks by their
    // `type` field instead of by variant.
    #[serde(default)]
    content: Vec<Value>,
    #[serde(default)]
    usage: Option<Value>,
    #[serde(default)]
    stop_reason: Option<String>,
}

/// Returns the `type` discriminator of an Anthropic content block.
fn anthropic_content_block_type(block: &Value) -> Option<&str> {
    block.get("type").and_then(Value::as_str)
}

/// Visible assistant text — the `text` of a `text` block.
fn anthropic_content_block_text(block: &Value) -> Option<&str> {
    if anthropic_content_block_type(block) == Some("text") {
        block.get("text").and_then(Value::as_str)
    } else {
        None
    }
}

/// Surfaced reasoning — the `thinking` of a `thinking` block.
fn anthropic_content_block_reasoning(block: &Value) -> Option<&str> {
    if anthropic_content_block_type(block) == Some("thinking") {
        block.get("thinking").and_then(Value::as_str)
    } else {
        None
    }
}

fn collect_anthropic_tool_calls(
    blocks: &[Value],
) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
    blocks
        .iter()
        .filter(|block| anthropic_content_block_type(block) == Some("tool_use"))
        .map(|block| {
            super::tool_loop::decode_tool_call(
                "anthropic",
                block.get("id"),
                block.get("name"),
                block.get("input").cloned(),
            )
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorEnvelope {
    #[serde(default)]
    error: AnthropicErrorBody,
}

#[derive(Debug, Default, Deserialize)]
struct AnthropicErrorBody {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicStreamEvent {
    ContentBlockDelta {
        #[serde(default)]
        delta: AnthropicTextDelta,
    },
    Error {
        error: AnthropicStreamErrorBody,
    },
    MessageDelta {
        #[serde(default)]
        usage: Option<Value>,
    },
    MessageStart {
        #[serde(default)]
        message: Option<AnthropicStreamMessage>,
    },
    MessageStop {},
    ContentBlockStart {},
    ContentBlockStop {},
    Ping {},
}

impl AnthropicStreamEvent {
    /// Returns the stable stream event label recorded for diagnostics.
    fn event_type(&self) -> &'static str {
        match self {
            Self::ContentBlockDelta { .. } => "content_block_delta",
            Self::Error { .. } => "error",
            Self::MessageDelta { .. } => "message_delta",
            Self::MessageStart { .. } => "message_start",
            Self::MessageStop { .. } => "message_stop",
            Self::ContentBlockStart { .. } => "content_block_start",
            Self::ContentBlockStop { .. } => "content_block_stop",
            Self::Ping { .. } => "ping",
        }
    }

    fn token_usage(&self) -> Option<EngineTokenUsage> {
        let value = match self {
            Self::MessageStart { message } => message.as_ref()?.usage.as_ref()?,
            Self::MessageDelta { usage } => usage.as_ref()?,
            _ => return None,
        };
        parse_anthropic_usage(value)
    }
}

#[derive(Debug, Default, Deserialize, serde::Serialize)]
struct AnthropicStreamMessage {
    #[serde(default)]
    usage: Option<Value>,
}

fn parse_anthropic_usage(value: &Value) -> Option<EngineTokenUsage> {
    let input = value.get("input_tokens").and_then(Value::as_u64);
    let cache_creation = value
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = value
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let input_tokens = input.map(|tokens| tokens + cache_creation + cache_read);
    let output_tokens = value.get("output_tokens").and_then(Value::as_u64);
    (input_tokens.is_some() || output_tokens.is_some()).then_some(EngineTokenUsage {
        input_tokens,
        output_tokens,
        total_tokens: input_tokens
            .zip(output_tokens)
            .map(|(input, output)| input + output),
    })
}

#[derive(Debug, Default, Deserialize, serde::Serialize)]
struct AnthropicTextDelta {
    #[serde(default)]
    r#type: Option<String>,
    /// Visible response delta carried by `text_delta` content blocks.
    #[serde(default)]
    text: Option<String>,
    /// Extended-thinking delta carried by `thinking_delta` content blocks
    /// when the request enables a thinking budget. Anthropic uses a
    /// distinct field name from `text_delta`'s `text`, so we deserialise
    /// both and pick by `r#type`.
    #[serde(default)]
    thinking: Option<String>,
}

#[derive(Debug, Default, Deserialize, serde::Serialize)]
struct AnthropicStreamErrorBody {
    #[serde(default)]
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        anthropic_content_block_reasoning, anthropic_content_block_text, anthropic_decode_error,
        build_headers, build_input_token_count_body, build_local_tool_body, build_stream_body,
        classify_status, collect_anthropic_tool_calls, create_anthropic_adapter,
        normalize_anthropic_http_error, parse_anthropic_usage, resolve_anthropic_base_url,
        AnthropicMessagesResponse, AnthropicStreamEvent,
    };
    use crate::adapter::{
        EngineAdapterDebugContext, EngineAdapterRequest, EngineAuthConfig, EngineBackendConfig,
        EngineBackendOptions, EngineLocalToolDefinition, EngineOperation, EngineTextStreamRequest,
    };
    use battersea_model::engine::{
        EngineAnthropicThinkingCapabilities, EngineBackendCapabilities, EngineChatParameters,
        EngineChatToolChoiceMode, EngineReasoningEffort, EngineTemperatureDispatch,
        EngineThinkingVisibility,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    const SAMPLE_MODEL: &str = "synthetic-anthropic-model";

    fn supported_parameters(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn budget_capabilities() -> EngineBackendCapabilities {
        EngineBackendCapabilities {
            supported_chat_parameters: supported_parameters(&[
                "stream",
                "maxOutputTokens",
                "temperature",
                "topP",
                "topK",
                "stopSequences",
                "toolExecution",
                "maxToolRounds",
                "toolChoice",
                "parallelToolCalls",
                "strictToolInputs",
                "serviceTier",
                "safetyIdentifier",
                "requestMetadata",
                "reasoningEffort",
                "thinkingBudgetTokens",
                "thinkingVisibility",
            ]),
            supported_tool_execution_modes: vec!["engine-orchestrated".to_string()],
            supported_tool_choices: battersea_model::engine::EngineBackendCapabilities::mock()
                .supported_tool_choices,
            supported_reasoning_efforts: battersea_model::engine::EngineBackendCapabilities::mock()
                .supported_reasoning_efforts,
            temperature_dispatch: EngineTemperatureDispatch::Always,
            reasoning_effort_when_unset: None,
            anthropic_thinking: Some(EngineAnthropicThinkingCapabilities {
                mode: "budget".to_string(),
                disable_supported: true,
                manual_budget_supported: true,
                output_effort: HashMap::new(),
                budget_tokens_by_effort: HashMap::from([
                    ("minimal".to_string(), 1_024),
                    ("low".to_string(), 1_024),
                    ("medium".to_string(), 4_096),
                    ("high".to_string(), 8_192),
                    ("xhigh".to_string(), 16_384),
                ]),
            }),
            google_thinking: None,
        }
    }

    fn adaptive_capabilities(
        disable_supported: bool,
        supports_service_tier: bool,
        xhigh_value: &str,
    ) -> EngineBackendCapabilities {
        let mut supported = supported_parameters(&[
            "stream",
            "maxOutputTokens",
            "stopSequences",
            "toolExecution",
            "maxToolRounds",
            "toolChoice",
            "parallelToolCalls",
            "strictToolInputs",
            "safetyIdentifier",
            "requestMetadata",
            "reasoningEffort",
            "thinkingVisibility",
        ]);
        if supports_service_tier {
            supported.push("serviceTier".to_string());
        }
        EngineBackendCapabilities {
            supported_chat_parameters: supported,
            supported_tool_execution_modes: vec!["engine-orchestrated".to_string()],
            supported_tool_choices: battersea_model::engine::EngineBackendCapabilities::mock()
                .supported_tool_choices,
            supported_reasoning_efforts: battersea_model::engine::EngineBackendCapabilities::mock()
                .supported_reasoning_efforts,
            temperature_dispatch: EngineTemperatureDispatch::Never,
            reasoning_effort_when_unset: None,
            anthropic_thinking: Some(EngineAnthropicThinkingCapabilities {
                mode: "adaptive".to_string(),
                disable_supported,
                manual_budget_supported: false,
                output_effort: HashMap::from([
                    ("minimal".to_string(), "low".to_string()),
                    ("low".to_string(), "low".to_string()),
                    ("medium".to_string(), "medium".to_string()),
                    ("high".to_string(), "high".to_string()),
                    ("xhigh".to_string(), xhigh_value.to_string()),
                ]),
                budget_tokens_by_effort: HashMap::new(),
            }),
            google_thinking: None,
        }
    }

    fn sample_backend() -> EngineBackendConfig {
        EngineBackendConfig {
            id: "anthropic-backend".to_string(),
            provider: "anthropic".to_string(),
            label: "Anthropic".to_string(),
            enabled: true,
            endpoint: "https://api.anthropic.com/v1/messages".to_string(),
            model: SAMPLE_MODEL.to_string(),
            display_order: None,
            chat: EngineChatParameters {
                temperature: 0.2,
                max_output_tokens: Some(512),
                ..EngineChatParameters::default_for_provider("anthropic")
            },
            capabilities: budget_capabilities(),
            context_window_tokens: 200_000,
            options: EngineBackendOptions {
                timeout_ms: Some(600_000),
                max_retries: Some(2),
                stream_idle_timeout_ms: Some(180_000),
                ..EngineBackendOptions::default()
            },
            auth: EngineAuthConfig {
                auth_type: "bearer".to_string(),
                api_key_env: "ANTHROPIC_API_KEY".to_string(),
                header: Some("x-api-key".to_string()),
                version_header: Some("anthropic-version".to_string()),
                version: Some("2023-06-01".to_string()),
                has_api_key: true,
                api_key: Some("secret".to_string()),
            },
            short_description: String::new(),
            long_description: String::new(),
        }
    }

    fn sample_stream_request() -> EngineTextStreamRequest {
        EngineTextStreamRequest {
            mock_response: None,
            shared: EngineAdapterRequest {
                operation: EngineOperation::FlowTextStream,
                messages: vec![
                    battersea_model::Message::text(
                        battersea_model::Role::System,
                        "Stream a reply.".to_string(),
                    ),
                    battersea_model::Message::text(
                        battersea_model::Role::User,
                        "Hello there".to_string(),
                    ),
                ],
                chat: EngineChatParameters {
                    temperature: 0.65,
                    max_output_tokens: Some(1400),
                    ..EngineChatParameters::default_for_provider("anthropic")
                },
                debug_rules: None,
            },
            local_tools: Vec::new(),
            local_tool_executor: None,
            max_tool_rounds: 8,
        }
    }

    #[test]
    fn configured_max_effort_reaches_anthropic() {
        let mut backend = sample_backend();
        backend.capabilities = adaptive_capabilities(false, false, "xhigh");
        let effort = EngineReasoningEffort::Max;
        let native = "max";
        backend
            .capabilities
            .anthropic_thinking
            .as_mut()
            .unwrap()
            .output_effort
            .insert(effort.as_str().into(), native.into());
        let mut request = sample_stream_request();
        request.shared.chat.reasoning_effort = Some(effort);
        let body = build_stream_body(&backend, &request).unwrap();
        assert_eq!(body["output_config"]["effort"], native);
    }

    #[test]
    fn build_local_tool_body_uses_messages_function_tools_with_descriptions() {
        let mut request = sample_stream_request();
        request.local_tools = vec![EngineLocalToolDefinition {
            name: "session.get".to_string(),
            description: "Full snapshot of the active session.".to_string(),
            input_schema: json!({"type":"object","properties":{}}),
        }];
        let body = build_local_tool_body(
            &sample_backend(),
            &request,
            vec![json!({ "role": "user", "content": "Hello" })],
        )
        .expect("body");

        assert_eq!(body["tools"][0]["name"], "session_get");
        assert_eq!(
            body["tools"][0]["input_schema"],
            json!({"type":"object","properties":{}})
        );
        assert_eq!(
            body["tools"][0]["description"],
            "Full snapshot of the active session."
        );
        assert!(body.get("mcp_servers").is_none());

        request.shared.chat.tool_choice.mode = EngineChatToolChoiceMode::Tool;
        request.shared.chat.tool_choice.tool_name = Some("session.get".to_string());
        let body = build_local_tool_body(
            &sample_backend(),
            &request,
            vec![json!({ "role": "user", "content": "Hello" })],
        )
        .expect("body");
        assert_eq!(body["tool_choice"]["name"], json!("session_get"));
    }

    #[test]
    fn collect_anthropic_tool_calls_preserves_tool_use_ids_and_inputs() {
        let blocks = vec![json!({
            "type": "tool_use",
            "id": "toolu_1",
            "name": "session.get",
            "input": {"limit": 2}
        })];

        let calls = collect_anthropic_tool_calls(&blocks).expect("calls");

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_1");
        assert_eq!(calls[0].name, "session.get");
        assert_eq!(calls[0].arguments, json!({"limit": 2}));
    }

    #[test]
    fn anthropic_decode_error_names_backend_model_request_and_body_preview() {
        let context = EngineAdapterDebugContext {
            provider: "anthropic".to_string(),
            backend: "anthropic-debug-backend".to_string(),
            model: "anthropic-debug-model".to_string(),
            sdk: "reqwest".to_string(),
            base_url: Some("https://api.anthropic.com".to_string()),
            timeout_ms: Some(600_000),
            max_retries: Some(2),
        };
        let parse_error = serde_json::from_str::<serde_json::Value>("<html>nope</html>")
            .expect_err("invalid json");

        let error = anthropic_decode_error(
            parse_error,
            "<html>nope</html>",
            Some("req_123".to_string()),
            &context,
            200,
        );

        assert_eq!(error.classification.as_str(), "invalid_response");
        assert_eq!(error.status_code, Some(200));
        assert_eq!(error.request_id.as_deref(), Some("req_123"));
        assert!(error.message.contains("backend `anthropic-debug-backend`"));
        assert!(error.message.contains("model `anthropic-debug-model`"));
        assert!(error.message.contains("request `req_123`"));
        assert!(error.message.contains("preview `<html>nope</html>`"));
    }

    #[test]
    fn anthropic_message_response_accepts_thinking_content_blocks() {
        let payload: AnthropicMessagesResponse = serde_json::from_value(json!({
            "id": "msg_thinking",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "I am planning the response.",
                    "signature": "sig_123"
                },
                {
                    "type": "text",
                    "text": "Here is the answer."
                },
                {
                    "type": "redacted_thinking",
                    "data": "opaque"
                }
            ],
            "usage": {
                "input_tokens": 12,
                "output_tokens": 34
            },
            "stop_reason": "end_turn"
        }))
        .expect("thinking content blocks should parse");

        let visible_text = payload
            .content
            .iter()
            .filter_map(anthropic_content_block_text)
            .collect::<Vec<_>>()
            .join("");
        let reasoning_text = payload
            .content
            .iter()
            .filter_map(anthropic_content_block_reasoning)
            .collect::<Vec<_>>()
            .join("");

        assert_eq!(visible_text, "Here is the answer.");
        assert_eq!(reasoning_text, "I am planning the response.");

        let roundtrip = serde_json::to_value(&payload.content[0]).expect("roundtrip thinking");
        assert_eq!(roundtrip["type"], json!("thinking"));
        assert_eq!(roundtrip["thinking"], json!("I am planning the response."));
        assert_eq!(roundtrip["signature"], json!("sig_123"));
    }

    #[test]
    fn anthropic_response_preserves_unknown_block_types_and_fields_verbatim() {
        // Hardening regression: content blocks are stored raw, so a
        // block `type` the adapter doesn't model must NOT fail
        // deserialisation, and every block (known or not, including
        // fields we don't name like a text block's `citations`) must
        // survive the deserialise -> reserialise round-trip byte-for-
        // byte. The tool loop echoes this straight back to Anthropic,
        // which rejects any modification to the assistant turn.
        let content = json!([
            {
                "type": "thinking",
                "thinking": "reasoning",
                "signature": "sig_abc"
            },
            {
                "type": "text",
                "text": "hi",
                "citations": [{"type": "char_location", "start": 0}]
            },
            {
                "type": "server_tool_use",
                "id": "srv_1",
                "name": "web_search",
                "input": {"q": "weather"},
                "future_field": {"nested": true}
            }
        ]);
        let payload: AnthropicMessagesResponse = serde_json::from_value(json!({
            "id": "msg_unknown",
            "content": content,
        }))
        .expect("unknown block types must not fail deserialisation");

        // The reassembled assistant turn is byte-for-byte identical —
        // unknown type, unmodelled `citations`, and `future_field` all
        // intact.
        let echoed = serde_json::to_value(&payload.content).expect("reserialise content");
        assert_eq!(echoed, content);

        // Typed accessors still classify the blocks they understand.
        assert_eq!(
            payload
                .content
                .iter()
                .filter_map(anthropic_content_block_text)
                .collect::<Vec<_>>(),
            vec!["hi"]
        );
        assert_eq!(
            payload
                .content
                .iter()
                .filter_map(anthropic_content_block_reasoning)
                .collect::<Vec<_>>(),
            vec!["reasoning"]
        );
        // `server_tool_use` is intentionally not collected as an
        // engine-executable local tool call.
        assert!(collect_anthropic_tool_calls(&payload.content)
            .expect("calls")
            .is_empty());
    }

    fn spawn_input_token_count_server() -> (String, thread::JoinHandle<()>) {
        let listener =
            TcpListener::bind(crate::test_endpoints::TEST_BIND_ADDRESS).expect("bind loopback");
        let address = listener.local_addr().expect("local addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 4096];
            let size = stream.read(&mut request).expect("read request");
            let request_text = String::from_utf8_lossy(&request[..size]);
            assert!(request_text.starts_with("POST /v1/messages/count_tokens HTTP/1.1"));
            assert!(request_text.contains("\"model\":\"synthetic-anthropic-model\""));
            let body = r#"{"input_tokens":37}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            stream.flush().expect("flush response");
        });
        (format!("http://{address}/v1/messages"), handle)
    }

    #[test]
    fn create_anthropic_adapter_derives_debug_context_defaults() {
        let adapter = create_anthropic_adapter(sample_backend(), None).expect("create adapter");
        let context = adapter.describe_debug_context();

        assert_eq!(
            context,
            EngineAdapterDebugContext {
                provider: "anthropic".to_string(),
                backend: "anthropic-backend".to_string(),
                model: SAMPLE_MODEL.to_string(),
                sdk: "reqwest".to_string(),
                base_url: Some("https://api.anthropic.com".to_string()),
                timeout_ms: Some(600_000),
                max_retries: Some(2),
            }
        );
    }

    #[test]
    fn create_anthropic_adapter_rejects_invalid_custom_header_names() {
        let mut backend = sample_backend();
        backend.auth.header = Some("bad header".to_string());

        let error = match create_anthropic_adapter(backend, None) {
            Ok(_) => panic!("invalid header should fail"),
            Err(error) => error,
        };
        assert_eq!(error.provider, "anthropic");
        assert_eq!(error.classification.as_str(), "transport");
    }

    #[test]
    fn build_headers_uses_defaults_and_custom_names_and_rejects_invalid_values() {
        let default_headers = build_headers(&sample_backend()).expect("default headers");
        assert_eq!(
            default_headers
                .get("x-api-key")
                .expect("default auth header"),
            "secret"
        );
        assert_eq!(
            default_headers
                .get("anthropic-version")
                .expect("default version header"),
            "2023-06-01"
        );
        assert_eq!(
            default_headers.get("content-type").expect("content type"),
            "application/json"
        );
        assert_eq!(
            default_headers.get("accept").expect("accept"),
            "application/json"
        );

        let mut custom = sample_backend();
        custom.auth.header = Some("x-custom-key".to_string());
        custom.auth.version_header = Some("x-custom-version".to_string());
        custom.auth.version = Some("2024-01-01".to_string());
        let custom_headers = build_headers(&custom).expect("custom headers");
        assert_eq!(
            custom_headers.get("x-custom-key").expect("custom auth"),
            "secret"
        );
        assert_eq!(
            custom_headers
                .get("x-custom-version")
                .expect("custom version"),
            "2024-01-01"
        );

        let mut invalid_value = sample_backend();
        invalid_value.auth.api_key = Some("bad\nvalue".to_string());
        assert_eq!(
            build_headers(&invalid_value)
                .expect_err("invalid header value")
                .classification
                .as_str(),
            "transport"
        );
    }

    #[test]
    fn build_stream_body_includes_stream_flag_and_chat_token_cap() {
        let backend = sample_backend();
        let body = build_stream_body(&backend, &sample_stream_request()).expect("body");

        assert_eq!(body["model"], json!(SAMPLE_MODEL));
        assert_eq!(body["max_tokens"], json!(1400));
        assert_eq!(body["temperature"], json!(0.65));
        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["system"], json!("Stream a reply."));
        assert_eq!(body["messages"][0]["content"], json!("Hello there"));
    }

    #[test]
    fn build_stream_body_maps_advanced_chat_parameters() {
        let backend = sample_backend();
        let mut request = sample_stream_request();
        request.shared.chat.top_p = Some(0.7);
        request.shared.chat.top_k = Some(40);
        request.shared.chat.stop_sequences = vec!["STOP".to_string()];
        request.shared.chat.tool_choice.mode = EngineChatToolChoiceMode::Any;
        request.shared.chat.parallel_tool_calls = Some(false);
        request.shared.chat.service_tier = Some("standard_only".to_string());
        request.shared.chat.safety_identifier = Some("player-1".to_string());
        request.shared.chat.request_metadata = Some(json!({"trace":"abc"}));
        request.shared.chat.thinking_budget_tokens = Some(2048);

        let body = build_stream_body(&backend, &request).expect("body");

        assert_eq!(body["top_p"], json!(0.7));
        assert_eq!(body["top_k"], json!(40));
        assert_eq!(body["stop_sequences"], json!(["STOP"]));
        assert_eq!(body["tool_choice"]["type"], json!("any"));
        assert_eq!(
            body["tool_choice"]["disable_parallel_tool_use"],
            json!(true)
        );
        assert_eq!(body["service_tier"], json!("standard_only"));
        assert_eq!(
            body["metadata"],
            json!({"trace":"abc","user_id":"player-1"})
        );
        assert_eq!(body["thinking"]["type"], json!("enabled"));
        assert_eq!(body["thinking"]["budget_tokens"], json!(2048));
    }

    #[test]
    fn build_stream_body_maps_configured_adaptive_effort_to_output_effort() {
        let mut backend = sample_backend();
        backend.capabilities = adaptive_capabilities(true, true, "high");
        let mut request = sample_stream_request();
        request.shared.chat.temperature = 0.55;
        request.shared.chat.top_k = Some(40);
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::High);
        request.shared.chat.thinking_visibility = Some(EngineThinkingVisibility::Summarized);

        let body = build_stream_body(&backend, &request).expect("body");

        assert!(body.get("temperature").is_none());
        assert!(body.get("top_k").is_none());
        assert_eq!(body["thinking"]["type"], json!("adaptive"));
        assert_eq!(body["thinking"]["display"], json!("summarized"));
        assert_eq!(body["output_config"]["effort"], json!("high"));
        assert!(body["thinking"].get("budget_tokens").is_none());
    }

    #[test]
    fn build_stream_body_ignores_manual_budget_when_adaptive_capability_disallows_it() {
        let mut backend = sample_backend();
        backend.capabilities = adaptive_capabilities(false, true, "xhigh");
        let mut request = sample_stream_request();
        request.shared.chat.temperature = 0.55;
        request.shared.chat.top_k = Some(40);
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::High);
        request.shared.chat.thinking_visibility = Some(EngineThinkingVisibility::Summarized);
        request.shared.chat.thinking_budget_tokens = Some(2048);

        let body = build_stream_body(&backend, &request).expect("body");

        assert!(body.get("temperature").is_none());
        assert!(body.get("top_k").is_none());
        assert_eq!(body["thinking"]["type"], json!("adaptive"));
        assert_eq!(body["thinking"]["display"], json!("summarized"));
        assert_eq!(body["output_config"]["effort"], json!("high"));
        assert!(body["thinking"].get("budget_tokens").is_none());
    }

    #[test]
    fn build_stream_body_maps_configured_medium_effort_to_provider_medium() {
        let mut backend = sample_backend();
        backend.capabilities = adaptive_capabilities(false, true, "xhigh");
        let mut request = sample_stream_request();
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::Medium);

        let body = build_stream_body(&backend, &request).expect("body");

        assert_eq!(body["thinking"]["type"], json!("adaptive"));
        assert_eq!(body["output_config"]["effort"], json!("medium"));
    }

    #[test]
    fn build_stream_body_maps_configured_xhigh_effort_to_provider_xhigh() {
        let mut backend = sample_backend();
        backend.capabilities = adaptive_capabilities(false, true, "xhigh");
        let mut request = sample_stream_request();
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::XHigh);

        let body = build_stream_body(&backend, &request).expect("body");

        assert_eq!(body["thinking"]["type"], json!("adaptive"));
        assert_eq!(body["output_config"]["effort"], json!("xhigh"));
    }

    #[test]
    fn build_stream_body_omits_parameters_not_listed_by_capabilities() {
        let mut backend = sample_backend();
        backend.capabilities = adaptive_capabilities(true, false, "xhigh");
        let mut request = sample_stream_request();
        request.shared.chat.temperature = 0.55;
        request.shared.chat.top_p = Some(0.8);
        request.shared.chat.top_k = Some(40);
        request.shared.chat.service_tier = Some("priority".to_string());
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::High);
        request.shared.chat.thinking_visibility = Some(EngineThinkingVisibility::Summarized);
        request.shared.chat.thinking_budget_tokens = Some(2048);

        let body = build_stream_body(&backend, &request).expect("body");

        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert!(body.get("top_k").is_none());
        assert!(body.get("service_tier").is_none());
        assert_eq!(body["thinking"]["type"], json!("adaptive"));
        assert_eq!(body["thinking"]["display"], json!("summarized"));
        assert!(body["thinking"].get("budget_tokens").is_none());
        assert_eq!(body["output_config"]["effort"], json!("high"));
    }

    #[test]
    fn build_stream_body_maps_configured_summarized_xhigh_effort() {
        let mut backend = sample_backend();
        backend.capabilities = adaptive_capabilities(true, false, "xhigh");
        let mut request = sample_stream_request();
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::XHigh);

        let body = build_stream_body(&backend, &request).expect("body");

        assert_eq!(body["thinking"]["type"], json!("adaptive"));
        assert_eq!(body["output_config"]["effort"], json!("xhigh"));
    }

    #[test]
    fn build_stream_body_can_disable_adaptive_thinking_when_configured() {
        let mut backend = sample_backend();
        backend.capabilities = adaptive_capabilities(true, false, "xhigh");
        let mut request = sample_stream_request();
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::None);

        let body = build_stream_body(&backend, &request).expect("body");

        assert_eq!(body["thinking"]["type"], json!("disabled"));
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn build_stream_body_does_not_disable_adaptive_thinking_when_unsupported() {
        let mut backend = sample_backend();
        backend.capabilities = adaptive_capabilities(false, true, "xhigh");
        let mut request = sample_stream_request();
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::None);

        let body = build_stream_body(&backend, &request).expect("body");

        assert!(body.get("thinking").is_none());
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn build_input_token_count_body_uses_messages_shape_without_streaming() {
        let body = build_input_token_count_body(&sample_backend(), &sample_stream_request());

        assert_eq!(body["model"], json!(SAMPLE_MODEL));
        assert_eq!(body["system"], json!("Stream a reply."));
        assert_eq!(body["messages"][0]["role"], json!("user"));
        assert_eq!(body["messages"][0]["content"], json!("Hello there"));
        assert!(body.get("stream").is_none());
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn parse_anthropic_usage_preserves_cumulative_stream_counts() {
        let usage = parse_anthropic_usage(&json!({
            "input_tokens": 20,
            "cache_creation_input_tokens": 3,
            "cache_read_input_tokens": 2,
            "output_tokens": 7
        }))
        .expect("usage");

        assert_eq!(usage.input_tokens, Some(25));
        assert_eq!(usage.output_tokens, Some(7));
        assert_eq!(usage.total_tokens, Some(32));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn count_text_stream_input_tokens_calls_messages_count_tokens_endpoint() {
        let (url, handle) = spawn_input_token_count_server();
        let mut backend = sample_backend();
        backend.endpoint = url;
        let adapter = create_anthropic_adapter(backend, None).expect("adapter");

        let count = adapter
            .count_text_stream_input_tokens(sample_stream_request())
            .await
            .expect("token count");

        assert_eq!(count, 37);
        handle.join().expect("join server");
    }

    #[test]
    fn normalize_anthropic_http_error_classifies_statuses_and_preserves_metadata() {
        let auth = normalize_anthropic_http_error(
            401,
            Some("req-auth".to_string()),
            Some(r#"{"error":{"message":"bad key"}}"#.to_string()),
        );
        assert_eq!(auth.classification.as_str(), "auth");
        assert_eq!(auth.status_code, Some(401));
        assert_eq!(auth.request_id.as_deref(), Some("req-auth"));
        assert_eq!(auth.message, "bad key");

        assert_eq!(
            normalize_anthropic_http_error(429, None, Some("{}".to_string()))
                .classification
                .as_str(),
            "rate_limit"
        );
        assert_eq!(
            normalize_anthropic_http_error(503, None, Some("{}".to_string()))
                .classification
                .as_str(),
            "server"
        );
        assert_eq!(
            normalize_anthropic_http_error(400, None, Some("{}".to_string()))
                .classification
                .as_str(),
            "request"
        );
    }

    #[test]
    fn normalize_anthropic_http_error_falls_back_to_generic_message_for_blank_or_malformed_body() {
        let blank = normalize_anthropic_http_error(500, None, Some("   ".to_string()));
        let malformed =
            normalize_anthropic_http_error(500, None, Some("not-json-at-all".to_string()));

        assert_eq!(blank.message, "Anthropic request failed.");
        assert_eq!(malformed.message, "Anthropic request failed.");
    }

    #[test]
    fn classify_status_covers_provider_error_buckets() {
        assert_eq!(classify_status(401), "auth");
        assert_eq!(classify_status(403), "auth");
        assert_eq!(classify_status(429), "rate_limit");
        assert_eq!(classify_status(502), "server");
        assert_eq!(classify_status(400), "request");
    }

    #[test]
    fn resolve_anthropic_base_url_trims_known_suffixes_and_query_data() {
        assert_eq!(
            resolve_anthropic_base_url("https://api.anthropic.com/v1/messages?foo=1#frag"),
            "https://api.anthropic.com"
        );
        assert_eq!(
            resolve_anthropic_base_url("https://api.anthropic.com/messages"),
            "https://api.anthropic.com"
        );
        assert_eq!(
            resolve_anthropic_base_url("https://api.anthropic.com/v1"),
            "https://api.anthropic.com"
        );
        assert_eq!(resolve_anthropic_base_url("custom/v1/messages"), "custom");
    }

    #[test]
    fn anthropic_stream_event_accepts_message_delta_metadata_frames() {
        let parsed = serde_json::from_str::<AnthropicStreamEvent>(
            r#"{"type":"message_delta","delta":{"stop_reason":null,"stop_sequence":null},"usage":{"output_tokens":12}}"#,
        )
        .expect("message_delta should parse");

        assert_eq!(parsed.event_type(), "message_delta");
    }

    #[test]
    fn anthropic_stream_event_accepts_additional_known_event_shapes() {
        let ping = serde_json::from_str::<AnthropicStreamEvent>(r#"{"type":"ping"}"#)
            .expect("ping should parse");
        let content_block_start = serde_json::from_str::<AnthropicStreamEvent>(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"hi"}}"#,
        )
        .expect("content block start should parse");

        assert_eq!(ping.event_type(), "ping");
        assert_eq!(content_block_start.event_type(), "content_block_start");
    }
}
