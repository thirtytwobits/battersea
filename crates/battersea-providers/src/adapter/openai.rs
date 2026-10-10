use super::error::EngineAdapterRequestError;
use super::tool_loop::{
    run_tool_loop, send_event, ExecutedToolCall, ToolConversation, ToolEventSender,
};
use super::{
    emit_error, emit_request, emit_response, emit_stream_event, missing_backend_option,
    EngineAdapter, EngineAdapterDebugContext, EngineAdapterLogger, EngineAdapterResponseDetail,
    EngineBackendConfig, EngineLocalToolCall, EngineTextStream, EngineTextStreamEvent,
    EngineTextStreamRequest, EngineTokenUsage,
};
use crate::http_payload::BoundedResponse as _;
use async_openai::error::OpenAIError;
use async_openai::traits::EventType;
use async_openai::types::responses::ResponseStreamEvent;
use async_trait::async_trait;
use battersea_model::engine::{
    EngineChatToolChoiceMode, EngineContextOverflow, EnginePromptCacheRetention,
    EngineReasoningSummary, EngineResponseVerbosity, EngineTemperatureDispatch,
};
use eventsource_stream::Eventsource;
use futures_util::{stream, StreamExt};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// Builds an OpenAI text adapter from backend config, deriving stable debug context values and
/// validating any custom auth header names during construction.
pub(crate) fn create_openai_adapter(
    backend: EngineBackendConfig,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
) -> Result<Arc<dyn EngineAdapter>, EngineAdapterRequestError> {
    crate::retry::validate(&backend)?;
    let timeout_ms = backend
        .options
        .timeout_ms
        .ok_or_else(|| missing_backend_option("openai", "options.timeoutMs"))?;
    let max_retries = backend
        .options
        .max_retries
        .ok_or_else(|| missing_backend_option("openai", "options.maxRetries"))?;
    let stream_idle_timeout_ms = backend
        .options
        .stream_idle_timeout_ms
        .ok_or_else(|| missing_backend_option("openai", "options.streamIdleTimeoutMs"))?;
    // Background mode: the "pro" reasoning models reason silently for
    // minutes, so they run through OpenAI background mode. `backgroundMode`
    // selects "stream" (create with stream:true and consume the SSE feed,
    // reconnecting from the sequence_number cursor across drops) or "poll"
    // (create then poll). The relevant knobs are required per mode — no
    // hidden fallbacks.
    let background = backend.options.background.unwrap_or(false);
    let background_mode = if background {
        backend
            .options
            .background_mode
            .clone()
            .ok_or_else(|| missing_backend_option("openai", "options.backgroundMode"))?
    } else {
        String::new()
    };
    let background_max_wait_ms = if background {
        backend
            .options
            .background_max_wait_ms
            .ok_or_else(|| missing_backend_option("openai", "options.backgroundMaxWaitMs"))?
    } else {
        0
    };
    let (background_poll_interval_ms, background_reconnect_idle_ms, background_max_reconnects) =
        if background {
            match background_mode.as_str() {
                "poll" => (
                    backend.options.background_poll_interval_ms.ok_or_else(|| {
                        missing_backend_option("openai", "options.backgroundPollIntervalMs")
                    })?,
                    0,
                    0,
                ),
                "stream" => (
                    0,
                    backend
                        .options
                        .background_reconnect_idle_ms
                        .ok_or_else(|| {
                            missing_backend_option("openai", "options.backgroundReconnectIdleMs")
                        })?,
                    backend.options.background_max_reconnects.ok_or_else(|| {
                        missing_backend_option("openai", "options.backgroundMaxReconnects")
                    })?,
                ),
                other => {
                    return Err(EngineAdapterRequestError::new(
                        "openai",
                        format!(
                            "Unknown backgroundMode \"{other}\"; expected \"stream\" or \"poll\"."
                        ),
                        "request",
                    ))
                }
            }
        } else {
            (0, 0, 0)
        };
    let base_url = resolve_openai_base_url(&backend.endpoint);
    let backend_id = backend.id.clone();
    let model = backend.model.clone();
    build_openai_headers(&backend)?;
    let http_client = reqwest::Client::builder()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
        .map_err(|error| EngineAdapterRequestError::transport("openai", error.to_string()))?;

    Ok(Arc::new(OpenAiEngineAdapter {
        http_client,
        logger,
        backend,
        base_url: base_url.clone(),
        stream_idle_timeout_ms,
        background,
        background_mode,
        background_poll_interval_ms,
        background_reconnect_idle_ms,
        background_max_reconnects,
        background_max_wait_ms,
        context: EngineAdapterDebugContext {
            provider: "openai".to_string(),
            backend: backend_id,
            model,
            sdk: "reqwest".to_string(),
            base_url: Some(base_url),
            timeout_ms: Some(timeout_ms),
            max_retries: Some(max_retries),
        },
    }))
}

struct OpenAiEngineAdapter {
    http_client: reqwest::Client,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
    backend: EngineBackendConfig,
    base_url: String,
    stream_idle_timeout_ms: u64,
    /// When true, route generations through OpenAI background mode
    /// instead of a plain stream. Set for the "pro" models.
    background: bool,
    /// `"stream"` (resumable SSE) or `"poll"`. Empty when not background.
    background_mode: String,
    background_poll_interval_ms: u64,
    background_reconnect_idle_ms: u64,
    background_max_reconnects: u32,
    background_max_wait_ms: u64,
    context: EngineAdapterDebugContext,
}

fn completed_stop_reason(response: &Value) -> super::StopReason {
    if response["output"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["content"]
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "refusal"))
        })
    }) {
        super::StopReason::Refusal
    } else {
        super::StopReason::Complete
    }
}

fn openai_events(event: &ResponseStreamEvent, raw: &Value) -> Vec<EngineTextStreamEvent> {
    if matches!(event, ResponseStreamEvent::ResponseCompleted(_)) {
        let response = &raw["response"];
        let mut events = response["output"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|payload| EngineTextStreamEvent::ContentBlock {
                block: battersea_model::ContentBlock::Native {
                    provider: "openai".into(),
                    payload: payload.clone(),
                },
            })
            .collect::<Vec<_>>();
        events.push(EngineTextStreamEvent::MessageStop {
            reason: completed_stop_reason(response),
        });
        events
    } else {
        openai_text_stream_events(event)
    }
}

fn openai_text_stream_events(event: &ResponseStreamEvent) -> Vec<EngineTextStreamEvent> {
    match event {
        ResponseStreamEvent::ResponseCompleted(item) => {
            openai_events(event, &json!({"response": item.response}))
        }
        ResponseStreamEvent::ResponseOutputTextDelta(item) => {
            vec![EngineTextStreamEvent::TextDelta {
                text: item.delta.clone(),
            }]
        }
        ResponseStreamEvent::ResponseRefusalDelta(item) => {
            vec![EngineTextStreamEvent::TextDelta {
                text: item.delta.clone(),
            }]
        }
        ResponseStreamEvent::ResponseReasoningSummaryTextDelta(item) => {
            vec![EngineTextStreamEvent::ReasoningDelta {
                text: item.delta.clone(),
            }]
        }
        ResponseStreamEvent::ResponseOutputTextDone(_)
        | ResponseStreamEvent::ResponseRefusalDone(_)
        | ResponseStreamEvent::ResponseReasoningSummaryTextDone(_)
        | ResponseStreamEvent::ResponseReasoningSummaryPartAdded(_)
        | ResponseStreamEvent::ResponseReasoningSummaryPartDone(_) => Vec::new(),
        _ => Vec::new(),
    }
}

#[async_trait]
impl EngineAdapter for OpenAiEngineAdapter {
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
        if self.background {
            return if self.background_mode == "stream" {
                self.background_stream(request).await
            } else {
                self.background_text(request).await
            };
        }

        let body = build_stream_body(&self.backend, &request)?;
        battersea_model::adapter::payload::check_payload(&body, "openai")?;
        emit_request(
            self.logger.as_ref(),
            &self.context,
            &request.shared,
            None,
            Some(body.clone()),
        )
        .await;

        let stream = open_response_stream(
            &self.http_client,
            &self.base_url,
            &self.backend,
            &body,
            &self.logger,
        )
        .await?;
        emit_response(
            self.logger.as_ref(),
            &self.context,
            request.shared.operation,
            200,
            EngineAdapterResponseDetail {
                ok: true,
                request_id: None,
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
        let mapped = stream
            .then(move |event| {
                let logger = logger.clone();
                let context = context.clone();
                async move {
                    match event {
                        Ok((event, event_value)) => {
                            // Per-event scratch: each ResponseStreamEvent variant maps to
                            // zero or more neutral text-stream events. Usage frames (which
                            // share variants with text frames at the protocol level) are
                            // appended below.
                            let text_events = openai_events(&event, &event_value);
                            match &event {
                                ResponseStreamEvent::ResponseFailed(item) => {
                                    let message = item
                                        .response
                                        .error
                                        .as_ref()
                                        .map(|error| error.message.to_string())
                                        .unwrap_or_else(|| {
                                            "OpenAI streaming response failed.".to_string()
                                        });
                                    return Err(EngineAdapterRequestError::invalid_response(
                                        "openai", message,
                                    ));
                                }
                                ResponseStreamEvent::ResponseIncomplete(item) => {
                                    let response = serde_json::to_value(&item.response)
                                        .unwrap_or_else(|_| json!({}));
                                    return Err(openai_missing_output_text_error(&response));
                                }
                                ResponseStreamEvent::ResponseError(item) => {
                                    return Err(EngineAdapterRequestError::transport(
                                        "openai",
                                        item.message.clone(),
                                    ));
                                }
                                _ => {}
                            }

                            emit_stream_event(
                                logger.as_ref(),
                                &context,
                                operation,
                                event.event_type(),
                                event_value.clone(),
                            )
                            .await;

                            let mut events = Vec::new();
                            if let Some(usage) = extract_token_usage(&event_value) {
                                events.push(EngineTextStreamEvent::TokenUsage { usage });
                            }
                            for text_event in text_events {
                                let is_empty = match &text_event {
                                    EngineTextStreamEvent::TextDelta { text }
                                    | EngineTextStreamEvent::ReasoningDelta { text } => {
                                        text.is_empty()
                                    }
                                    _ => false,
                                };
                                if !is_empty {
                                    events.push(text_event);
                                }
                            }

                            Ok(events)
                        }
                        Err(error) => {
                            emit_error(logger.as_ref(), &context, operation, &error).await;
                            Err(error)
                        }
                    }
                }
            })
            .flat_map(|result| {
                let events = match result {
                    Ok(events) => events.into_iter().map(Ok).collect::<Vec<_>>(),
                    Err(error) => vec![Err(error)],
                };
                stream::iter(events)
            })
            .boxed();

        Ok(super::with_stream_idle_watchdog(
            Box::pin(mapped),
            std::time::Duration::from_millis(self.stream_idle_timeout_ms),
            "openai",
        ))
    }

    async fn count_text_stream_input_tokens(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<u64, EngineAdapterRequestError> {
        super::validate_chat_request(&self.backend, &request)?;
        let body = build_input_token_count_body(&self.backend, &request)?;
        let response = crate::retry::send(
            self.http_client
                .post(format!("{}/responses/input_tokens", self.base_url))
                .headers(build_openai_headers(&self.backend)?)
                .json(&body),
            &self.backend,
            self.logger.as_ref(),
        )
        .await?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body = response.bounded_text().await.ok();
            return Err(EngineAdapterRequestError::new(
                "openai",
                body.unwrap_or_else(|| {
                    format!("OpenAI input token count request failed with status {status}.")
                }),
                "request",
            )
            .with_status_code(status));
        }
        let payload: Value = response.bounded_json().await.map_err(|error| {
            EngineAdapterRequestError::invalid_response("openai", error.to_string())
        })?;
        payload
            .get("input_tokens")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                EngineAdapterRequestError::invalid_response(
                    "openai",
                    "OpenAI input token count response omitted input_tokens.",
                )
            })
    }
}

