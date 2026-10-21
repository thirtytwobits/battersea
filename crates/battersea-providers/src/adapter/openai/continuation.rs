//! Copyright (c) Scott A Dixon
//! Responses checkpoints retain native evidence and derive durable observations.
use super::*;
use battersea_model::{
    adapter::payload::check_payload, configuration_digest, ContentBlock, ConversationBinding,
    ConversationRequest, ConversationToolResult, ConversationTurn, ErrorKind, StopReason, ToolCode,
};
use serde::{Deserialize, Serialize};

mod background;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: u32,
    binding: ConversationBinding,
    request_revision: String,
    /// Exact input sent for this turn, including the preceding tool-result projections.
    input: Vec<Value>,
    response: Value,
}

fn invalid(message: &str) -> EngineAdapterRequestError {
    EngineAdapterRequestError::invalid_response("openai", message)
}

fn decode(state: &Value) -> Result<Checkpoint, EngineAdapterRequestError> {
    check_payload(state, "openai")?;
    serde_json::from_value(state.clone()).map_err(|_| invalid("Invalid Responses checkpoint."))
}

impl OpenAiEngineAdapter {
    pub(super) fn validate_durable_request(
        &self,
        request: &ConversationRequest,
        state: Option<&Value>,
    ) -> Result<ConversationBinding, EngineAdapterRequestError> {
        super::super::validate_chat_request(&self.backend, &request.to_request())?;
        if (self.background && self.background_mode != "poll") || !request.shared.chat.stream {
            return Err(EngineAdapterRequestError::new(
                &self.backend.provider,
                "Durable Responses continuation requires foreground streaming or background polling.",
                ErrorKind::InvalidRequest,
            )
            .with_tool_code(ToolCode::UnsupportedToolContinuation));
        }
        if request.shared.chat.context_overflow != EngineContextOverflow::Error {
            return Err(EngineAdapterRequestError::new(
                &self.backend.provider,
                "Durable Responses continuation requires contextOverflow=error.",
                ErrorKind::InvalidRequest,
            ));
        }
        let binding = ConversationBinding {
            origin: self
                .backend
                .binding
                .as_ref()
                .ok_or_else(|| invalid("Missing Responses binding."))?
                .origin(),
            configuration_revision: configuration_digest(
                &serde_json::to_value(&self.backend).expect("safe backend"),
            ),
            timeout_ms: self
                .backend
                .options
                .timeout_ms
                .ok_or_else(|| invalid("Missing Responses timeout."))?
                .min(if self.background {
                    self.background_max_wait_ms
                } else {
                    u64::MAX
                }),
        };
        let original =
            crate::content::messages("openai", &self.backend.provider, &request.shared.messages)?;
        if let Some(state) = state {
            let checkpoint = decode(state)?;
            if checkpoint.version != 1
                || checkpoint.binding != binding
                || checkpoint.request_revision
                    != configuration_digest(&serde_json::to_value(request).expect("request"))
                || !checkpoint.input.starts_with(&original)
            {
                return Err(invalid(
                    "Responses checkpoint does not match its request and binding.",
                ));
            }
            response_events(&self.backend.provider, request, &checkpoint.response, false)?;
        }
        Ok(binding)
    }

    fn continuation_input(
        &self,
        request: &ConversationRequest,
        previous: Option<&Value>,
        results: &[ConversationToolResult],
    ) -> Result<Vec<Value>, EngineAdapterRequestError> {
        let (mut input, calls) = if let Some(state) = previous {
            self.validate_durable_request(request, Some(state))?;
            let checkpoint = decode(state)?;
            let calls = collect_openai_function_calls(&checkpoint.response)?;
            if calls.is_empty() {
                return Err(invalid("Completed Responses turn cannot continue."));
            }
            let mut input = checkpoint.input;
            input.extend(
                checkpoint.response["output"]
                    .as_array()
                    .expect("validated output")
                    .iter()
                    .cloned(),
            );
            (input, calls)
        } else {
            (
                crate::content::messages(
                    "openai",
                    &self.backend.provider,
                    &request.shared.messages,
                )?,
                vec![],
            )
        };
        if calls.len() != results.len()
            || calls.iter().zip(results).any(|(call, result)| {
                call.id != result.call.id
                    || call.name != super::super::local_tool_name(&result.call.name)
                    || call.arguments != result.call.arguments
            })
        {
            return Err(invalid(
                "Responses results differ from the retained call batch.",
            ));
        }
        for result in results {
            input.push(
                json!({"type":"function_call_output", "call_id":result.call.id,
                "output":result.result.provider_content()?.to_string()}),
            );
        }
        check_payload(&input, "openai")?;
        Ok(input)
    }

