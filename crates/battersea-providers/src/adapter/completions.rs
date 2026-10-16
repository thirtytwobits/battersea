//! OpenAI Chat Completions, with a distinct request and stream codec.
use super::tool_loop::*;
use super::*;
use crate::http_payload::BoundedResponse as _;
use async_trait::async_trait;
use battersea_model::{engine::*, *};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};
use tokio_stream::wrappers::ReceiverStream;

pub(crate) fn registration() -> ChatProvider {
    ChatProvider {
        content:ContentCapabilities { input_modalities:vec![Modality::Text,Modality::Image],output_modalities:vec![Modality::Text],structured_output:vec![StructuredOutputMode::JsonObject,StructuredOutputMode::JsonSchema],native_continuation:true,prompt_caching:false },
        parameters:"responseFormat outputModalities model stream maxOutputTokens temperature topP stopSequences toolExecution maxToolRounds toolChoice parallelToolCalls strictToolInputs contextOverflow".split_whitespace().map(str::to_owned).collect(),
        background_modes:vec![],factory:Arc::new(create),
    }
}
fn create(
    backend: EngineBackendConfig,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
) -> Result<Arc<dyn EngineAdapter>, EngineAdapterRequestError> {
    crate::retry::validate(&backend)?;
    let binding = backend
        .binding
        .as_ref()
        .ok_or_else(|| missing_backend_option(&backend.provider, "binding"))?;
    crate::connection::headers(binding, &backend.auth)?;
    if backend.chat.context_overflow != EngineContextOverflow::Error {
        return Err(EngineAdapterRequestError::new(
            &backend.provider,
            "Chat Completions requires contextOverflow=error.",
            ErrorKind::InvalidRequest,
        ));
    }
    let client = reqwest::Client::builder()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(
            backend
                .options
                .timeout_ms
                .ok_or_else(|| missing_backend_option(&backend.provider, "timeoutMs"))?,
        ))
        .build()
        .map_err(|_| {
            EngineAdapterRequestError::new(
                &backend.provider,
                "Could not construct HTTP client.",
                ErrorKind::InvalidRequest,
            )
        })?;
    Ok(Arc::new(Completions {
        backend,
        logger,
        client,
    }))
}
struct Completions {
    backend: EngineBackendConfig,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
    client: reqwest::Client,
}
#[async_trait]
impl EngineAdapter for Completions {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        EngineAdapterDebugContext {
            provider: self.backend.provider.clone(),
            backend: self.backend.id.clone(),
            model: self.backend.model.clone(),
            sdk: "reqwest".into(),
            base_url: Some(self.backend.endpoint.clone()),
            timeout_ms: self.backend.options.timeout_ms,
            max_retries: self.backend.options.max_retries,
        }
    }
    async fn count_text_stream_input_tokens(
        &self,
        _request: EngineTextStreamRequest,
    ) -> Result<u64, EngineAdapterRequestError> {
        Err(EngineAdapterRequestError::new(
            &self.backend.provider,
            "Chat Completions does not implement preflight input counting.",
            ErrorKind::InvalidRequest,
        ))
    }
    async fn stream_text(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        validate_chat_request(&self.backend, &request)?;
        if request.shared.chat.context_overflow != EngineContextOverflow::Error {
            return Err(EngineAdapterRequestError::new(
                &self.backend.provider,
                "Chat Completions requires contextOverflow=error.",
                ErrorKind::InvalidRequest,
            ));
        }
        let messages = messages(&self.backend.provider, &request.shared.messages)?;
        let context = self.describe_debug_context();
        let conversation = Conversation {
            backend: self.backend.clone(),
            logger: self.logger.clone(),
            client: self.client.clone(),
            messages,
            context: context.clone(),
        };
        let executor = request
            .local_tool_executor
            .clone()
            .unwrap_or_else(|| Arc::new(ToolRegistry::default()));
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        tokio::spawn(run_tool_loop(
            conversation,
            request,
            executor,
            self.logger.clone(),
            context,
            tx,
        ));
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
fn messages(provider: &str, messages: &[Message]) -> Result<Vec<Value>, EngineAdapterRequestError> {
    let mut result = Vec::new();
    for message in messages {
        let role = match message.role {
            Role::System => "system",
            Role::Assistant => "assistant",
            Role::User => "user",
            Role::Tool => "tool",
        };
        let mut content = Vec::new();
        let mut calls = Vec::new();
        let mut special = false;
        for block in &message.content {
            match block {
                ContentBlock::Text { text } => content.push(json!({"type":"text","text":text})),
                ContentBlock::Image { url,mime_type } => { crate::content::media_source("openai",provider,url,mime_type)?;content.push(json!({"type":"image_url","image_url":{"url":url}})); },
                ContentBlock::ToolCall { id,name,arguments } => calls.push(json!({"id":id,"type":"function","function":{"name":local_tool_name(name),"arguments":arguments.to_string()}})),
                ContentBlock::ToolResult { id,content } => { special=true;result.push(json!({"role":"tool","tool_call_id":id,"content":content.to_string()})); },
                ContentBlock::Native { provider:owner,payload } if owner == provider && payload["role"] == "assistant" => { special=true;result.push(payload.clone()); },
                _ => return Err(EngineAdapterRequestError::new(provider,"Unsupported Chat Completions content.",ErrorKind::InvalidRequest)),
            }
        }
        if special && (!content.is_empty() || !calls.is_empty()) {
            return Err(EngineAdapterRequestError::new(
                provider,
                "Native or tool-result messages must retain their own envelope.",
                ErrorKind::InvalidRequest,
            ));
        }
        if !special {
            let mut value = json!({"role":role,"content":if content.is_empty() { Value::Null } else { json!(content) }});
            if !calls.is_empty() {
                value["tool_calls"] = json!(calls);
            }
            result.push(value);
        }
    }
    Ok(result)
}
struct Conversation {
    backend: EngineBackendConfig,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
    client: reqwest::Client,
    messages: Vec<Value>,
    context: EngineAdapterDebugContext,
}
#[async_trait]
impl ToolConversation for Conversation {
    async fn next_turn(
        &mut self,
        request: &EngineTextStreamRequest,
        results: Vec<ExecutedToolCall>,
        events: &ToolEventSender,
    ) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
        let provider = &self.backend.provider;
        for result in results {
            self.messages.push(json!({"role":"tool","tool_call_id":result.call.id,"content":result.result.content.to_string()}));
        }
        let chat = &request.shared.chat;
        let mut body =
            json!({"model":self.backend.model,"messages":self.messages,"stream":chat.stream});
        if self.backend.capabilities.temperature_dispatch == EngineTemperatureDispatch::Always {
            body["temperature"] = json!(chat.temperature);
        }
        if let Some(value) = chat.max_output_tokens {
            body["max_completion_tokens"] = json!(value);
        }
        if let Some(value) = chat.top_p {
            body["top_p"] = json!(value);
        }
        if !chat.stop_sequences.is_empty() {
            body["stop"] = json!(chat.stop_sequences);
        }
        if let Some(format) = &chat.response_format {
            body["response_format"] = match format {
                ResponseFormat::JsonObject => json!({"type":"json_object"}),
                ResponseFormat::JsonSchema { name, schema } => {
                    json!({"type":"json_schema","json_schema":{"name":name,"schema":schema,"strict":true}})
                }
            };
        }
        if !request.local_tools.is_empty() {
            body["tools"] = json!(request.local_tools.iter().map(|tool| {
                let mut function = json!({"name":local_tool_name(&tool.name),"description":tool.description,"parameters":tool.input_schema});
                if let Some(strict) = chat.strict_tool_inputs { function["strict"] = json!(strict); }
                json!({"type":"function","function":function})
            }).collect::<Vec<_>>());
            body["tool_choice"] = match chat.tool_choice.mode {
                EngineChatToolChoiceMode::Auto => json!("auto"),
                EngineChatToolChoiceMode::None => json!("none"),
                EngineChatToolChoiceMode::Required | EngineChatToolChoiceMode::Any => {
                    json!("required")
                }
                EngineChatToolChoiceMode::Tool => {
                    json!({"type":"function","function":{"name":local_tool_name(chat.tool_choice.tool_name.as_deref().unwrap_or(""))}})
                }
            };
            if let Some(parallel) = chat.parallel_tool_calls {
                body["parallel_tool_calls"] = json!(parallel);
            }
        }
        if chat.stream {
            body["stream_options"] = json!({"include_usage":true});
        }
        payload::check_payload(&body, provider)?;
        emit_request(
            self.logger.as_ref(),
            &self.context,
            &request.shared,
            None,
            Some(body.clone()),
        )
        .await;
        let url = self
            .backend
            .binding
            .as_ref()
            .unwrap()
            .connection
            .route("chat/completions")?;
        let response = crate::retry::send(
            self.client.post(url).json(&body),
            &self.backend,
            self.logger.as_ref(),
        )
        .await?;
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let status = response.status().as_u16();
        let (message, finish, usage) = if chat.stream {
            decode_stream(provider, response, events).await?
        } else {
            let value: Value = response.bounded_json().await?;
            if value["choices"].as_array().is_none_or(|v| v.len() != 1) {
                return Err(invalid(provider, "Expected one completion choice."));
            }
            let choice = &value["choices"][0];
            (
                choice["message"].clone(),
                choice["finish_reason"].as_str().unwrap_or("").to_owned(),
                value["usage"].clone(),
            )
        };
        if !matches!(finish.as_str(), "stop" | "tool_calls")
            || message["role"] != "assistant"
            || !message["refusal"].is_null()
        {
            return Err(invalid(
                provider,
                "Completion was refused, incomplete or malformed.",
            ));
        }
        let mut calls = Vec::new();
        if !message["tool_calls"].is_null() && !message["tool_calls"].is_array() {
            return Err(invalid(provider, "Malformed completion tool calls."));
        }
        if let Some(items) = message["tool_calls"].as_array() {
            for call in items {
                if call["type"] != "function" {
                    return Err(invalid(provider, "Unsupported completion tool type."));
                }
                let arguments = call["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok());
                calls.push(decode_tool_call(
                    provider,
                    call.get("id"),
                    call["function"].get("name"),
                    arguments,
                )?);
            }
        }
        if (finish == "tool_calls") == calls.is_empty() {
            return Err(invalid(
                provider,
                "Finish reason disagrees with the tool calls.",
            ));
        }
        if !chat.stream {
            if let Some(text) = message["content"].as_str() {
                send_event(
                    events,
                    EngineTextStreamEvent::TextDelta { text: text.into() },
                )
                .await?;
            } else if !message["content"].is_null() {
                return Err(invalid(provider, "Unsupported completion content."));
            }
        }
        if usage.is_object() {
            send_event(
                events,
                EngineTextStreamEvent::TokenUsage {
                    usage: EngineTokenUsage {
                        turn_index: 0,
                        input_tokens: usage["prompt_tokens"].as_u64(),
                        output_tokens: usage["completion_tokens"].as_u64(),
                        total_tokens: usage["total_tokens"].as_u64(),
                        cached_input_tokens: usage["prompt_tokens_details"]["cached_tokens"]
                            .as_u64(),
                        cache_write_input_tokens: usage["prompt_tokens_details"]
                            ["cache_write_tokens"]
                            .as_u64(),
                        reasoning_output_tokens: usage["completion_tokens_details"]
                            ["reasoning_tokens"]
                            .as_u64(),
                    },
                },
            )
            .await?;
        }
        emit_response(
            self.logger.as_ref(),
            &self.context,
            request.shared.operation,
            status,
            EngineAdapterResponseDetail {
                ok: true,
                request_id,
                response_id: None,
                output_chars: message["content"].as_str().map(str::len),
                usage: usage.is_object().then_some(usage),
                streaming: Some(chat.stream),
                stop_reason: Some(finish),
            },
        )
        .await;
        send_event(
            events,
            EngineTextStreamEvent::ContentBlock {
                block: ContentBlock::Native {
                    provider: provider.clone(),
                    payload: message.clone(),
                },
            },
        )
        .await?;
        self.messages.push(message);
        Ok(calls)
    }
}
fn invalid(provider: &str, message: &str) -> EngineAdapterRequestError {
    EngineAdapterRequestError::invalid_response(provider, message)
}
async fn decode_stream(
    provider: &str,
    response: reqwest::Response,
    events: &ToolEventSender,
) -> Result<(Value, String, Value), EngineAdapterRequestError> {
    let stream = crate::http_payload::bounded_stream(response).eventsource();
    futures_util::pin_mut!(stream);
    let mut text = String::new();
    let mut finish = String::new();
    let mut usage = Value::Null;
    let mut tools = BTreeMap::<u64, Value>::new();
    let mut done = false;
    while let Some(event) = stream.next().await {
        let event = event.map_err(|_| invalid(provider, "Interrupted completion stream."))?;
        if event.data == "[DONE]" {
            done = true;
            break;
        }
        let value: Value = serde_json::from_str(&event.data)
            .map_err(|_| invalid(provider, "Malformed completion event."))?;
        if value["usage"].is_object() {
            usage = value["usage"].clone();
        }
        let choices = value["choices"]
            .as_array()
            .ok_or_else(|| invalid(provider, "Missing stream choices."))?;
        if choices.is_empty() {
            continue;
        }
        if choices.len() != 1 || choices[0]["index"] != 0 {
            return Err(invalid(provider, "Unexpected completion choice."));
        }
        if !finish.is_empty() {
            return Err(invalid(
                provider,
                "Completion content followed its terminal choice.",
            ));
        }
        let delta = &choices[0]["delta"];
        if !delta.is_object()
            || (!delta["role"].is_null() && delta["role"] != "assistant")
            || (!delta["content"].is_null() && !delta["content"].is_string())
            || (!delta["tool_calls"].is_null() && !delta["tool_calls"].is_array())
        {
            return Err(invalid(provider, "Malformed completion delta."));
        }
        if delta
            .as_object()
            .unwrap()
            .keys()
            .any(|key| !matches!(key.as_str(), "role" | "content" | "tool_calls" | "refusal"))
        {
            return Err(invalid(provider, "Unsupported completion delta field."));
        }
        if !delta["refusal"].is_null() {
            return Err(invalid(provider, "Completion refused."));
        }
        if let Some(part) = delta["content"].as_str() {
            text.push_str(part);
            send_event(
                events,
                EngineTextStreamEvent::TextDelta { text: part.into() },
            )
            .await?;
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for call in calls {
                if !call["type"].is_null() && call["type"] != "function" {
                    return Err(invalid(provider, "Unsupported streamed tool type."));
                }
                let index = call["index"]
                    .as_u64()
                    .ok_or_else(|| invalid(provider, "Missing tool index."))?;
                let accumulated = tools.entry(index).or_insert_with(
                    || json!({"id":"","type":"function","function":{"name":"","arguments":""}}),
                );
                for pointer in ["/id", "/function/name", "/function/arguments"] {
                    if call
                        .pointer(pointer)
                        .is_some_and(|v| !v.is_null() && !v.is_string())
                    {
                        return Err(invalid(provider, "Malformed tool delta."));
                    }
                    if let Some(part) = call.pointer(pointer).and_then(Value::as_str) {
                        let target = accumulated.pointer_mut(pointer).unwrap();
                        *target = json!(format!("{}{part}", target.as_str().unwrap()));
                    }
                }
            }
        }
        if let Some(reason) = choices[0]["finish_reason"].as_str() {
            if !finish.is_empty() {
                return Err(invalid(provider, "Repeated completion finish."));
            }
            finish = reason.into();
        }
        payload::check_payload(&(&text, &tools), provider)?;
    }
    if !done || finish.is_empty() {
        return Err(invalid(
            provider,
            "Completion stream ended before its terminal event.",
        ));
    }
    let mut message = json!({"role":"assistant","content":text});
    if !tools.is_empty() {
        message["tool_calls"] = json!(tools.into_values().collect::<Vec<_>>());
    }
    Ok((message, finish, usage))
}