impl OpenAiEngineAdapter {
    async fn stream_text_with_local_tools(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        let executor = request.local_tool_executor.clone().ok_or_else(|| {
            EngineAdapterRequestError::new(
                "openai",
                "Local engine tool executor is unavailable.",
                "request",
            )
        })?;
        let (tx, rx) = mpsc::channel(64);
        let conversation = OpenAiToolConversation {
            http_client: self.http_client.clone(),
            base_url: self.base_url.clone(),
            backend: self.backend.clone(),
            logger: self.logger.clone(),
            context: self.context.clone(),
            input: crate::content::messages("openai", &request.shared.messages)?,
            transport: if !self.background {
                OpenAiTurnTransport::Stream {
                    idle_ms: self.stream_idle_timeout_ms,
                }
            } else if self.background_mode == "stream" {
                OpenAiTurnTransport::BackgroundStream {
                    idle_ms: self.background_reconnect_idle_ms,
                    max_reconnects: self.background_max_reconnects,
                    max_wait_ms: self.background_max_wait_ms,
                }
            } else {
                OpenAiTurnTransport::BackgroundPoll {
                    poll_ms: self.background_poll_interval_ms,
                    max_wait_ms: self.background_max_wait_ms,
                }
            },
        };
        tokio::spawn(run_tool_loop(
            conversation,
            request,
            executor,
            self.logger.clone(),
            self.context.clone(),
            tx,
        ));
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    /// Background-mode plain generation (no tools). Creates the response
    /// with `background: true` and polls it to completion, then emits the
    /// final text + usage. Not wrapped in the idle watchdog: creation
    /// returns instantly and the poll loop is bounded by
    /// `backgroundMaxWaitMs`.
    async fn background_text(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        let body = to_background_body(build_stream_body(&self.backend, &request)?);
        battersea_model::adapter::payload::check_payload(&body, "openai")?;
        emit_request(
            self.logger.as_ref(),
            &self.context,
            &request.shared,
            None,
            Some(body.clone()),
        )
        .await;

        let (tx, rx) =
            mpsc::channel::<Result<EngineTextStreamEvent, EngineAdapterRequestError>>(64);
        let http_client = self.http_client.clone();
        let base_url = self.base_url.clone();
        let backend = self.backend.clone();
        let logger = self.logger.clone();
        let context = self.context.clone();
        let operation = request.shared.operation;
        let poll = self.background_poll_interval_ms;
        let max_wait = self.background_max_wait_ms;

        tokio::spawn(async move {
            let response = tokio::select! {
                _ = tx.closed() => return,
                response = create_and_poll_background(
                &http_client,
                &base_url,
                &backend,
                &body,
                &logger,
                poll,
                max_wait,
                &tx,
            )
                => response,
            };
            match response {
                Ok(response) => {
                    emit_background_terminal_response(&logger, &context, operation, &response, &tx)
                        .await;
                }
                Err(error) => {
                    emit_error(logger.as_ref(), &context, operation, &error).await;
                    let _ = tx.send(Err(error)).await;
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    /// Background-mode plain generation, streamed. Creates the response as
    /// a resumable background SSE stream and forwards tokens live,
    /// reconnecting from the cursor across idle gaps / drops.
    async fn background_stream(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        let body = to_background_stream_body(build_stream_body(&self.backend, &request)?);
        battersea_model::adapter::payload::check_payload(&body, "openai")?;
        emit_request(
            self.logger.as_ref(),
            &self.context,
            &request.shared,
            None,
            Some(body.clone()),
        )
        .await;

        let (tx, rx) =
            mpsc::channel::<Result<EngineTextStreamEvent, EngineAdapterRequestError>>(64);
        let http_client = self.http_client.clone();
        let base_url = self.base_url.clone();
        let backend = self.backend.clone();
        let logger = self.logger.clone();
        let context = self.context.clone();
        let operation = request.shared.operation;
        let reconnect_idle = self.background_reconnect_idle_ms;
        let max_reconnects = self.background_max_reconnects;
        let max_wait = self.background_max_wait_ms;

        tokio::spawn(async move {
            let response = tokio::select! {
                _ = tx.closed() => return,
                response = drive_background_stream(
                &http_client,
                &base_url,
                &backend,
                &body,
                reconnect_idle,
                max_reconnects,
                max_wait,
                &logger,
                &context,
                operation,
                &tx,
            )
                => response,
            };
            match response {
                Ok(response) => {
                    // Text was already streamed live; log completion and
                    // surface a missing-output response as an error.
                    let output_text = read_openai_output_text(&response);
                    let missing = output_text
                        .is_empty()
                        .then(|| openai_missing_output_text_error(&response));
                    emit_response(
                        logger.as_ref(),
                        &context,
                        operation,
                        200,
                        EngineAdapterResponseDetail {
                            ok: missing.is_none(),
                            request_id: None,
                            response_id: response
                                .get("id")
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned),
                            output_chars: Some(output_text.chars().count()),
                            usage: response.get("usage").cloned(),
                            streaming: Some(true),
                            stop_reason: None,
                        },
                    )
                    .await;
                    if let Some(error) = missing {
                        emit_error(logger.as_ref(), &context, operation, &error).await;
                        let _ = tx.send(Err(error)).await;
                    }
                }
                Err(error) => {
                    emit_error(logger.as_ref(), &context, operation, &error).await;
                    let _ = tx.send(Err(error)).await;
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

/// Rewrites a streaming request body for background execution: drops
/// `stream` and sets `background: true`. Everything else (model,
/// instructions, input, tools, reasoning, token caps) is unchanged.
fn to_background_body(mut body: Value) -> Value {
    if let Some(object) = body.as_object_mut() {
        object.remove("stream");
        object.insert("background".to_string(), json!(true));
    }
    body
}

/// True for the terminal states of a background response.
fn is_terminal_background_status(status: &str) -> bool {
    matches!(status, "completed" | "incomplete" | "failed" | "cancelled")
}

/// Maps a terminal background response to either its value (completed /
/// incomplete — the caller extracts whatever text exists) or an error
/// (failed / cancelled / unexpected).
fn finalize_background_response(
    response: Value,
    status: &str,
) -> Result<Value, EngineAdapterRequestError> {
    match status {
        "completed" => Ok(response),
        "incomplete" => Err(openai_missing_output_text_error(&response)),
        "failed" => {
            let message = response
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("OpenAI background response failed.")
                .to_string();
            Err(EngineAdapterRequestError::invalid_response(
                "openai", message,
            ))
        }
        "cancelled" => Err(EngineAdapterRequestError::transport(
            "openai",
            "OpenAI background response was cancelled.".to_string(),
        )),
        other => Err(EngineAdapterRequestError::invalid_response(
            "openai",
            format!("OpenAI background response ended in unexpected status \"{other}\"."),
        )),
    }
}

fn openai_http_error(status: u16, body: Option<String>) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new(
        "openai",
        body.unwrap_or_else(|| format!("OpenAI request failed with status {status}.")),
        "request",
    )
}

/// Best-effort cancel of a background response (to stop server-side spend
/// when the consumer went away or we hit the max-wait ceiling). Ignores
/// the outcome — a "cannot cancel a completed response" error is fine.

#[derive(Clone, Copy)]
enum OpenAiTurnTransport {
    Stream {
        idle_ms: u64,
    },
    BackgroundPoll {
        poll_ms: u64,
        max_wait_ms: u64,
    },
    BackgroundStream {
        idle_ms: u64,
        max_reconnects: u32,
        max_wait_ms: u64,
    },
}

struct OpenAiToolConversation {
    http_client: reqwest::Client,
    base_url: String,
    backend: EngineBackendConfig,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
    context: EngineAdapterDebugContext,
    input: Vec<Value>,
    transport: OpenAiTurnTransport,
}

#[async_trait]
impl ToolConversation for OpenAiToolConversation {
    async fn next_turn(
        &mut self,
        request: &EngineTextStreamRequest,
        results: Vec<ExecutedToolCall>,
        events: &ToolEventSender,
    ) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
        self.input.extend(results.into_iter().map(|item| json!({
            "type": "function_call_output", "call_id": item.call.id, "output": item.result.content.to_string()
        })));
        let body = build_local_tool_body(&self.backend, request, Value::Array(self.input.clone()));
        let body = match self.transport {
            OpenAiTurnTransport::Stream { .. } => body,
            OpenAiTurnTransport::BackgroundPoll { .. } => to_background_body(body),
            OpenAiTurnTransport::BackgroundStream { .. } => to_background_stream_body(body),
        };
        battersea_model::adapter::payload::check_payload(&body, "openai")?;
        emit_request(
            self.logger.as_ref(),
            &self.context,
            &request.shared,
            None,
            Some(body.clone()),
        )
        .await;
        let response = match self.transport {
            OpenAiTurnTransport::Stream { idle_ms } => {
                self.stream_turn(&body, request.shared.operation, events, idle_ms)
                    .await?
            }
            OpenAiTurnTransport::BackgroundPoll {
                poll_ms,
                max_wait_ms,
            } => {
                create_and_poll_background(
                    &self.http_client,
                    &self.base_url,
                    &self.backend,
                    &body,
                    &self.logger,
                    poll_ms,
                    max_wait_ms,
                    events,
                )
                .await?
            }
            OpenAiTurnTransport::BackgroundStream {
                idle_ms,
                max_reconnects,
                max_wait_ms,
            } => {
                drive_background_stream(
                    &self.http_client,
                    &self.base_url,
                    &self.backend,
                    &body,
                    idle_ms,
                    max_reconnects,
                    max_wait_ms,
                    &self.logger,
                    &self.context,
                    request.shared.operation,
                    events,
                )
                .await?
            }
        };
        if response["output"].as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item["content"]
                    .as_array()
                    .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "refusal"))
            })
        }) {
            return Err(EngineAdapterRequestError::invalid_response(
                "openai",
                "Provider refused the tool conversation.",
            ));
        }
        let calls = collect_openai_function_calls(&response)?;
        let output_text = read_openai_output_text(&response);
        if calls.is_empty() && output_text.is_empty() {
            return Err(openai_missing_output_text_error(&response));
        }
        emit_response(
            self.logger.as_ref(),
            &self.context,
            request.shared.operation,
            200,
            EngineAdapterResponseDetail {
                ok: true,
                request_id: None,
                response_id: response
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                output_chars: Some(output_text.chars().count()),
                usage: response.get("usage").cloned(),
                streaming: Some(!matches!(
                    self.transport,
                    OpenAiTurnTransport::BackgroundPoll { .. }
                )),
                stop_reason: None,
            },
        )
        .await;
        if matches!(self.transport, OpenAiTurnTransport::BackgroundPoll { .. }) {
            if !output_text.is_empty() {
                send_event(
                    events,
                    EngineTextStreamEvent::TextDelta { text: output_text },
                )
                .await?;
            }
            if let Some(usage) = extract_token_usage(&response) {
                send_event(events, EngineTextStreamEvent::TokenUsage { usage }).await?;
            }
        }
        if let Some(output) = response.get("output").and_then(Value::as_array) {
            self.input.extend(output.iter().cloned());
        }
        Ok(calls)
    }
}

impl OpenAiToolConversation {
    async fn stream_turn(
        &self,
        body: &Value,
        operation: super::EngineOperation,
        events: &ToolEventSender,
        idle_ms: u64,
    ) -> Result<Value, EngineAdapterRequestError> {
        let mut stream = open_response_stream(
            &self.http_client,
            &self.base_url,
            &self.backend,
            body,
            &self.logger,
        )
        .await?;
        loop {
            let (event, value) =
                tokio::time::timeout(std::time::Duration::from_millis(idle_ms), stream.next())
                    .await
                    .map_err(|_| {
                        EngineAdapterRequestError::transport(
                            "openai",
                            format!("OpenAI stream stalled for {idle_ms}ms."),
                        )
                    })?
                    .ok_or_else(|| {
                        EngineAdapterRequestError::invalid_response(
                            "openai",
                            "OpenAI tool stream ended without ResponseCompleted.",
                        )
                    })??;
            emit_stream_event(
                self.logger.as_ref(),
                &self.context,
                operation,
                event.event_type(),
                value.clone(),
            )
            .await;
            match &event {
                ResponseStreamEvent::ResponseFailed(item) => {
                    return Err(EngineAdapterRequestError::invalid_response(
                        "openai",
                        item.response
                            .error
                            .as_ref()
                            .map(|error| error.message.to_string())
                            .unwrap_or_else(|| "OpenAI response failed.".to_string()),
                    ))
                }
                ResponseStreamEvent::ResponseIncomplete(item) => {
                    return Err(openai_missing_output_text_error(
                        &serde_json::to_value(&item.response).expect("response serializes"),
                    ))
                }
                ResponseStreamEvent::ResponseError(item) => {
                    return Err(EngineAdapterRequestError::transport(
                        "openai",
                        item.message.clone(),
                    ))
                }
                _ => {}
            }
            for event in openai_events(&event, &value) {
                if !matches!(&event, EngineTextStreamEvent::TextDelta { text } | EngineTextStreamEvent::ReasoningDelta { text } if text.is_empty())
                {
                    send_event(events, event).await?;
                }
            }
            if let Some(usage) = extract_token_usage(&value) {
                send_event(events, EngineTextStreamEvent::TokenUsage { usage }).await?;
            }
            if matches!(event, ResponseStreamEvent::ResponseCompleted(_)) {
                return Ok(value["response"].clone());
            }
        }
    }
}

async fn open_response_stream(
    client: &reqwest::Client,
    base_url: &str,
    backend: &EngineBackendConfig,
    body: &Value,
    logger: &Option<Arc<dyn EngineAdapterLogger>>,
) -> Result<
    futures_util::stream::BoxStream<
        'static,
        Result<(ResponseStreamEvent, Value), EngineAdapterRequestError>,
    >,
    EngineAdapterRequestError,
> {
    battersea_model::adapter::payload::check_payload(body, "openai")?;
    let response = crate::retry::send(
        client
            .post(format!("{base_url}/responses"))
            .headers(build_openai_headers(backend)?)
            .json(body),
        backend,
        logger.as_ref(),
    )
    .await?;
    if !response.status().is_success() {
        return Err(openai_http_error(
            response.status().as_u16(),
            response.bounded_text().await.ok(),
        ));
    }
    Ok(crate::http_payload::bounded_stream(response)
        .eventsource()
        .filter_map(|event| async move {
            match event {
                Ok(event) if event.data.trim() == "[DONE]" || event.data.is_empty() => None,
                Ok(event) => Some((|| {
                    let raw: Value = serde_json::from_str(&event.data).map_err(|error| {
                        normalize_openai_error(OpenAIError::JSONDeserialize(
                            error,
                            event.data.clone(),
                        ))
                    })?;
                    let typed = serde_json::from_value(raw.clone()).map_err(|error| {
                        normalize_openai_error(OpenAIError::JSONDeserialize(error, event.data))
                    })?;
                    Ok((typed, raw))
                })()),
                Err(error) => Some(Err(EngineAdapterRequestError::transport(
                    "openai",
                    error.to_string(),
                ))),
            }
        })
        .boxed())
}

/// Cancels server-side work when a provider turn is dropped by the shared runner.
struct BackgroundResponseGuard {
    client: reqwest::Client,
    base_url: String,
    headers: reqwest::header::HeaderMap,
    id: Option<String>,
}

impl BackgroundResponseGuard {
    async fn cancel(&mut self) {
        if let Some(id) = self.id.take() {
            cancel_background_response(&self.client, &self.base_url, &self.headers, &id).await;
        }
    }
}

impl Drop for BackgroundResponseGuard {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let client = self.client.clone();
            let base_url = self.base_url.clone();
            let headers = self.headers.clone();
            tokio::spawn(async move {
                cancel_background_response(&client, &base_url, &headers, &id).await;
            });
        }
    }
}

async fn cancel_background_response(
    http_client: &reqwest::Client,
    base_url: &str,
    headers: &reqwest::header::HeaderMap,
    id: &str,
) {
    let _ = http_client
        .post(format!("{base_url}/responses/{id}/cancel"))
        .headers(headers.clone())
        .send()
        .await;
}

/// Creates a background response and polls it to a terminal state.
///
/// Creation returns almost immediately (status `queued`), so this is not
/// subject to the streaming idle watchdog. The poll loop is bounded by
/// `max_wait_ms` (money-safety ceiling) and aborts early if the consumer
/// drops the stream (activation cancelled), cancelling the response
/// server-side in both cases.
#[allow(clippy::too_many_arguments)]
async fn create_and_poll_background(
    http_client: &reqwest::Client,
    base_url: &str,
    backend: &EngineBackendConfig,
    body: &Value,
    logger: &Option<Arc<dyn EngineAdapterLogger>>,
    poll_interval_ms: u64,
    max_wait_ms: u64,
    tx: &mpsc::Sender<Result<EngineTextStreamEvent, EngineAdapterRequestError>>,
) -> Result<Value, EngineAdapterRequestError> {
    let headers = build_openai_headers(backend)?;

    let created = crate::retry::send(
        http_client
            .post(format!("{base_url}/responses"))
            .headers(headers.clone())
            .json(body),
        backend,
        logger.as_ref(),
    )
    .await?;
    if !created.status().is_success() {
        let status = created.status().as_u16();
        return Err(openai_http_error(status, created.bounded_text().await.ok()));
    }
    let response: Value = created.bounded_json().await.map_err(|error| {
        EngineAdapterRequestError::invalid_response("openai", error.to_string())
    })?;
    let id = response
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            EngineAdapterRequestError::invalid_response(
                "openai",
                "OpenAI background create returned no response id.".to_string(),
            )
        })?
        .to_string();
    let status = response
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if is_terminal_background_status(&status) {
        return finalize_background_response(response, &status);
    }

    let mut guard = BackgroundResponseGuard {
        client: http_client.clone(),
        base_url: base_url.to_string(),
        headers: headers.clone(),
        id: Some(id.clone()),
    };
    let started = tokio::time::Instant::now();
    loop {
        if tx.is_closed() {
            guard.cancel().await;
            return Err(EngineAdapterRequestError::transport(
                "openai",
                "OpenAI background response cancelled — consumer dropped the stream.".to_string(),
            ));
        }
        if started.elapsed() >= std::time::Duration::from_millis(max_wait_ms) {
            guard.cancel().await;
            return Err(EngineAdapterRequestError::transport(
                "openai",
                format!(
                    "OpenAI background response exceeded backgroundMaxWaitMs ({max_wait_ms}ms); \
                     aborted to bound cost."
                ),
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(poll_interval_ms)).await;
        let polled = crate::retry::send(
            http_client
                .get(format!("{base_url}/responses/{id}"))
                .headers(headers.clone()),
            backend,
            logger.as_ref(),
        )
        .await?;
        if !polled.status().is_success() {
            let status = polled.status().as_u16();
            return Err(openai_http_error(status, polled.bounded_text().await.ok()));
        }
        let response: Value = polled.bounded_json().await.map_err(|error| {
            EngineAdapterRequestError::invalid_response("openai", error.to_string())
        })?;
        let status = response
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if is_terminal_background_status(&status) {
            guard.id = None;
            return finalize_background_response(response, &status);
        }
    }
}

/// Emits a completed background response to the consumer: the final text
/// as one `TextDelta` and any usage as `TokenUsage`. A response with no
/// output text is surfaced as an error (mirrors the streaming path's
/// missing-output handling).
async fn emit_background_terminal_response(
    logger: &Option<Arc<dyn EngineAdapterLogger>>,
    context: &EngineAdapterDebugContext,
    operation: super::EngineOperation,
    response: &Value,
    tx: &mpsc::Sender<Result<EngineTextStreamEvent, EngineAdapterRequestError>>,
) {
    let output_text = read_openai_output_text(response);
    let missing_output_error = output_text
        .is_empty()
        .then(|| openai_missing_output_text_error(response));
    emit_response(
        logger.as_ref(),
        context,
        operation,
        200,
        EngineAdapterResponseDetail {
            ok: missing_output_error.is_none(),
            request_id: None,
            response_id: response
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            output_chars: Some(output_text.chars().count()),
            usage: response.get("usage").cloned(),
            streaming: Some(false),
            stop_reason: None,
        },
    )
    .await;
    if let Some(error) = missing_output_error {
        emit_error(logger.as_ref(), context, operation, &error).await;
        let _ = tx.send(Err(error)).await;
        return;
    }
    if tx
        .send(Ok(EngineTextStreamEvent::TextDelta { text: output_text }))
        .await
        .is_err()
    {
        return;
    }
    for payload in response["output"].as_array().into_iter().flatten() {
        if tx
            .send(Ok(EngineTextStreamEvent::ContentBlock {
                block: battersea_model::ContentBlock::Native {
                    provider: "openai".into(),
                    payload: payload.clone(),
                },
            }))
            .await
            .is_err()
        {
            return;
        }
    }
    if let Some(usage) = extract_token_usage(response) {
        let _ = tx
            .send(Ok(EngineTextStreamEvent::TokenUsage { usage }))
            .await;
    }
    let _ = tx
        .send(Ok(EngineTextStreamEvent::MessageStop {
            reason: if response["status"] == "completed" {
                completed_stop_reason(response)
            } else {
                super::StopReason::Length
            },
        }))
        .await;
}

/// Rewrites a streaming request body for resumable background streaming:
/// keeps `stream: true` and adds `background: true` so the response is
/// created as a resumable background SSE stream.
fn to_background_stream_body(mut body: Value) -> Value {
    if let Some(object) = body.as_object_mut() {
        object.insert("stream".to_string(), json!(true));
        object.insert("background".to_string(), json!(true));
    }
    body
}

/// Builds the resume URL for a background stream — continue the SSE feed
/// after the last observed `sequence_number` cursor.
fn background_resume_url(base_url: &str, id: &str, cursor: i64) -> String {
    format!("{base_url}/responses/{id}?stream=true&starting_after={cursor}")
}

/// Drives a resumable background SSE stream to a terminal state.
///
/// Creates the response with `background: true, stream: true`, forwards
/// text/reasoning/usage deltas to `tx` live, and returns the completed
/// response `Value`. If the connection idles past `reconnect_idle_ms`,
/// errors, or ends without a terminal event, it reconnects from the last
/// `sequence_number` cursor (`GET …?stream=true&starting_after=`) instead
/// of aborting — so a long silent reasoning phase never fails the request.
/// Bounded by `max_reconnects` and `max_wait_ms`; cancels the response
/// server-side when a bound trips or the consumer drops the stream.
#[allow(clippy::too_many_arguments)]
async fn drive_background_stream(
    http_client: &reqwest::Client,
    base_url: &str,
    backend: &EngineBackendConfig,
    body: &Value,
    reconnect_idle_ms: u64,
    max_reconnects: u32,
    max_wait_ms: u64,
    logger: &Option<Arc<dyn EngineAdapterLogger>>,
    context: &EngineAdapterDebugContext,
    operation: super::EngineOperation,
    tx: &mpsc::Sender<Result<EngineTextStreamEvent, EngineAdapterRequestError>>,
) -> Result<Value, EngineAdapterRequestError> {
    let headers = build_openai_headers(backend)?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(max_wait_ms);
    let mut guard = BackgroundResponseGuard {
        client: http_client.clone(),
        base_url: base_url.to_string(),
        headers: headers.clone(),
        id: None,
    };
    let mut cursor: i64 = -1;
    let mut reconnects: u32 = 0;

    let created = crate::retry::send(
        http_client
            .post(format!("{base_url}/responses"))
            .headers(headers.clone())
            .json(body),
        backend,
        logger.as_ref(),
    )
    .await?;
    if !created.status().is_success() {
        let status = created.status().as_u16();
        return Err(openai_http_error(status, created.bounded_text().await.ok()));
    }
    let mut sse = Box::pin(crate::http_payload::bounded_stream(created).eventsource());

    loop {
        let reconnect_reason = loop {
            if tx.is_closed() {
                guard.cancel().await;
                return Err(EngineAdapterRequestError::transport(
                    "openai",
                    "OpenAI background stream cancelled — consumer dropped the stream.".to_string(),
                ));
            }
            if tokio::time::Instant::now() >= deadline {
                guard.cancel().await;
                return Err(EngineAdapterRequestError::transport(
                    "openai",
                    format!(
                        "OpenAI background stream exceeded backgroundMaxWaitMs ({max_wait_ms}ms); \
                         aborted to bound cost."
                    ),
                ));
            }
            match tokio::time::timeout(
                std::time::Duration::from_millis(reconnect_idle_ms),
                sse.next(),
            )
            .await
            {
                Ok(Some(Ok(event))) => {
                    let value: Value = match serde_json::from_str(&event.data) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    let seq = value
                        .get("sequence_number")
                        .and_then(Value::as_i64)
                        .filter(|seq| *seq >= 0)
                        .ok_or_else(|| {
                            EngineAdapterRequestError::invalid_response(
                                "openai",
                                "Resumable event omitted its sequence number.",
                            )
                        })?;
                    if seq <= cursor {
                        continue;
                    }
                    cursor = seq;
                    if let Some(id) = value
                        .get("response")
                        .and_then(|response| response.get("id"))
                        .and_then(Value::as_str)
                    {
                        if guard.id.as_deref().is_some_and(|known| known != id) {
                            return Err(EngineAdapterRequestError::invalid_response(
                                "openai",
                                "Resumed stream changed response identity.",
                            ));
                        }
                        guard.id = Some(id.to_string());
                    }
                    let type_name = value
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    emit_stream_event(
                        logger.as_ref(),
                        context,
                        operation,
                        &type_name,
                        value.clone(),
                    )
                    .await;

                    let Ok(event) = serde_json::from_value::<ResponseStreamEvent>(value.clone())
                    else {
                        // An event type the SDK doesn't model — the cursor
                        // has already advanced, so resume stays correct.
                        continue;
                    };
                    for text_event in openai_events(&event, &value) {
                        if matches!(text_event, EngineTextStreamEvent::MessageStop { .. }) {
                            continue;
                        }
                        let is_empty = matches!(
                            &text_event,
                            EngineTextStreamEvent::TextDelta { text }
                                | EngineTextStreamEvent::ReasoningDelta { text }
                                if text.is_empty()
                        );
                        if !is_empty && tx.send(Ok(text_event)).await.is_err() {
                            guard.cancel().await;
                            return Err(EngineAdapterRequestError::transport(
                                "openai",
                                "OpenAI background stream cancelled — consumer dropped the stream."
                                    .to_string(),
                            ));
                        }
                    }
                    match &event {
                        ResponseStreamEvent::ResponseCompleted(_) => {
                            guard.id = None;
                            let response = value["response"].clone();
                            if let Some(usage) = extract_token_usage(&response) {
                                let _ = tx
                                    .send(Ok(EngineTextStreamEvent::TokenUsage { usage }))
                                    .await;
                            }
                            send_event(
                                tx,
                                EngineTextStreamEvent::MessageStop {
                                    reason: completed_stop_reason(&response),
                                },
                            )
                            .await?;
                            return Ok(response);
                        }
                        ResponseStreamEvent::ResponseFailed(item) => {
                            guard.id = None;
                            let message = item
                                .response
                                .error
                                .as_ref()
                                .map(|error| error.message.to_string())
                                .unwrap_or_else(|| {
                                    "OpenAI background response failed.".to_string()
                                });
                            return Err(EngineAdapterRequestError::invalid_response(
                                "openai", message,
                            ));
                        }
                        ResponseStreamEvent::ResponseIncomplete(item) => {
                            guard.id = None;
                            let response =
                                serde_json::to_value(&item.response).unwrap_or_else(|_| json!({}));
                            return Err(openai_missing_output_text_error(&response));
                        }
                        ResponseStreamEvent::ResponseError(item) => {
                            return Err(EngineAdapterRequestError::transport(
                                "openai",
                                item.message.clone(),
                            ));
                        }
                        _ => {}
                    }
                }
                Ok(Some(Err(_stream_error))) => break "stream error",
                Ok(None) => break "stream ended",
                Err(_elapsed) => break "idle",
            }
        };

        let Some(id) = guard.id.clone() else {
            return Err(EngineAdapterRequestError::invalid_response(
                "openai",
                "OpenAI background stream ended before returning a response id; cannot resume."
                    .to_string(),
            ));
        };
        reconnects += 1;
        if reconnects > max_reconnects {
            guard.cancel().await;
            return Err(EngineAdapterRequestError::transport(
                "openai",
                format!(
                    "OpenAI background stream exceeded backgroundMaxReconnects ({max_reconnects}) \
                     after \"{reconnect_reason}\"; aborted."
                ),
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            guard.cancel().await;
            return Err(EngineAdapterRequestError::transport(
                "openai",
                format!(
                    "OpenAI background stream exceeded backgroundMaxWaitMs ({max_wait_ms}ms); \
                     aborted to bound cost."
                ),
            ));
        }
        if let Some(logger) = logger {
            logger
                .on_retry(super::EngineAdapterRetryLog {
                    provider: backend.provider.clone(),
                    backend: backend.id.clone(),
                    attempt: reconnects + 1,
                    delay_ms: 0,
                    dispatch: super::error::DispatchState::Accepted,
                    status_code: None,
                    request_id: Some(id.clone()),
                    resume: true,
                })
                .await;
        }
        let resumed = crate::retry::send(
            http_client
                .get(background_resume_url(base_url, &id, cursor))
                .headers(headers.clone()),
            backend,
            logger.as_ref(),
        )
        .await?;
        if !resumed.status().is_success() {
            let status = resumed.status().as_u16();
            return Err(openai_http_error(status, resumed.bounded_text().await.ok()));
        }
        sse = Box::pin(crate::http_payload::bounded_stream(resumed).eventsource());
    }
}

fn apply_openai_temperature(body: &mut Value, backend: &EngineBackendConfig, temperature: f64) {
    if matches!(
        backend.capabilities.temperature_dispatch,
        EngineTemperatureDispatch::Always
    ) {
        body["temperature"] = json!(temperature);
    }
}

/// Builds the OpenAI streaming request body with streaming explicitly enabled.
fn build_stream_body(
    backend: &EngineBackendConfig,
    request: &EngineTextStreamRequest,
) -> Result<Value, EngineAdapterRequestError> {
    let mut body = json!({
        "model": backend.model,
        "stream": true,
        "instructions": request.shared.text_for_role(battersea_model::Role::System),
        "input": crate::content::messages("openai", &request.shared.messages)?,
    });
    apply_openai_temperature(&mut body, backend, request.shared.chat.temperature);
    apply_openai_max_output_tokens(&mut body, request.shared.chat.max_output_tokens);
    apply_openai_chat_parameters(&mut body, &request.shared);
    crate::content::output_format("openai", &mut body, &request.shared.chat);
    Ok(body)
}

fn build_local_tool_body(
    backend: &EngineBackendConfig,
    request: &EngineTextStreamRequest,
    input: Value,
) -> Value {
    let mut shared = request.shared.clone();
    if let Some(tool_name) = shared.chat.tool_choice.tool_name.as_mut() {
        *tool_name = super::local_tool_name(tool_name);
    }
    let mut body = json!({
        "model": backend.model,
        "stream": true,
        "instructions": shared.text_for_role(battersea_model::Role::System),
        "input": input,
        "tools": request.local_tools.iter().map(|tool| {
            json!({
                "type": "function",
                "name": super::local_tool_name(&tool.name),
                "description": tool.description,
                "parameters": tool.input_schema,
                "strict": shared.chat.strict_tool_inputs.unwrap_or(false),
            })
        }).collect::<Vec<_>>(),
    });
    apply_openai_temperature(&mut body, backend, shared.chat.temperature);
    apply_openai_max_output_tokens(&mut body, shared.chat.max_output_tokens);
    apply_openai_chat_parameters(&mut body, &shared);
    crate::content::output_format("openai", &mut body, &shared.chat);
    body
}

fn apply_openai_max_output_tokens(body: &mut Value, max_output_tokens: Option<u32>) {
    if let Some(max_output_tokens) = max_output_tokens {
        body["max_output_tokens"] = json!(max_output_tokens);
    }
}

fn apply_openai_chat_parameters(body: &mut Value, request: &super::EngineAdapterRequest) {
    let chat = &request.chat;
    if let Some(top_p) = chat.top_p {
        body["top_p"] = json!(top_p);
    }
    if !chat.stop_sequences.is_empty() {
        body["stop"] = json!(chat.stop_sequences);
    }
    apply_openai_tool_choice(
        body,
        chat.tool_choice.mode,
        chat.tool_choice.tool_name.as_deref(),
    );
    if let Some(value) = chat.parallel_tool_calls {
        body["parallel_tool_calls"] = json!(value);
    }
    if let Some(value) = chat.service_tier.as_deref() {
        body["service_tier"] = json!(value);
    }
    if let Some(value) = chat.safety_identifier.as_deref() {
        body["safety_identifier"] = json!(value);
    }
    if let Some(value) = chat.request_metadata.clone() {
        body["metadata"] = value;
    }
    if chat.reasoning_effort.is_some() || chat.reasoning_summary.is_some() {
        let mut reasoning = serde_json::Map::new();
        if let Some(value) = chat.reasoning_effort {
            reasoning.insert("effort".to_string(), json!(value.as_str()));
        }
        if let Some(value) = chat.reasoning_summary {
            reasoning.insert(
                "summary".to_string(),
                json!(openai_reasoning_summary(value)),
            );
        }
        body["reasoning"] = Value::Object(reasoning);
    }
    if let Some(value) = chat.response_verbosity {
        if body.get("text").is_none() {
            body["text"] = json!({});
        }
        body["text"]["verbosity"] = json!(openai_response_verbosity(value));
    }
    body["truncation"] = json!(match chat.context_overflow {
        EngineContextOverflow::Error => "disabled",
        EngineContextOverflow::ProviderTruncate => "auto",
    });
    if let Some(value) = chat.prompt_cache_key.as_deref() {
        body["prompt_cache_key"] = json!(value);
    }
    if let Some(value) = chat.prompt_cache_retention {
        body["prompt_cache_retention"] = json!(openai_prompt_cache_retention(value));
    }
    if let Some(value) = chat.store_response {
        body["store"] = json!(value);
    }
    if let Some(value) = chat.max_provider_tool_calls {
        body["max_tool_calls"] = json!(value);
    }
    if chat.logprobs.unwrap_or(false) || chat.top_logprobs.is_some() {
        body["include"] = json!(["message.output_text.logprobs"]);
    }
    if let Some(value) = chat.top_logprobs {
        body["top_logprobs"] = json!(value);
    }
}

fn apply_openai_tool_choice(
    body: &mut Value,
    mode: EngineChatToolChoiceMode,
    tool_name: Option<&str>,
) {
    match mode {
        EngineChatToolChoiceMode::Auto => {}
        EngineChatToolChoiceMode::None => body["tool_choice"] = json!("none"),
        EngineChatToolChoiceMode::Required | EngineChatToolChoiceMode::Any => {
            body["tool_choice"] = json!("required");
        }
        EngineChatToolChoiceMode::Tool => {
            if let Some(tool_name) = tool_name {
                body["tool_choice"] = json!({
                    "type": "function",
                    "name": tool_name,
                });
            }
        }
    }
}

fn openai_reasoning_summary(value: EngineReasoningSummary) -> &'static str {
    match value {
        EngineReasoningSummary::None => "none",
        EngineReasoningSummary::Auto => "auto",
        EngineReasoningSummary::Concise => "concise",
        EngineReasoningSummary::Detailed => "detailed",
    }
}

fn openai_response_verbosity(value: EngineResponseVerbosity) -> &'static str {
    match value {
        EngineResponseVerbosity::Low => "low",
        EngineResponseVerbosity::Medium => "medium",
        EngineResponseVerbosity::High => "high",
    }
}

fn openai_prompt_cache_retention(value: EnginePromptCacheRetention) -> &'static str {
    match value {
        EnginePromptCacheRetention::InMemory => "in-memory",
        EnginePromptCacheRetention::TwentyFourHours => "24h",
    }
}