    pub(super) fn validate_durable_turn(
        &self,
        request: &ConversationRequest,
        previous: Option<&Value>,
        results: &[ConversationToolResult],
        turn: &ConversationTurn,
    ) -> Result<ConversationBinding, EngineAdapterRequestError> {
        check_payload(turn, "openai")?;
        let binding = self.validate_durable_request(request, Some(&turn.state))?;
        let checkpoint = decode(&turn.state)?;
        if checkpoint.input != self.continuation_input(request, previous, results)? {
            return Err(invalid(
                "Responses transcript differs from its predecessor and recorded results.",
            ));
        }
        let calls = collect_openai_function_calls(&checkpoint.response)?;
        if calls.len() != turn.calls.len()
            || calls.iter().zip(&turn.calls).any(|(a, b)| {
                a.id != b.id
                    || a.name != super::super::local_tool_name(&b.name)
                    || a.arguments != b.arguments
            })
        {
            return Err(invalid(
                "Responses calls differ from the retained response.",
            ));
        }
        let expected =
            response_events(&self.backend.provider, request, &checkpoint.response, true)?;
        let mut actual = turn.events.clone();
        for event in &mut actual {
            match event {
                EngineTextStreamEvent::MessageStart { turn_index } => *turn_index = 0,
                EngineTextStreamEvent::TokenUsage { usage } => usage.turn_index = 0,
                EngineTextStreamEvent::ContentBlock {
                    block: block @ ContentBlock::NativeV2 { .. },
                } => {
                    let ContentBlock::NativeV2 { origin, payload } = block else {
                        unreachable!()
                    };
                    if origin != &binding.origin {
                        return Err(invalid("Responses event belongs to another binding."));
                    }
                    *block = ContentBlock::Native {
                        provider: self.backend.provider.clone(),
                        payload: payload.clone(),
                    };
                }
                _ => (),
            }
        }
        if actual != expected {
            return Err(invalid(
                "Responses events differ from retained provider evidence.",
            ));
        }
        Ok(binding)
    }

    pub(super) async fn durable_turn(
        &self,
        request: ConversationRequest,
        state: Option<Value>,
        results: Vec<ConversationToolResult>,
    ) -> Result<ConversationTurn, EngineAdapterRequestError> {
        if self.background {
            return Err(EngineAdapterRequestError::new(
                &self.backend.provider,
                "Background Responses requires the durable step interface.",
                ErrorKind::InvalidRequest,
            )
            .with_tool_code(ToolCode::UnsupportedToolContinuation));
        }
        let binding = self.validate_durable_request(&request, state.as_ref())?;
        let input = self.continuation_input(&request, state.as_ref(), &results)?;
        let runtime = request.to_request();
        let mut body = build_local_tool_body(&self.backend, &runtime, json!(input));
        // Retain opaque reasoning for replay with the host-owned transcript.
        body.as_object_mut()
            .expect("request object")
            .entry("include")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("include array")
            .push(json!("reasoning.encrypted_content"));
        check_payload(&body, "openai")?;
        emit_request(
            self.logger.as_ref(),
            &self.context,
            &request.shared,
            None,
            Some(body.clone()),
        )
        .await;
        let conversation = OpenAiToolConversation {
            http_client: self.http_client.clone(),
            base_url: self.base_url.clone(),
            backend: self.backend.clone(),
            logger: self.logger.clone(),
            context: self.context.clone(),
            input: vec![],
            transport: OpenAiTurnTransport::Stream {
                idle_ms: self.stream_idle_timeout_ms,
            },
        };
        let (tx, mut rx) = mpsc::channel(1);
        let work = conversation.stream_turn(
            &body,
            request.shared.operation,
            &tx,
            self.stream_idle_timeout_ms,
        );
        tokio::pin!(work);
        let response = loop {
            tokio::select! {
                result = &mut work => break result?,
                Some(event) = rx.recv() => { event?; },
            }
        };
        self.completed_turn(&request, binding, input, response, true)
            .await
    }

