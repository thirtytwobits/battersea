//! Copyright (c) Scott A Dixon
//! Gemini checkpoints retain the native transcript and derive durable observations.
use super::*;
use battersea_model::{
    adapter::payload::check_payload, configuration_digest, engine::EngineContextOverflow,
    ContentBlock, ConversationBinding, ConversationRequest, ConversationToolResult,
    ConversationTurn, ErrorKind, Modality, StopReason,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: u32,
    binding: ConversationBinding,
    request_revision: String,
    /// Exact contents sent for this turn, including the preceding function responses.
    contents: Vec<Value>,
    /// Native `generateContent` response for this turn.
    response: Value,
}

const CHECKPOINT_VERSION: u32 = 1;

/// Part fields that carry data. Every other field of an admitted part is retained verbatim.
const DATA_FIELDS: &[&str] = &[
    "text",
    "functionCall",
    "functionResponse",
    "inlineData",
    "fileData",
    "executableCode",
    "codeExecutionResult",
];

fn invalid(message: &str) -> EngineAdapterRequestError {
    EngineAdapterRequestError::invalid_response("google", message)
}

fn decode(state: &Value) -> Result<Checkpoint, EngineAdapterRequestError> {
    check_payload(state, "google")?;
    serde_json::from_value(state.clone()).map_err(|_| invalid("Invalid Gemini checkpoint."))
}

fn request_revision(request: &ConversationRequest) -> String {
    configuration_digest(&serde_json::to_value(request).expect("request"))
}

/// The number of contents the request's own messages project to; model turns follow them.
fn original_len(
    backend: &EngineBackendConfig,
    request: &ConversationRequest,
) -> Result<usize, EngineAdapterRequestError> {
    Ok(crate::content::messages("google", &backend.provider, &request.shared.messages)?.len())
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

/// Durable turns admit text, thought text and function-call parts.
fn validate_part(part: &Value) -> Result<(), EngineAdapterRequestError> {
    let fields = part
        .as_object()
        .ok_or_else(|| invalid("Malformed Gemini part."))?;
    let data = DATA_FIELDS
        .iter()
        .filter(|field| fields.contains_key(**field))
        .collect::<Vec<_>>();
    let valid = match data.as_slice() {
        [&"text"] => {
            part["text"].is_string() && (part["thought"].is_null() || part["thought"].is_boolean())
        }
        [&"functionCall"] => {
            let call = &part["functionCall"];
            call["name"].is_string()
                && (call["args"].is_null() || call["args"].is_object())
                && (call["id"].is_null() || call["id"].is_string())
        }
        [] | [_] => return Err(invalid("Unsupported Gemini part.")),
        _ => false,
    };
    if valid && (part["thoughtSignature"].is_null() || part["thoughtSignature"].is_string()) {
        Ok(())
    } else {
        Err(invalid("Malformed Gemini part."))
    }
}

/// Validate one retained model content and return its tool calls. `turn` counts the model
/// contents that precede it, so a generated correlation ID is unique across the conversation.
fn model_calls(
    request: &ConversationRequest,
    content: &Value,
    turn: usize,
) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
    let parts = content["parts"]
        .as_array()
        .filter(|parts| !parts.is_empty() && content["role"] == "model")
        .ok_or_else(|| invalid("Gemini turn has no model content."))?;
    for part in parts {
        validate_part(part)?;
    }
    let payload = GeminiGenerateContentResponse {
        candidates: vec![GeminiCandidate {
            content: Some(
                serde_json::from_value(content.clone())
                    .map_err(|_| invalid("Malformed Gemini content."))?,
            ),
            finish_reason: None,
        }],
        ..Default::default()
    };
    let calls = collect_google_function_calls(&payload, turn)?;
    super::super::tool_loop::validate_tool_calls("google", &request.to_request(), &calls)?;
    Ok(calls)
}

/// The function-call parts of a validated model content, in call order.
fn call_parts(content: &Value) -> Vec<&Value> {
    content["parts"]
        .as_array()
        .expect("validated content")
        .iter()
        .filter_map(|part| part.get("functionCall"))
        .collect()
}

/// Project results as the live tool loop does: the native name, and the ID only when the
/// provider assigned one.
fn function_responses(
    content: &Value,
    results: &[ConversationToolResult],
) -> Result<Value, EngineAdapterRequestError> {
    let parts = call_parts(content)
        .into_iter()
        .zip(results)
        .map(|(call, item)| {
            let response = match item.result.provider_content()? {
                value @ Value::Object(_) => value,
                value => json!({"result": value}),
            };
            let mut function_response = json!({"name": call["name"], "response": response});
            if let Some(id) = call.get("id") {
                function_response["id"] = id.clone();
            }
            Ok(json!({"functionResponse": function_response}))
        })
        .collect::<Result<Vec<_>, EngineAdapterRequestError>>()?;
    Ok(json!({"role": "user", "parts": parts}))
}