fn collect_openai_function_calls(
    response: &Value,
) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
    response
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
        .map(|item| {
            let arguments = match item.get("arguments") {
                Some(Value::String(text)) => Some(serde_json::from_str(text).map_err(|error| {
                    EngineAdapterRequestError::invalid_response(
                        "openai",
                        format!("Invalid tool arguments: {error}"),
                    )
                })?),
                value => value.cloned(),
            };
            super::tool_loop::decode_tool_call(
                "openai",
                item.get("call_id"),
                item.get("name"),
                arguments,
            )
        })
        .collect()
}

fn build_input_token_count_body(
    backend: &EngineBackendConfig,
    request: &EngineTextStreamRequest,
) -> Result<Value, EngineAdapterRequestError> {
    let generation = if request.local_tools.is_empty() {
        build_stream_body(backend, request)?
    } else {
        build_local_tool_body(
            backend,
            request,
            json!(crate::content::messages(
                "openai",
                &request.shared.messages
            )?),
        )
    };
    let mut body = json!({});
    for key in [
        "model",
        "instructions",
        "input",
        "tools",
        "tool_choice",
        "text",
        "reasoning",
    ] {
        if let Some(value) = generation.get(key) {
            body[key] = value.clone();
        }
    }
    Ok(body)
}

fn build_openai_headers(
    backend: &EngineBackendConfig,
) -> Result<HeaderMap, EngineAdapterRequestError> {
    let mut headers = HeaderMap::new();
    let header_name = backend
        .auth
        .header
        .as_deref()
        .ok_or_else(|| missing_backend_option("openai", "auth.header"))?;
    headers.insert(
        HeaderName::from_bytes(header_name.as_bytes())
            .map_err(|error| EngineAdapterRequestError::transport("openai", error.to_string()))?,
        HeaderValue::from_str(&format!(
            "Bearer {}",
            backend.auth.api_key.as_deref().unwrap_or("")
        ))
        .map_err(|error| EngineAdapterRequestError::transport("openai", error.to_string()))?,
    );
    Ok(headers)
}

