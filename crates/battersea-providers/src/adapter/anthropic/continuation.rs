//! Copyright (c) Scott A Dixon
//! Anthropic checkpoints retain the native transcript and derive durable observations.
use super::*;
use battersea_model::{
    adapter::payload::check_payload, configuration_digest, engine::EngineContextOverflow,
    ContentBlock, ConversationBinding, ConversationRequest, ConversationToolResult,
    ConversationTurn, ErrorKind, StopReason,
};

#[derive(serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: u32,
    binding: ConversationBinding,
    request_revision: String,
    /// Exact messages sent for this turn, including the preceding tool-result projections.
    messages: Vec<Value>,
    /// Native Messages response for this turn.
    response: Value,
}

const CHECKPOINT_VERSION: u32 = 1;

fn invalid(message: &str) -> EngineAdapterRequestError {
    EngineAdapterRequestError::invalid_response("anthropic", message)
}

fn decode(state: &Value) -> Result<Checkpoint, EngineAdapterRequestError> {
    check_payload(state, "anthropic")?;
    serde_json::from_value(state.clone()).map_err(|_| invalid("Invalid Anthropic checkpoint."))
}

fn request_revision(request: &ConversationRequest) -> String {
    configuration_digest(&serde_json::to_value(request).expect("request"))
}

fn same_calls<'a>(
    retained: &[EngineLocalToolCall],
    other: impl ExactSizeIterator<Item = &'a EngineLocalToolCall>,
) -> bool {
    retained.len() == other.len()
        && retained.iter().zip(other).all(|(a, b)| {
            a.id == b.id
                && a.name == super::super::local_tool_name(&b.name)
                && a.arguments == b.arguments
        })
}

/// Classify one native assistant block. Durable turns admit text, signed thinking, redacted
/// thinking and client tool use; every other field of an admitted block is retained verbatim.
fn validate_block(block: &Value) -> Result<(), EngineAdapterRequestError> {
    let valid = match anthropic_content_block_type(block) {
        Some("text") => block["text"].is_string(),
        Some("thinking") => block["thinking"].is_string() && block["signature"].is_string(),
        Some("redacted_thinking") => block["data"].is_string(),
        Some("tool_use") => {
            block["id"].is_string() && block["name"].is_string() && block["input"].is_object()
        }
        _ => return Err(invalid("Unsupported Anthropic content block.")),
    };
    if valid {
        Ok(())
    } else {
        Err(invalid("Malformed Anthropic content block."))
    }
}

/// Validate a retained assistant turn and return its tool calls.
fn assistant_calls(
    request: &ConversationRequest,
    content: &Value,
) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
    let blocks = content
        .as_array()
        .filter(|blocks| !blocks.is_empty())
        .ok_or_else(|| invalid("Anthropic turn has no content blocks."))?;
    for block in blocks {
        validate_block(block)?;
    }
    let calls = collect_anthropic_tool_calls(blocks)?;
    super::super::tool_loop::validate_tool_calls("anthropic", &request.to_request(), &calls)?;
    Ok(calls)
}

fn tool_result_message(
    results: &[ConversationToolResult],
) -> Result<Value, EngineAdapterRequestError> {
    let content = results
        .iter()
        .map(|item| {
            Ok(json!({"type": "tool_result", "tool_use_id": item.call.id,
                "content": item.result.provider_content()?.to_string(),
                "is_error": item.result.is_error()}))
        })
        .collect::<Result<Vec<_>, EngineAdapterRequestError>>()?;
    Ok(json!({"role": "user", "content": content}))
}