    async fn completed_turn(
        &self,
        request: &ConversationRequest,
        binding: ConversationBinding,
        input: Vec<Value>,
        response: Value,
        streaming: bool,
    ) -> Result<ConversationTurn, EngineAdapterRequestError> {
        // The completed native response determines durable events; partial stream observations
        // remain diagnostics and cannot disagree with restored output.
        let events = response_events(&self.backend.provider, request, &response, false)?;
        let calls = collect_openai_function_calls(&response)?;
        emit_response(
            self.logger.as_ref(),
            &self.context,
            request.shared.operation,
            200,
            EngineAdapterResponseDetail {
                ok: true,
                request_id: None,
                response_id: response["id"].as_str().map(ToOwned::to_owned),
                output_chars: Some(
                    events
                        .iter()
                        .filter_map(|event| match event {
                            EngineTextStreamEvent::TextDelta { text } => Some(text.chars().count()),
                            _ => None,
                        })
                        .sum(),
                ),
                usage: response.get("usage").cloned(),
                streaming: Some(streaming),
                stop_reason: None,
            },
        )
        .await;
        let checkpoint = Checkpoint {
            version: 1,
            binding,
            request_revision: configuration_digest(
                &serde_json::to_value(request).expect("request"),
            ),
            input,
            response,
        };
        let turn = ConversationTurn {
            state: serde_json::to_value(checkpoint).expect("checkpoint"),
            calls,
            events,
        };
        check_payload(&turn, "openai")?;
        Ok(turn)
    }
}

/// Project completed native evidence in a fixed order. The registry adds final structured output.
fn response_events(
    provider: &str,
    request: &ConversationRequest,
    response: &Value,
    structured: bool,
) -> Result<Vec<EngineTextStreamEvent>, EngineAdapterRequestError> {
    // Recovery admits the same native schema as the live stream without reserialising it.
    serde_json::from_value::<async_openai::types::responses::Response>(response.clone())
        .map_err(|_| invalid("Invalid native Responses response."))?;
    if response["status"] != "completed"
        || !response["error"].is_null()
        || !response["incomplete_details"].is_null()
    {
        return Err(invalid("Responses turn is not successfully completed."));
    }
    let output = response["output"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| invalid("Responses turn has no output items."))?;
    let mut text = String::new();
    let mut reasoning = String::new();
    for item in output {
        if !item["status"].is_null() && item["status"] != "completed" {
            return Err(invalid("Responses output item is incomplete."));
        }
        match item["type"].as_str() {
            Some("message") if item["role"] == "assistant" => {
                for block in item["content"]
                    .as_array()
                    .ok_or_else(|| invalid("Invalid Responses message content."))?
                {
                    if block["type"] != "output_text" {
                        return Err(invalid(
                            "Responses output was refused or contains unsupported content.",
                        ));
                    }
                    text.push_str(
                        block["text"]
                            .as_str()
                            .ok_or_else(|| invalid("Invalid Responses text."))?,
                    );
                }
            }
            Some("reasoning") => {
                for block in item["summary"]
                    .as_array()
                    .ok_or_else(|| invalid("Invalid Responses reasoning summary."))?
                {
                    if block["type"] != "summary_text" {
                        return Err(invalid("Unsupported Responses reasoning summary."));
                    }
                    reasoning.push_str(
                        block["text"]
                            .as_str()
                            .ok_or_else(|| invalid("Invalid Responses reasoning text."))?,
                    );
                }
            }
            Some("function_call") if item["arguments"].is_string() => (),
            _ => return Err(invalid("Unsupported Responses output item.")),
        }
    }
    if !response["output_text"].is_null() && response["output_text"] != text {
        return Err(invalid(
            "Responses text disagrees with its native output items.",
        ));
    }
    let calls = collect_openai_function_calls(response)?;
    super::super::tool_loop::validate_tool_calls(provider, &request.to_request(), &calls)?;
    if calls.is_empty() && text.trim().is_empty() {
        return Err(invalid("Responses final turn has no output text."));
    }
    let mut events = vec![EngineTextStreamEvent::MessageStart { turn_index: 0 }];
    if !reasoning.is_empty() {
        events.push(EngineTextStreamEvent::ReasoningDelta { text: reasoning });
    }
    for item in output {
        events.push(EngineTextStreamEvent::ContentBlock {
            block: ContentBlock::Native {
                provider: provider.into(),
                payload: item.clone(),
            },
        });
    }
    if let Some(usage) = extract_token_usage(response) {
        events.push(EngineTextStreamEvent::TokenUsage { usage });
    }
    if !text.is_empty() {
        events.push(EngineTextStreamEvent::TextDelta { text: text.clone() });
    }
    if calls.is_empty() {
        if let Some(format) = &request.shared.chat.response_format {
            let value = format.parse(provider, &text)?;
            if structured {
                events.push(EngineTextStreamEvent::StructuredOutput { value });
            }
        }
    }
    events.push(EngineTextStreamEvent::MessageStop {
        reason: if calls.is_empty() {
            StopReason::Complete
        } else {
            StopReason::ToolUse
        },
    });
    Ok(events)
}