impl GoogleEngineAdapter {
    pub(super) fn validate_durable_request(
        &self,
        request: &ConversationRequest,
        state: Option<&Value>,
    ) -> Result<ConversationBinding, EngineAdapterRequestError> {
        super::super::validate_chat_request(&self.backend, &request.to_request())?;
        if request.shared.chat.context_overflow != EngineContextOverflow::Error {
            return Err(EngineAdapterRequestError::new(
                "google",
                "Durable Gemini continuation requires contextOverflow=error.",
                ErrorKind::InvalidRequest,
            ));
        }
        if request
            .shared
            .chat
            .output_modalities
            .as_ref()
            .is_some_and(|modalities| modalities.as_slice() != [Modality::Text])
        {
            return Err(EngineAdapterRequestError::new(
                "google",
                "Durable Gemini continuation requires text output.",
                ErrorKind::InvalidRequest,
            ));
        }
        let binding = ConversationBinding {
            origin: self
                .backend
                .binding
                .as_ref()
                .ok_or_else(|| invalid("Missing Gemini binding."))?
                .origin(),
            configuration_revision: configuration_digest(
                &serde_json::to_value(&self.backend).expect("safe backend"),
            ),
            timeout_ms: self
                .backend
                .options
                .timeout_ms
                .ok_or_else(|| invalid("Missing Gemini timeout."))?,
        };
        let Some(state) = state else {
            return Ok(binding);
        };
        let original =
            crate::content::messages("google", &self.backend.provider, &request.shared.messages)?;
        let checkpoint = decode(state)?;
        if checkpoint.version != CHECKPOINT_VERSION
            || checkpoint.binding != binding
            || checkpoint.request_revision != request_revision(request)
            || !checkpoint.contents.starts_with(&original)
            || (checkpoint.contents.len() - original.len()) % 2 != 0
        {
            return Err(invalid(
                "Gemini checkpoint does not match its request and binding.",
            ));
        }
        // Each preceding turn is a function-calling model content answered by its ordered results.
        for (turn, pair) in checkpoint.contents[original.len()..].chunks(2).enumerate() {
            let (model, answer) = (&pair[0], &pair[1]);
            if model_calls(request, model, turn)?.is_empty() || answer["role"] != "user" {
                return Err(invalid("Invalid Gemini transcript order."));
            }
            let calls = call_parts(model);
            let responses = answer["parts"].as_array();
            if responses.is_none_or(|responses| {
                responses.len() != calls.len()
                    || responses.iter().zip(&calls).any(|(part, call)| {
                        let response = &part["functionResponse"];
                        part.as_object().is_none_or(|part| part.len() != 1)
                            || response["name"] != call["name"]
                            || response.get("id") != call.get("id")
                            || !response["response"].is_object()
                    })
            }) {
                return Err(invalid("Invalid retained Gemini function response."));
            }
        }
        response_events(request, &checkpoint.response, false)?;
        Ok(binding)
    }

    fn continuation_contents(
        &self,
        request: &ConversationRequest,
        previous: Option<&Value>,
        results: &[ConversationToolResult],
    ) -> Result<Vec<Value>, EngineAdapterRequestError> {
        let (mut contents, calls, model) = if let Some(state) = previous {
            self.validate_durable_request(request, Some(state))?;
            let checkpoint = decode(state)?;
            let model = checkpoint.response["candidates"][0]["content"].clone();
            let turn = (checkpoint.contents.len() - original_len(&self.backend, request)?) / 2;
            let calls = model_calls(request, &model, turn)?;
            if calls.is_empty() {
                return Err(invalid("Completed Gemini turn cannot continue."));
            }
            let mut contents = checkpoint.contents;
            // Thought signatures and every other native part field return unchanged.
            contents.push(model.clone());
            (contents, calls, model)
        } else {
            let contents = crate::content::messages(
                "google",
                &self.backend.provider,
                &request.shared.messages,
            )?;
            (contents, vec![], Value::Null)
        };
        if !same_calls(&calls, results.iter().map(|result| &result.call)) {
            return Err(invalid(
                "Gemini results differ from the retained call batch.",
            ));
        }
        if !results.is_empty() {
            contents.push(function_responses(&model, results)?);
        }
        check_payload(&contents, "google")?;
        Ok(contents)
    }