impl AnthropicEngineAdapter {
    pub(super) fn validate_durable_request(
        &self,
        request: &ConversationRequest,
        state: Option<&Value>,
    ) -> Result<ConversationBinding, EngineAdapterRequestError> {
        super::super::validate_chat_request(&self.backend, &request.to_request())?;
        require_anthropic_max_tokens(&request.shared.chat)?;
        if request.shared.chat.context_overflow != EngineContextOverflow::Error {
            return Err(EngineAdapterRequestError::new(
                "anthropic",
                "Durable Anthropic continuation requires contextOverflow=error.",
                ErrorKind::InvalidRequest,
            ));
        }
        let binding = ConversationBinding {
            origin: self
                .backend
                .binding
                .as_ref()
                .ok_or_else(|| invalid("Missing Anthropic binding."))?
                .origin(),
            configuration_revision: configuration_digest(
                &serde_json::to_value(&self.backend).expect("safe backend"),
            ),
            timeout_ms: self
                .backend
                .options
                .timeout_ms
                .ok_or_else(|| invalid("Missing Anthropic timeout."))?,
        };
        let Some(state) = state else {
            return Ok(binding);
        };
        let original = crate::content::messages(
            "anthropic",
            &self.backend.provider,
            &request.shared.messages,
        )?;
        let checkpoint = decode(state)?;
        if checkpoint.version != CHECKPOINT_VERSION
            || checkpoint.binding != binding
            || checkpoint.request_revision != request_revision(request)
            || !checkpoint.messages.starts_with(&original)
            || (checkpoint.messages.len() - original.len()) % 2 != 0
        {
            return Err(invalid(
                "Anthropic checkpoint does not match its request and binding.",
            ));
        }
        // Each preceding turn is a tool-use assistant message answered by its ordered results.
        for pair in checkpoint.messages[original.len()..].chunks(2) {
            let (assistant, answer) = (&pair[0], &pair[1]);
            if assistant["role"] != "assistant" || answer["role"] != "user" {
                return Err(invalid("Invalid Anthropic transcript order."));
            }
            let calls = assistant_calls(request, &assistant["content"])?;
            let results = answer["content"].as_array();
            if calls.is_empty()
                || results.is_none_or(|results| {
                    results.len() != calls.len()
                        || results.iter().zip(&calls).any(|(result, call)| {
                            result["type"] != "tool_result"
                                || result["tool_use_id"] != call.id.as_str()
                                || !result["content"].is_string()
                                || !result["is_error"].is_boolean()
                        })
                })
            {
                return Err(invalid("Invalid retained Anthropic tool result."));
            }
        }
        response_events(request, &checkpoint.response, false)?;
        Ok(binding)
    }

    fn continuation_messages(
        &self,
        request: &ConversationRequest,
        previous: Option<&Value>,
        results: &[ConversationToolResult],
    ) -> Result<Vec<Value>, EngineAdapterRequestError> {
        let (mut messages, calls) = if let Some(state) = previous {
            self.validate_durable_request(request, Some(state))?;
            let checkpoint = decode(state)?;
            let calls = assistant_calls(request, &checkpoint.response["content"])?;
            if calls.is_empty() {
                return Err(invalid("Completed Anthropic turn cannot continue."));
            }
            let mut messages = checkpoint.messages;
            // Signed thinking and every other native block return unchanged.
            messages.push(json!({"role": "assistant", "content": checkpoint.response["content"]}));
            (messages, calls)
        } else {
            let messages = crate::content::messages(
                "anthropic",
                &self.backend.provider,
                &request.shared.messages,
            )?;
            (messages, vec![])
        };
        if !same_calls(&calls, results.iter().map(|result| &result.call)) {
            return Err(invalid(
                "Anthropic results differ from the retained call batch.",
            ));
        }
        if !results.is_empty() {
            messages.push(tool_result_message(results)?);
        }
        check_payload(&messages, "anthropic")?;
        Ok(messages)
    }