fn extract_token_usage(value: &Value) -> Option<EngineTokenUsage> {
    let usage = find_usage_object(value)?;
    let input_tokens = usage.get("input_tokens").and_then(Value::as_u64);
    let output_tokens = usage.get("output_tokens").and_then(Value::as_u64);
    let total_tokens = usage.get("total_tokens").and_then(Value::as_u64);
    (input_tokens.is_some() || output_tokens.is_some() || total_tokens.is_some()).then_some(
        EngineTokenUsage {
            cached_input_tokens: usage
                .get("input_tokens_details")
                .and_then(|v| v.get("cached_tokens"))
                .and_then(Value::as_u64)
                .or(input_tokens.map(|_| 0)),
            cache_write_input_tokens: input_tokens.map(|_| 0),
            reasoning_output_tokens: usage
                .get("output_tokens_details")
                .and_then(|v| v.get("reasoning_tokens"))
                .and_then(Value::as_u64),
            turn_index: 0,
            input_tokens,
            output_tokens,
            total_tokens,
        },
    )
}

fn find_usage_object(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    if let Some(usage) = value.get("usage").and_then(Value::as_object) {
        return Some(usage);
    }
    match value {
        Value::Array(items) => items.iter().find_map(find_usage_object),
        Value::Object(object) => object.values().find_map(find_usage_object),
        _ => None,
    }
}