    pub(super) fn validate_durable_turn(
        &self,
        request: &ConversationRequest,
        previous: Option<&Value>,
        results: &[ConversationToolResult],
        turn: &ConversationTurn,
    ) -> Result<ConversationBinding, EngineAdapterRequestError> {
        check_payload(turn, "google")?;
        let binding = self.validate_durable_request(request, Some(&turn.state))?;
        let checkpoint = decode(&turn.state)?;
        if checkpoint.contents != self.continuation_contents(request, previous, results)? {
            return Err(invalid(
                "Gemini transcript differs from its predecessor and recorded results.",
            ));
        }
        let index = (checkpoint.contents.len() - original_len(&self.backend, request)?) / 2;
        let calls = model_calls(
            request,
            &checkpoint.response["candidates"][0]["content"],
            index,
        )?;
        if !same_calls(&calls, turn.calls.iter()) {
            return Err(invalid("Gemini calls differ from the retained response."));
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
                        return Err(invalid("Gemini event belongs to another binding."));
                    }
                    *block = ContentBlock::Native {
                        provider: "google".into(),
                        payload: payload.clone(),
                    };
                }
                _ => (),
            }
        }
        if actual != expected {
            return Err(invalid(
                "Gemini events differ from retained provider evidence.",
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
        let contents = self.continuation_contents(&request, state.as_ref(), &results)?;
        let body = build_local_tool_body(&self.backend, &request.to_request(), contents.clone());
        check_payload(&body, "google")?;
        emit_request(
            self.logger.as_ref(),
            &self.context,
            &request.shared,
            None,
            Some(body.clone()),
        )
        .await;
        let response = crate::retry::send(
            self.client
                .post(self.build_request_url("generateContent", false))
                .json(&body),
            &self.backend,
            self.logger.as_ref(),
            self.clock.as_ref(),
        )
        .await?;
        let status = response.status().as_u16();
        let native: Value = response
            .bounded_json()
            .await
            .map_err(|error| invalid(&error.to_string()))?;
        // The completed native response determines durable events and calls.
        let events = response_events(&request, &native, false)?;
        let index = (contents.len() - original_len(&self.backend, &request)?) / 2;
        let calls = model_calls(&request, &native["candidates"][0]["content"], index)?;
        emit_response(
            self.logger.as_ref(),
            &self.context,
            request.shared.operation,
            status,
            EngineAdapterResponseDetail {
                ok: true,
                request_id: None,
                response_id: native["responseId"].as_str().map(ToOwned::to_owned),
                output_chars: Some(
                    events
                        .iter()
                        .filter_map(|event| match event {
                            EngineTextStreamEvent::TextDelta { text } => Some(text.chars().count()),
                            _ => None,
                        })
                        .sum(),
                ),
                usage: native.get("usageMetadata").cloned(),
                streaming: Some(false),
                stop_reason: native["candidates"][0]["finishReason"]
                    .as_str()
                    .map(ToOwned::to_owned),
            },
        )
        .await;
        let checkpoint = Checkpoint {
            version: CHECKPOINT_VERSION,
            binding,
            request_revision: request_revision(&request),
            contents,
            response: native,
        };
        let turn = ConversationTurn {
            state: serde_json::to_value(checkpoint).expect("checkpoint"),
            calls,
            events,
        };
        check_payload(&turn, "google")?;
        Ok(turn)
    }
}

/// Project completed native evidence in a fixed order. The registry adds final structured output.
fn response_events(
    request: &ConversationRequest,
    response: &Value,
    structured: bool,
) -> Result<Vec<EngineTextStreamEvent>, EngineAdapterRequestError> {
    let parsed: GeminiGenerateContentResponse = serde_json::from_value(response.clone())
        .map_err(|_| invalid("Invalid native Gemini response."))?;
    if !response["promptFeedback"]["blockReason"].is_null() {
        return Err(invalid("Gemini blocked the request."));
    }
    let [candidate] = response["candidates"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
    else {
        return Err(invalid("Gemini turn requires exactly one candidate."));
    };
    if candidate["finishReason"] != "STOP" {
        return Err(invalid("Gemini turn is not successfully completed."));
    }
    // Only whether the turn calls tools matters here, not the calls' correlation IDs.
    let calls = model_calls(request, &candidate["content"], 0)?;
    let parts = candidate["content"]["parts"]
        .as_array()
        .expect("validated content");
    let (mut text, mut reasoning) = (String::new(), String::new());
    for part in parts {
        if let Some(part_text) = part["text"].as_str() {
            if part["thought"] == true {
                reasoning.push_str(part_text);
            } else {
                text.push_str(part_text);
            }
        }
    }
    if calls.is_empty() && text.trim().is_empty() {
        return Err(invalid("Gemini final turn has no output text."));
    }
    match response.get("usageMetadata") {
        None | Some(Value::Null) => (),
        Some(Value::Object(fields))
            if fields
                .iter()
                .filter(|(name, _)| name.ends_with("TokenCount"))
                .all(|(_, value)| value.is_null() || value.is_u64()) => {}
        Some(_) => return Err(invalid("Invalid Gemini usage.")),
    }
    let mut events = vec![EngineTextStreamEvent::MessageStart { turn_index: 0 }];
    if !reasoning.is_empty() {
        events.push(EngineTextStreamEvent::ReasoningDelta { text: reasoning });
    }
    for part in parts {
        events.push(EngineTextStreamEvent::ContentBlock {
            block: ContentBlock::Native {
                provider: "google".into(),
                payload: part.clone(),
            },
        });
    }
    if let Some(usage) = parsed.token_usage() {
        events.push(EngineTextStreamEvent::TokenUsage { usage });
    }
    if !text.is_empty() {
        events.push(EngineTextStreamEvent::TextDelta { text: text.clone() });
    }
    if calls.is_empty() {
        if let Some(format) = &request.shared.chat.response_format {
            let value = format.parse("google", &text)?;
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