    pub(super) fn validate_durable_turn(
        &self,
        request: &ConversationRequest,
        previous: Option<&Value>,
        results: &[ConversationToolResult],
        turn: &ConversationTurn,
    ) -> Result<ConversationBinding, EngineAdapterRequestError> {
        check_payload(turn, "anthropic")?;
        let binding = self.validate_durable_request(request, Some(&turn.state))?;
        let checkpoint = decode(&turn.state)?;
        if checkpoint.messages != self.continuation_messages(request, previous, results)? {
            return Err(invalid(
                "Anthropic transcript differs from its predecessor and recorded results.",
            ));
        }
        let calls = assistant_calls(request, &checkpoint.response["content"])?;
        if !same_calls(&calls, turn.calls.iter()) {
            return Err(invalid("Anthropic calls differ from the retained message."));
        }
        let expected = response_events(request, &checkpoint.response, true)?;
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
                        return Err(invalid("Anthropic event belongs to another binding."));
                    }
                    *block = ContentBlock::Native {
                        provider: "anthropic".into(),
                        payload: payload.clone(),
                    };
                }
                _ => (),
            }
        }
        if actual != expected {
            return Err(invalid(
                "Anthropic events differ from retained provider evidence.",
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
        let binding = self.validate_durable_request(&request, state.as_ref())?;
        let messages = self.continuation_messages(&request, state.as_ref(), &results)?;
        let body = build_local_tool_body(&self.backend, &request.to_request(), messages.clone())?;
        check_payload(&body, "anthropic")?;
        emit_request(
            self.logger.as_ref(),
            &self.context,
            &request.shared,
            None,
            Some(body.clone()),
        )
        .await;
        let response = crate::retry::send(
            self.client.post(&self.backend.endpoint).json(&body),
            &self.backend,
            self.logger.as_ref(),
        )
        .await?;
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let native: Value =
            read_anthropic_json(response, request_id.clone(), &self.context).await?;
        // The completed native message determines durable events and calls.
        let events = response_events(&request, &native, false)
            .map_err(|error| error.with_request_id(request_id.clone()))?;
        let calls = assistant_calls(&request, &native["content"])?;
        emit_response(
            self.logger.as_ref(),
            &self.context,
            request.shared.operation,
            status,
            EngineAdapterResponseDetail {
                ok: true,
                request_id,
                response_id: native["id"].as_str().map(ToOwned::to_owned),
                output_chars: Some(
                    events
                        .iter()
                        .filter_map(|event| match event {
                            EngineTextStreamEvent::TextDelta { text } => Some(text.chars().count()),
                            _ => None,
                        })
                        .sum(),
                ),
                usage: native.get("usage").cloned(),
                streaming: Some(false),
                stop_reason: native["stop_reason"].as_str().map(ToOwned::to_owned),
            },
        )
        .await;
        let checkpoint = Checkpoint {
            version: CHECKPOINT_VERSION,
            binding,
            request_revision: request_revision(&request),
            messages,
            response: native,
        };
        let turn = ConversationTurn {
            state: serde_json::to_value(checkpoint).expect("checkpoint"),
            calls,
            events,
        };
        check_payload(&turn, "anthropic")?;
        Ok(turn)
    }
}

/// Project completed native evidence in a fixed order. The registry adds final structured output.
fn response_events(
    request: &ConversationRequest,
    response: &Value,
    structured: bool,
) -> Result<Vec<EngineTextStreamEvent>, EngineAdapterRequestError> {
    if response["type"] != "message"
        || response["role"] != "assistant"
        || response["id"].as_str().is_none_or(|id| id.is_empty())
    {
        return Err(invalid("Anthropic turn lacks its message identity."));
    }
    let stop = match response["stop_reason"].as_str() {
        Some("end_turn" | "stop_sequence") => StopReason::Complete,
        Some("tool_use") => StopReason::ToolUse,
        _ => return Err(invalid("Anthropic turn is not successfully completed.")),
    };
    let calls = assistant_calls(request, &response["content"])?;
    if calls.is_empty() == matches!(stop, StopReason::ToolUse) {
        return Err(invalid(
            "Anthropic stop reason disagrees with its tool calls.",
        ));
    }
    let blocks = response["content"].as_array().expect("validated content");
    let text = blocks
        .iter()
        .filter_map(anthropic_content_block_text)
        .collect::<String>();
    let reasoning = blocks
        .iter()
        .filter_map(anthropic_content_block_reasoning)
        .collect::<String>();
    if calls.is_empty() && text.trim().is_empty() {
        return Err(invalid("Anthropic final turn has no output text."));
    }
    let usage = match response.get("usage") {
        None | Some(Value::Null) => None,
        Some(usage @ Value::Object(fields)) => {
            if fields
                .iter()
                .filter(|(name, _)| name.ends_with("_tokens"))
                .any(|(_, value)| !value.is_null() && !value.is_u64())
            {
                return Err(invalid("Invalid Anthropic usage."));
            }
            parse_anthropic_usage(usage)
        }
        Some(_) => return Err(invalid("Invalid Anthropic usage.")),
    };
    let mut events = vec![EngineTextStreamEvent::MessageStart { turn_index: 0 }];
    if !reasoning.is_empty() {
        events.push(EngineTextStreamEvent::ReasoningDelta { text: reasoning });
    }
    for block in blocks {
        events.push(EngineTextStreamEvent::ContentBlock {
            block: ContentBlock::Native {
                provider: "anthropic".into(),
                payload: block.clone(),
            },
        });
    }
    if let Some(usage) = usage {
        events.push(EngineTextStreamEvent::TokenUsage { usage });
    }
    if !text.is_empty() {
        events.push(EngineTextStreamEvent::TextDelta { text: text.clone() });
    }
    if calls.is_empty() {
        if let Some(format) = &request.shared.chat.response_format {
            let value = format.parse("anthropic", &text)?;
            if structured {
                events.push(EngineTextStreamEvent::StructuredOutput { value });
            }
        }
    }
    events.push(EngineTextStreamEvent::MessageStop { reason: stop });
    Ok(events)
}