/// Extracts response text from an OpenAI JSON payload, preferring top-level `output_text` before
/// falling back to nested `output[].content[].text` fragments.
fn read_openai_output_text(value: &Value) -> String {
    if let Some(text) = value.get("output_text").and_then(Value::as_str) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    value
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

fn openai_missing_output_text_error(response: &Value) -> EngineAdapterRequestError {
    if let Some(message) = response
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
    {
        return EngineAdapterRequestError::invalid_response(
            "openai",
            format!("OpenAI response failed before producing text: {message}"),
        );
    }

    let status = response.get("status").and_then(Value::as_str);
    if matches!(status, Some("incomplete")) {
        let reason = response
            .get("incomplete_details")
            .and_then(Value::as_object)
            .and_then(|details| details.get("reason"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|reason| !reason.is_empty());
        let message = match reason {
            Some(reason) => {
                format!("OpenAI response ended incomplete before producing text: {reason}.")
            }
            None => "OpenAI response ended incomplete before producing text.".to_string(),
        };
        return EngineAdapterRequestError::invalid_response("openai", message);
    }

    let output_types = collect_openai_output_item_types(response);
    let message = if output_types.is_empty() {
        "OpenAI response did not include output text.".to_string()
    } else {
        format!(
            "OpenAI response did not include output text. Output item types: {}.",
            output_types.join(", ")
        )
    };
    EngineAdapterRequestError::invalid_response("openai", message)
}

fn collect_openai_output_item_types(response: &Value) -> Vec<String> {
    let mut types = Vec::new();
    if let Some(output) = response.get("output").and_then(Value::as_array) {
        for item in output {
            if let Some(kind) = item.get("type").and_then(Value::as_str) {
                let kind = kind.trim();
                if !kind.is_empty() && !types.iter().any(|existing| existing == kind) {
                    types.push(kind.to_string());
                }
            }
        }
    }
    types
}

/// Normalises OpenAI client, API, transport, and deserialisation failures into the shared
/// provider-neutral error shape.
fn normalize_openai_error(error: OpenAIError) -> EngineAdapterRequestError {
    match error {
        OpenAIError::Reqwest(inner) => {
            let mut normalized = EngineAdapterRequestError::transport("openai", inner.to_string());
            if let Some(status) = inner.status() {
                normalized.status_code = Some(status.as_u16());
                normalized.classification = classify_status(status.as_u16()).into();
            }
            normalized
        }
        OpenAIError::ApiError(inner) => EngineAdapterRequestError::new(
            "openai",
            inner.message,
            classify_openai_message(inner.r#type.as_deref(), inner.code.as_deref()),
        ),
        OpenAIError::JSONDeserialize(_, content) => {
            // Before treating as a generic parse failure, check for a provider error envelope.
            // The Responses API streams errors as:
            //   {"type":"error","error":{"type":...,"code":...,"message":...},"sequence_number":N}
            // The Chat Completions API uses:
            //   {"error":{"type":...,"code":...,"message":...}}
            // Both arrive as JSONDeserialize because they don't match the success schema.
            if let Ok(body) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(error_obj) = body.get("error") {
                    let error_type = error_obj.get("type").and_then(serde_json::Value::as_str);
                    let error_code = error_obj.get("code").and_then(serde_json::Value::as_str);
                    let message = error_obj
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("Provider returned an unspecified error");
                    return EngineAdapterRequestError::new(
                        "openai",
                        message,
                        classify_openai_message(error_type, error_code),
                    );
                }
            }
            EngineAdapterRequestError::invalid_response(
                "openai",
                format!("Failed to deserialize OpenAI response: {content}"),
            )
        }
        OpenAIError::StreamError(inner) => {
            EngineAdapterRequestError::transport("openai", inner.to_string())
        }
        OpenAIError::InvalidArgument(message) => {
            EngineAdapterRequestError::new("openai", message, "request")
        }
        other => EngineAdapterRequestError::transport("openai", other.to_string()),
    }
}

/// Classifies structured OpenAI API errors by provider error type and error code.
fn classify_openai_message(error_type: Option<&str>, code: Option<&str>) -> &'static str {
    if matches!(error_type, Some("authentication_error")) || matches!(code, Some("invalid_api_key"))
    {
        "auth"
    } else if matches!(error_type, Some("rate_limit_error"))
        || matches!(code, Some("rate_limit_exceeded"))
    {
        "rate_limit"
    } else {
        "request"
    }
}

/// Classifies OpenAI HTTP status codes into the shared adapter error categories.
fn classify_status(status_code: u16) -> &'static str {
    if matches!(status_code, 401 | 403) {
        "auth"
    } else if status_code == 429 {
        "rate_limit"
    } else if status_code >= 500 {
        "server"
    } else {
        "request"
    }
}

/// Reduces an OpenAI responses endpoint to a reusable base URL by trimming `/responses` and
/// clearing query and fragment data when parsing succeeds.
fn resolve_openai_base_url(endpoint: &str) -> String {
    if let Ok(mut url) = reqwest::Url::parse(endpoint) {
        let maybe_path = url.path().strip_suffix("/responses").map(str::to_string);
        if let Some(path) = maybe_path {
            url.set_path(if path.is_empty() { "/" } else { &path });
        }
        url.set_query(None);
        url.set_fragment(None);
        return url.to_string().trim_end_matches('/').to_string();
    }

    endpoint
        .trim_end_matches("/responses")
        .trim_end_matches('/')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        background_resume_url, build_input_token_count_body, build_local_tool_body,
        build_stream_body, classify_openai_message, classify_status, collect_openai_function_calls,
        create_openai_adapter, drive_background_stream, emit_background_terminal_response,
        extract_token_usage, finalize_background_response, is_terminal_background_status,
        normalize_openai_error, openai_missing_output_text_error, openai_text_stream_events,
        read_openai_output_text, resolve_openai_base_url, to_background_body,
        to_background_stream_body,
    };
    use crate::adapter::{
        EngineAdapterDebugContext, EngineAdapterRequest, EngineAuthConfig, EngineBackendConfig,
        EngineBackendOptions, EngineLocalToolDefinition, EngineOperation, EngineTextStreamEvent,
        EngineTextStreamRequest,
    };
    use async_openai::error::{ApiError, OpenAIError, StreamError};
    use async_openai::types::responses::{
        ResponseRefusalDoneEvent, ResponseStreamEvent, ResponseTextDeltaEvent,
        ResponseTextDoneEvent,
    };
    use battersea_model::engine::{
        EngineBackendCapabilities, EngineChatParameters, EngineChatToolChoiceMode,
        EngineContextOverflow, EnginePromptCacheRetention, EngineReasoningEffort,
        EngineReasoningSummary, EngineResponseVerbosity, EngineTemperatureDispatch,
    };
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    const SAMPLE_MODEL: &str = "synthetic-openai-model";

    fn openai_capabilities(
        temperature_dispatch: EngineTemperatureDispatch,
    ) -> EngineBackendCapabilities {
        EngineBackendCapabilities {
            content: battersea_model::ContentCapabilities::text(),
            supported_chat_parameters: Vec::new(),
            supported_tool_execution_modes: vec!["engine-orchestrated".to_string()],
            supported_tool_choices: battersea_model::engine::EngineBackendCapabilities::mock()
                .supported_tool_choices,
            supported_reasoning_efforts: EngineBackendCapabilities::mock()
                .supported_reasoning_efforts,
            temperature_dispatch,
            reasoning_effort_when_unset: None,
            anthropic_thinking: None,
            google_thinking: None,
        }
    }

    fn sample_backend() -> EngineBackendConfig {
        EngineBackendConfig {
            id: "openai-backend".to_string(),
            provider: "openai".to_string(),
            label: "OpenAI".to_string(),
            enabled: true,
            endpoint: "https://api.openai.com/v1/responses".to_string(),
            model: SAMPLE_MODEL.to_string(),
            display_order: None,
            chat: EngineChatParameters {
                response_format: None,
                output_modalities: None,
                temperature: 0.2,
                ..EngineChatParameters::default_for_provider("openai")
            },
            capabilities: openai_capabilities(EngineTemperatureDispatch::Never),
            context_window_tokens: 400_000,
            options: EngineBackendOptions {
                timeout_ms: Some(600_000),
                max_retries: Some(2),
                retry_initial_delay_ms: Some(1),
                retry_max_delay_ms: Some(20),
                stream_idle_timeout_ms: Some(180_000),
                ..EngineBackendOptions::default()
            },
            auth: EngineAuthConfig {
                auth_type: "bearer".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
                header: Some("Authorization".to_string()),
                version_header: None,
                version: None,
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
                        "Stream text.".to_string(),
                    ),
                    battersea_model::Message::text(
                        battersea_model::Role::User,
                        "Say hello.".to_string(),
                    ),
                ],
                chat: EngineChatParameters {
                    response_format: None,
                    output_modalities: None,
                    temperature: 0.9,
                    ..EngineChatParameters::default_for_provider("openai")
                },
                debug_rules: None,
            },
            local_tools: Vec::new(),
            local_tool_executor: None,
            max_tool_rounds: 8,
        }
    }

    #[test]
    fn reasoning_effort_reaches_openai_without_coercion() {
        for effort in EngineBackendCapabilities::mock().supported_reasoning_efforts {
            let mut request = sample_stream_request();
            request.shared.chat.reasoning_effort = Some(effort);
            let body = build_stream_body(&sample_backend(), &request).unwrap();
            assert_eq!(
                body["reasoning"]["effort"],
                serde_json::to_value(effort).unwrap()
            );
        }
    }

    #[test]
    fn build_local_tool_body_uses_function_tools_with_descriptions() {
        let mut request = sample_stream_request();
        request.local_tools = vec![EngineLocalToolDefinition {
            name: "session.get".to_string(),
            description: "Full snapshot of the active session.".to_string(),
            input_schema: json!({"type":"object","properties":{}}),
        }];
        let body = build_local_tool_body(
            &sample_backend(),
            &request,
            json!([{ "role": "user", "content": "Hello" }]),
        );

        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "session_get");
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
            json!([{ "role": "user", "content": "Hello" }]),
        );
        assert_eq!(body["tool_choice"]["name"], json!("session_get"));
    }

    #[test]
    fn collect_openai_function_calls_parses_call_ids_and_arguments() {
        let calls = collect_openai_function_calls(&json!({
            "output": [{
                "type": "function_call",
                "call_id": "call-1",
                "name": "session.get",
                "arguments": "{\"limit\":2}"
            }]
        }))
        .expect("calls");

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call-1");
        assert_eq!(calls[0].name, "session.get");
        assert_eq!(calls[0].arguments, json!({"limit": 2}));
    }

    fn spawn_status_server(status: u16, body: &'static str) -> (String, thread::JoinHandle<()>) {
        let listener =
            TcpListener::bind(crate::test_endpoints::TEST_BIND_ADDRESS).expect("bind loopback");
        let address = listener.local_addr().expect("local addr");
        let status_line = match status {
            401 => "401 Unauthorized",
            429 => "429 Too Many Requests",
            500 => "500 Internal Server Error",
            _ => "400 Bad Request",
        };
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            stream.flush().expect("flush response");
        });
        (format!("http://{address}/responses"), handle)
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
            assert!(request_text.starts_with("POST /responses/input_tokens HTTP/1.1"));
            assert!(request_text.contains("\"model\":\"synthetic-openai-model\""));
            let body = r#"{"object":"response.input_tokens","input_tokens":42}"#;
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
        (format!("http://{address}/responses"), handle)
    }

    #[test]
    fn create_openai_adapter_derives_debug_context_defaults() {
        let adapter = create_openai_adapter(sample_backend(), None).expect("create adapter");
        let context = adapter.describe_debug_context();

        assert_eq!(
            context,
            EngineAdapterDebugContext {
                provider: "openai".to_string(),
                backend: "openai-backend".to_string(),
                model: SAMPLE_MODEL.to_string(),
                sdk: "reqwest".to_string(),
                base_url: Some("https://api.openai.com/v1".to_string()),
                timeout_ms: Some(600_000),
                max_retries: Some(2),
            }
        );
    }

    #[test]
    fn create_openai_adapter_accepts_custom_non_authorization_header_and_rejects_invalid_names() {
        let mut custom = sample_backend();
        custom.auth.header = Some("x-openai-key".to_string());
        assert!(create_openai_adapter(custom, None).is_ok());

        let mut invalid = sample_backend();
        invalid.auth.header = Some("bad header".to_string());
        let error = match create_openai_adapter(invalid, None) {
            Ok(_) => panic!("invalid header name should fail"),
            Err(error) => error,
        };
        assert_eq!(&*error.provider, "openai");
        assert_eq!(error.classification.as_str(), "transport");
    }

    #[test]
    fn build_stream_body_omits_temperature_when_capability_disallows_it() {
        let body = build_stream_body(&sample_backend(), &sample_stream_request()).unwrap();
        assert_eq!(body["model"], json!(SAMPLE_MODEL));
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn build_stream_body_sends_temperature_when_capability_allows_it() {
        let mut backend = sample_backend();
        backend.capabilities.temperature_dispatch = EngineTemperatureDispatch::Always;
        let body = build_stream_body(&backend, &sample_stream_request()).unwrap();
        assert_eq!(body["temperature"], json!(0.9));
    }

    #[test]
    fn build_stream_body_omits_default_token_cap_and_sends_explicit_cap() {
        let with_default = build_stream_body(&sample_backend(), &sample_stream_request()).unwrap();
        assert_eq!(with_default["model"], json!(SAMPLE_MODEL));
        assert_eq!(with_default["stream"], json!(true));
        assert_eq!(with_default["instructions"], json!("Stream text."));
        assert_eq!(
            with_default["input"][0]["content"][0]["text"],
            json!("Say hello.")
        );
        assert!(with_default.get("max_output_tokens").is_none());

        let mut request = sample_stream_request();
        request.shared.chat.max_output_tokens = Some(99);
        let with_tokens = build_stream_body(&sample_backend(), &request).unwrap();
        assert_eq!(with_tokens["max_output_tokens"], json!(99));
    }

    #[test]
    fn build_stream_body_maps_advanced_chat_parameters() {
        let mut request = sample_stream_request();
        request.shared.chat.top_p = Some(0.8);
        request.shared.chat.stop_sequences = vec!["END".to_string()];
        request.shared.chat.tool_choice.mode = EngineChatToolChoiceMode::Tool;
        request.shared.chat.tool_choice.tool_name = Some("session.get".to_string());
        request.shared.chat.parallel_tool_calls = Some(false);
        request.shared.chat.service_tier = Some("priority".to_string());
        request.shared.chat.safety_identifier = Some("player-1".to_string());
        request.shared.chat.request_metadata = Some(json!({"flow":"main"}));
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::XHigh);
        request.shared.chat.reasoning_summary = Some(EngineReasoningSummary::Detailed);
        request.shared.chat.response_verbosity = Some(EngineResponseVerbosity::Low);
        request.shared.chat.context_overflow = EngineContextOverflow::ProviderTruncate;
        request.shared.chat.prompt_cache_key = Some("session-1".to_string());
        request.shared.chat.prompt_cache_retention =
            Some(EnginePromptCacheRetention::TwentyFourHours);
        request.shared.chat.store_response = Some(false);
        request.shared.chat.max_provider_tool_calls = Some(3);
        request.shared.chat.logprobs = Some(true);
        request.shared.chat.top_logprobs = Some(5);

        let body = build_stream_body(&sample_backend(), &request).unwrap();

        assert_eq!(body["top_p"], json!(0.8));
        assert_eq!(body["stop"], json!(["END"]));
        assert_eq!(body["tool_choice"]["name"], json!("session.get"));
        assert_eq!(body["parallel_tool_calls"], json!(false));
        assert_eq!(body["service_tier"], json!("priority"));
        assert_eq!(body["safety_identifier"], json!("player-1"));
        assert_eq!(body["metadata"], json!({"flow":"main"}));
        assert_eq!(body["reasoning"]["effort"], json!("xhigh"));
        assert_eq!(body["reasoning"]["summary"], json!("detailed"));
        assert_eq!(body["text"]["verbosity"], json!("low"));
        assert_eq!(body["truncation"], json!("auto"));
        assert_eq!(body["prompt_cache_key"], json!("session-1"));
        assert_eq!(body["prompt_cache_retention"], json!("24h"));
        assert_eq!(body["store"], json!(false));
        assert_eq!(body["max_tool_calls"], json!(3));
        assert_eq!(body["include"], json!(["message.output_text.logprobs"]));
        assert_eq!(body["top_logprobs"], json!(5));
    }

    #[test]
    fn build_input_token_count_body_uses_responses_input_shape_without_streaming() {
        let body =
            build_input_token_count_body(&sample_backend(), &sample_stream_request()).unwrap();

        assert_eq!(body["model"], json!(SAMPLE_MODEL));
        assert_eq!(body["instructions"], json!("Stream text."));
        assert_eq!(body["input"][0]["role"], json!("user"));
        assert_eq!(body["input"][0]["content"][0]["text"], json!("Say hello."));
        assert!(body.get("stream").is_none());
    }

    #[test]
    fn extract_token_usage_reads_nested_responses_usage() {
        let usage = extract_token_usage(&json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 12,
                    "output_tokens": 5,
                    "total_tokens": 17
                }
            }
        }))
        .expect("usage");

        assert_eq!(usage.input_tokens, Some(12));
        assert_eq!(usage.output_tokens, Some(5));
        assert_eq!(usage.total_tokens, Some(17));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn count_text_stream_input_tokens_calls_responses_input_tokens_endpoint() {
        let (url, handle) = spawn_input_token_count_server();
        let mut backend = sample_backend();
        backend.endpoint = url;
        let adapter = create_openai_adapter(backend, None).expect("adapter");

        let count = adapter
            .count_text_stream_input_tokens(sample_stream_request())
            .await
            .expect("token count");

        assert_eq!(count, 42);
        handle.join().expect("join server");
    }

    #[test]
    fn read_openai_output_text_prefers_top_level_then_falls_back_to_nested_content() {
        let top_level = json!({
            "output_text": "  Top level text  ",
            "output": [{"content": [{"text": "ignored"}]}]
        });
        let nested = json!({
            "output": [
                {"content": [{"text": "  Hello "}, {"text": "world  "}]},
                {"content": [{"text": " ! "}]}
            ]
        });
        let empty = json!({"output_text": "   ", "output": [{"content": [{"text": ""}]}]});

        assert_eq!(read_openai_output_text(&top_level), "Top level text");
        assert_eq!(read_openai_output_text(&nested), "Hello world   !");
        assert_eq!(read_openai_output_text(&empty), "");
    }

    #[test]
    fn openai_stream_mapper_ignores_done_text_that_repeats_deltas() {
        let delta = ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent {
            sequence_number: 1,
            item_id: "msg-1".to_string(),
            output_index: 0,
            content_index: 0,
            delta: "hello".to_string(),
            logprobs: None,
        });
        let done = ResponseStreamEvent::ResponseOutputTextDone(ResponseTextDoneEvent {
            sequence_number: 2,
            item_id: "msg-1".to_string(),
            output_index: 0,
            content_index: 0,
            text: "hello".to_string(),
            logprobs: None,
        });
        let refusal_done = ResponseStreamEvent::ResponseRefusalDone(ResponseRefusalDoneEvent {
            sequence_number: 3,
            item_id: "msg-1".to_string(),
            output_index: 0,
            content_index: 0,
            refusal: "no".to_string(),
        });

        assert_eq!(
            openai_text_stream_events(&delta),
            vec![EngineTextStreamEvent::TextDelta {
                text: "hello".to_string()
            }]
        );
        assert!(
            openai_text_stream_events(&done).is_empty(),
            "done text repeats the completed output and must not be appended"
        );
        assert!(
            openai_text_stream_events(&refusal_done).is_empty(),
            "done refusal repeats the completed refusal and must not be appended"
        );
    }

    #[test]
    fn openai_missing_output_text_error_reports_provider_response_shape() {
        let incomplete = openai_missing_output_text_error(&json!({
            "status": "incomplete",
            "incomplete_details": {
                "reason": "max_output_tokens"
            },
            "output": [{
                "type": "reasoning"
            }]
        }));
        assert_eq!(incomplete.classification.as_str(), "invalid_response");
        assert_eq!(
            &*incomplete.message,
            "OpenAI response ended incomplete before producing text: max_output_tokens."
        );

        let shaped = openai_missing_output_text_error(&json!({
            "status": "completed",
            "output": [
                {"type": "reasoning"},
                {"type": "reasoning"},
                {"type": "message"}
            ]
        }));
        assert_eq!(
            &*shaped.message,
            "OpenAI response did not include output text. Output item types: reasoning, message."
        );

        let errored = openai_missing_output_text_error(&json!({
            "error": {
                "message": "provider-side failure"
            }
        }));
        assert_eq!(
            &*errored.message,
            "OpenAI response failed before producing text: provider-side failure"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn normalize_openai_error_maps_reqwest_status_api_and_transport_failures() {
        let (url, handle) = spawn_status_server(429, "rate limited");
        let reqwest_error = reqwest::get(url)
            .await
            .expect("response")
            .error_for_status()
            .expect_err("status error");
        handle.join().expect("join server");
        let normalized = normalize_openai_error(OpenAIError::Reqwest(reqwest_error));
        assert_eq!(normalized.classification.as_str(), "rate_limit");
        assert_eq!(normalized.status_code, Some(429));

        let api_auth = normalize_openai_error(OpenAIError::ApiError(ApiError {
            message: "bad key".to_string(),
            r#type: Some("authentication_error".to_string()),
            param: None,
            code: None,
        }));
        assert_eq!(api_auth.classification.as_str(), "auth");
        assert_eq!(&*api_auth.message, "bad key");

        let api_rate_limit = normalize_openai_error(OpenAIError::ApiError(ApiError {
            message: "slow down".to_string(),
            r#type: Some("rate_limit_error".to_string()),
            param: None,
            code: None,
        }));
        assert_eq!(api_rate_limit.classification.as_str(), "rate_limit");

        let invalid_response = normalize_openai_error(OpenAIError::JSONDeserialize(
            serde_json::from_str::<serde_json::Value>("{").expect_err("json error"),
            "{\"bad\":".to_string(),
        ));
        assert_eq!(invalid_response.classification.as_str(), "invalid_response");
        assert!(invalid_response
            .message
            .contains("Failed to deserialize OpenAI response"));

        let stream = normalize_openai_error(OpenAIError::StreamError(Box::new(
            StreamError::EventStream("stream broke".to_string()),
        )));
        assert_eq!(stream.classification.as_str(), "transport");
    }

    #[test]
    fn classify_helpers_and_base_url_resolution_cover_known_cases() {
        assert_eq!(
            classify_openai_message(Some("authentication_error"), None),
            "auth"
        );
        assert_eq!(
            classify_openai_message(None, Some("invalid_api_key")),
            "auth"
        );
        assert_eq!(
            classify_openai_message(Some("rate_limit_error"), None),
            "rate_limit"
        );
        assert_eq!(
            classify_openai_message(None, Some("rate_limit_exceeded")),
            "rate_limit"
        );
        assert_eq!(classify_openai_message(Some("other"), None), "request");

        assert_eq!(classify_status(401), "auth");
        assert_eq!(classify_status(429), "rate_limit");
        assert_eq!(classify_status(500), "server");
        assert_eq!(classify_status(400), "request");

        assert_eq!(
            resolve_openai_base_url("https://api.openai.com/v1/responses?foo=1#frag"),
            "https://api.openai.com/v1"
        );
        assert_eq!(resolve_openai_base_url("custom/responses"), "custom");
    }

    #[test]
    fn normalize_openai_error_detects_rate_limit_in_provider_error_envelopes() {
        let parse_err = || serde_json::from_str::<serde_json::Value>("{").expect_err("json error");

        // Responses API error: {"type":"error","error":{...},"sequence_number":N}
        // This is the exact shape observed in the wild (MarksGate-comely-Bellingham session).
        let responses_api = normalize_openai_error(OpenAIError::JSONDeserialize(
            parse_err(),
            r#"{"type":"error","error":{"type":"invalid_request_error","code":"rate_limit_exceeded","message":"You've exceeded the rate limit, please slow down and try again after 0.0 seconds.","param":null},"sequence_number":2}"#.to_string(),
        ));
        assert_eq!(responses_api.classification.as_str(), "rate_limit");
        assert!(responses_api.message.contains("rate limit"));
        assert_eq!(&*responses_api.provider, "openai");

        // OpenAI Chat Completions API error: {"error":{"type":"rate_limit_error",...}}
        let completions_api = normalize_openai_error(OpenAIError::JSONDeserialize(
            parse_err(),
            r#"{"error":{"type":"rate_limit_error","code":null,"message":"Rate limit reached for requests.","param":null}}"#.to_string(),
        ));
        assert_eq!(completions_api.classification.as_str(), "rate_limit");
        assert_eq!(&*completions_api.provider, "openai");

        // Auth error in envelope form
        let auth_envelope = normalize_openai_error(OpenAIError::JSONDeserialize(
            parse_err(),
            r#"{"error":{"type":"authentication_error","code":null,"message":"Invalid API key.","param":null}}"#.to_string(),
        ));
        assert_eq!(auth_envelope.classification.as_str(), "auth");

        // Unrecognised body still falls back to invalid_response
        let unrecognised = normalize_openai_error(OpenAIError::JSONDeserialize(
            parse_err(),
            r#"{"something_else":"value"}"#.to_string(),
        ));
        assert_eq!(unrecognised.classification.as_str(), "invalid_response");
        assert!(unrecognised
            .message
            .contains("Failed to deserialize OpenAI response"));
    }

    // ---- Background mode ("pro" models) ----

    fn background_backend() -> EngineBackendConfig {
        let mut backend = sample_backend();
        backend.options.background = Some(true);
        backend.options.background_mode = Some("poll".to_string());
        backend.options.background_poll_interval_ms = Some(3_000);
        backend.options.background_max_wait_ms = Some(1_800_000);
        backend
    }

    #[test]
    fn to_background_body_drops_stream_and_sets_background() {
        let streamed = build_stream_body(&sample_backend(), &sample_stream_request()).unwrap();
        assert_eq!(
            streamed["stream"],
            json!(true),
            "precondition: streaming body"
        );
        let background = to_background_body(streamed);
        assert_eq!(background["background"], json!(true));
        assert!(
            background.get("stream").is_none(),
            "stream must be removed so the request is not opened as an SSE stream"
        );
        // The rest of the request survives the rewrite unchanged.
        assert_eq!(background["model"], json!(SAMPLE_MODEL));
        assert_eq!(background["instructions"], json!("Stream text."));
        assert_eq!(
            background["input"][0]["content"][0]["text"],
            json!("Say hello.")
        );
    }

    #[test]
    fn is_terminal_background_status_recognises_terminal_states() {
        for terminal in ["completed", "incomplete", "failed", "cancelled"] {
            assert!(
                is_terminal_background_status(terminal),
                "{terminal} is terminal"
            );
        }
        for pending in ["queued", "in_progress", ""] {
            assert!(
                !is_terminal_background_status(pending),
                "{pending} is not terminal"
            );
        }
    }

    #[test]
    fn finalize_background_response_maps_terminal_states() {
        assert!(
            finalize_background_response(json!({ "status": "completed" }), "completed").is_ok()
        );
        // Incomplete is passed through; the caller extracts whatever text exists.
        assert!(
            finalize_background_response(json!({ "status": "incomplete" }), "incomplete").is_err()
        );

        let failed =
            finalize_background_response(json!({ "error": { "message": "boom" } }), "failed")
                .expect_err("failed status is an error");
        assert_eq!(failed.classification.as_str(), "invalid_response");
        assert!(failed.message.contains("boom"));

        let cancelled = finalize_background_response(json!({}), "cancelled")
            .expect_err("cancelled status is an error");
        assert!(cancelled.message.contains("cancelled"));

        let unexpected = finalize_background_response(json!({}), "banana")
            .expect_err("unknown status is an error");
        assert!(unexpected.message.contains("banana"));
    }

    #[test]
    fn create_openai_adapter_requires_background_suboptions_when_enabled() {
        // background: true, poll mode, but missing the poll interval -> hard error.
        let mut missing = sample_backend();
        missing.options.background = Some(true);
        missing.options.background_mode = Some("poll".to_string());
        missing.options.background_max_wait_ms = Some(1_800_000);
        let error = match create_openai_adapter(missing, None) {
            Ok(_) => panic!("poll mode without poll interval should fail"),
            Err(error) => error,
        };
        assert!(error.message.contains("backgroundPollIntervalMs"));

        // background: true but no mode at all -> hard error naming backgroundMode.
        let mut no_mode = sample_backend();
        no_mode.options.background = Some(true);
        let error = match create_openai_adapter(no_mode, None) {
            Ok(_) => panic!("background without a mode should fail"),
            Err(error) => error,
        };
        assert!(error.message.contains("backgroundMode"));

        // Fully configured background backend constructs fine.
        assert!(create_openai_adapter(background_backend(), None).is_ok());

        // A backend without background stays valid (regression guard).
        assert!(create_openai_adapter(sample_backend(), None).is_ok());
    }

    fn test_debug_context() -> EngineAdapterDebugContext {
        EngineAdapterDebugContext {
            provider: "openai".to_string(),
            backend: "openai-backend".to_string(),
            model: SAMPLE_MODEL.to_string(),
            sdk: "reqwest".to_string(),
            base_url: Some("https://api.openai.com/v1".to_string()),
            timeout_ms: Some(600_000),
            max_retries: Some(2),
        }
    }

    #[test]
    fn refusal_wins_over_completed_response_status() {
        let response = json!({"status":"completed", "output":[{"type":"message","content":[{"type":"refusal","refusal":"denied"}]}]});
        assert_eq!(
            super::completed_stop_reason(&response),
            super::super::StopReason::Refusal
        );
    }

    #[tokio::test]
    async fn emit_background_terminal_response_forwards_text_then_usage() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let response = json!({
            "id": "resp_1",
            "status": "completed",
            "output": [
                { "type": "reasoning", "summary": [] },
                { "type": "message", "content": [{ "type": "output_text", "text": "hello world" }] }
            ],
            "usage": { "input_tokens": 10, "output_tokens": 3 }
        });
        emit_background_terminal_response(
            &None,
            &test_debug_context(),
            EngineOperation::FlowTextStream,
            &response,
            &tx,
        )
        .await;
        drop(tx);

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event.expect("no error expected for a completed response"));
        }
        assert!(
            matches!(&events[0], EngineTextStreamEvent::TextDelta { text } if text == "hello world"),
            "first event is the full output text: {:?}",
            events.first()
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, EngineTextStreamEvent::TokenUsage { .. })),
            "usage is forwarded"
        );
    }

    #[tokio::test]
    async fn emit_background_terminal_response_errors_when_no_output_text() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let response = json!({ "id": "resp_2", "status": "completed", "output": [] });
        emit_background_terminal_response(
            &None,
            &test_debug_context(),
            EngineOperation::FlowTextStream,
            &response,
            &tx,
        )
        .await;
        drop(tx);

        let first = rx.recv().await.expect("an event");
        assert!(
            first.is_err(),
            "a completed response with no output text is surfaced as an error"
        );
    }

    // ---- Background streaming mode (resumable SSE) ----

    #[test]
    fn to_background_stream_body_keeps_stream_and_sets_background() {
        let streamed = build_stream_body(&sample_backend(), &sample_stream_request()).unwrap();
        let background = to_background_stream_body(streamed);
        assert_eq!(
            background["stream"],
            json!(true),
            "stream stays on for resumable SSE"
        );
        assert_eq!(background["background"], json!(true));
        assert_eq!(background["model"], json!(SAMPLE_MODEL));
    }

    #[test]
    fn background_resume_url_encodes_cursor() {
        assert_eq!(
            background_resume_url("https://api.openai.com/v1", "resp_abc", 7),
            "https://api.openai.com/v1/responses/resp_abc?stream=true&starting_after=7"
        );
    }

    #[test]
    fn create_openai_adapter_validates_background_mode_and_stream_suboptions() {
        // Unknown mode is rejected.
        let mut bad_mode = background_backend();
        bad_mode.options.background_mode = Some("teleport".to_string());
        let error = match create_openai_adapter(bad_mode, None) {
            Ok(_) => panic!("unknown backgroundMode should fail"),
            Err(error) => error,
        };
        assert!(error.message.contains("Unknown backgroundMode"));

        // Stream mode requires the reconnect knobs.
        let mut missing = background_backend();
        missing.options.background_mode = Some("stream".to_string());
        missing.options.background_reconnect_idle_ms = None;
        let error = match create_openai_adapter(missing, None) {
            Ok(_) => panic!("stream mode without reconnect idle should fail"),
            Err(error) => error,
        };
        assert!(error.message.contains("backgroundReconnectIdleMs"));

        // Fully configured stream-mode backend constructs.
        let mut stream_ok = background_backend();
        stream_ok.options.background_mode = Some("stream".to_string());
        stream_ok.options.background_reconnect_idle_ms = Some(120_000);
        stream_ok.options.background_max_reconnects = Some(30);
        assert!(create_openai_adapter(stream_ok, None).is_ok());
    }

    /// Mock server that serves two SSE connections. Each connection sends
    /// one `response.created` event (carrying the response id and
    /// `sequence_number: 2`) then closes, forcing the adapter to reconnect
    /// from the cursor. Captures each connection's request line.
    fn spawn_background_reconnect_server() -> (
        String,
        std::sync::mpsc::Receiver<String>,
        thread::JoinHandle<()>,
    ) {
        let listener =
            TcpListener::bind(crate::test_endpoints::TEST_BIND_ADDRESS).expect("bind loopback");
        let address = listener.local_addr().expect("local addr");
        let (req_tx, req_rx) = std::sync::mpsc::channel::<String>();
        let handle = thread::spawn(move || {
            let sse = "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\"},\"sequence_number\":2}\n\n";
            let http = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{sse}"
            );
            for _ in 0..2 {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(_) => return,
                };
                let mut buf = [0_u8; 4096];
                let size = stream.read(&mut buf).unwrap_or(0);
                let _ = req_tx.send(String::from_utf8_lossy(&buf[..size]).to_string());
                let _ = stream.write_all(http.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("http://{address}"), req_rx, handle)
    }

    #[tokio::test]
    async fn background_stream_reconnects_from_cursor_and_honours_max_reconnects() {
        let (base, req_rx, handle) = spawn_background_reconnect_server();
        let mut backend = background_backend();
        backend.endpoint = format!("{base}/responses");
        let base_url = resolve_openai_base_url(&backend.endpoint);
        let http = reqwest::Client::new();
        let body =
            json!({ "model": backend.model, "stream": true, "background": true, "input": [] });
        // A live sender so tx.is_closed() stays false during the run.
        let (tx, _rx) = tokio::sync::mpsc::channel(64);

        let result = drive_background_stream(
            &http,
            &base_url,
            &backend,
            &body,
            60_000, // reconnect idle (won't fire — connections end promptly)
            1,      // max reconnects
            60_000, // max wait
            &None,
            &test_debug_context(),
            EngineOperation::FlowTextStream,
            &tx,
        )
        .await;

        let error = match result {
            Ok(_) => panic!("bounded reconnect should error, not complete"),
            Err(error) => error,
        };
        assert!(
            error.message.contains("backgroundMaxReconnects"),
            "expected a max-reconnects error, got: {}",
            error.message
        );

        // First connection is the create POST; the second is the cursor resume.
        let create_req = req_rx.recv().expect("create request");
        assert!(
            create_req.starts_with("POST /responses"),
            "create: {create_req}"
        );
        let resume_req = req_rx.recv().expect("resume request");
        assert!(
            resume_req.starts_with("GET /responses/resp_test?stream=true&starting_after=2"),
            "resume must carry the cursor: {resume_req}"
        );
        handle.join().ok();
    }
}
