//! Google Gemini adapter — translates the engine's typed chat
//! requests into Gemini REST API calls and decodes the responses
//! back into provider-neutral engine events.
//!
//! Architecture mirrors the Anthropic adapter (raw `reqwest` plus
//! manual SSE parsing via `eventsource-stream`) rather than the
//! OpenAI adapter (which leans on `async-openai`). Gemini has no
//! mature first-party Rust SDK we want to vendor; the REST API is
//! straightforward enough to drive directly.
//!
//! Endpoints used:
//!   POST {base}/v1beta/models/{model}:generateContent?key={api_key}
//!   POST {base}/v1beta/models/{model}:streamGenerateContent?alt=sse&key={api_key}
//!   POST {base}/v1beta/models/{model}:countTokens?key={api_key}
//!
//! Auth: API key in the `key=` query parameter. Gemini does not use
//! a bearer token for the `generativelanguage.googleapis.com` host
//! the same way OpenAI does. The engine config carries the key via
//! `apiKeyEnv`; the adapter splices it into the URL per request.
//!

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
    EngineChatParameters, EngineChatToolChoiceMode, EngineThinkingVisibility,
};
use eventsource_stream::Eventsource;
use futures_util::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// Test-only endpoint literal. Production reads the Gemini host from the
/// backend's configured `endpoint`, which is validated non-blank at
/// construction; there is no production host fallback.
#[cfg(test)]
const DEFAULT_GEMINI_HOST: &str = "https://generativelanguage.googleapis.com";

/// Builds a Google Gemini text adapter from backend config. Resolves
/// the API key from the configured env var up front so authentication
/// failures surface at construction time rather than on the first
/// request.
pub(crate) fn create_google_adapter(
    backend: EngineBackendConfig,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
) -> Result<Arc<dyn EngineAdapter>, EngineAdapterRequestError> {
    let provider = "google";
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
    if backend.endpoint.trim().is_empty() {
        return Err(missing_backend_option(provider, "endpoint"));
    }
    let base_url = resolve_google_base_url(&backend.endpoint);
    let api_key = resolve_google_api_key(&backend)?;
    let model = backend.model.clone();
    let backend_id = backend.id.clone();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
        .map_err(|error| EngineAdapterRequestError::transport("google", error.to_string()))?;
    Ok(Arc::new(GoogleEngineAdapter {
        client,
        logger,
        backend,
        api_key,
        base_url: base_url.clone(),
        stream_idle_timeout_ms,
        context: EngineAdapterDebugContext {
            provider: "google".to_string(),
            backend: backend_id,
            model,
            sdk: "reqwest".to_string(),
            base_url: Some(base_url),
            timeout_ms: Some(timeout_ms),
            max_retries: Some(max_retries),
        },
    }))
}

struct GoogleEngineAdapter {
    client: reqwest::Client,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
    backend: EngineBackendConfig,
    api_key: String,
    base_url: String,
    stream_idle_timeout_ms: u64,
    context: EngineAdapterDebugContext,
}

#[async_trait]
impl EngineAdapter for GoogleEngineAdapter {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        self.context.clone()
    }

    async fn count_text_stream_input_tokens(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<u64, EngineAdapterRequestError> {
        super::validate_chat_request(&self.backend, &request)?;
        let body = build_count_tokens_body(&request);
        let url = self.build_request_url("countTokens", false);
        let response = self
            .client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|error| EngineAdapterRequestError::transport("google", error.to_string()))?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            return Err(normalize_google_http_error(
                status,
                response.bounded_text().await.ok(),
            ));
        }
        let payload: Value = response.bounded_json().await.map_err(|error| {
            EngineAdapterRequestError::invalid_response("google", error.to_string())
        })?;
        payload
            .get("totalTokens")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                EngineAdapterRequestError::invalid_response(
                    "google",
                    "Gemini countTokens response missing totalTokens.".to_string(),
                )
            })
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
        battersea_model::adapter::payload::check_payload(&body, "google")?;
        emit_request(
            self.logger.as_ref(),
            &self.context,
            &request.shared,
            None,
            Some(body.clone()),
        )
        .await;

        let url = self.build_request_url("streamGenerateContent", true);
        let response = self
            .client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|error| EngineAdapterRequestError::transport("google", error.to_string()))?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            return Err(normalize_google_http_error(
                status,
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
        let stream = crate::http_payload::bounded_stream(response)
            .eventsource()
            .map(move |event| {
                let logger = logger.clone();
                let context = context.clone();
                async move {
                    match event {
                        Ok(event) => {
                            // Gemini's `:streamGenerateContent?alt=sse`
                            // closes the stream by emitting nothing
                            // further, not by sending a sentinel event.
                            // Empty-data ticks are heartbeats we drop.
                            if event.data.trim().is_empty() {
                                return Ok(Vec::new());
                            }
                            let parsed: GeminiGenerateContentResponse =
                                serde_json::from_str(&event.data).map_err(|error| {
                                    EngineAdapterRequestError::invalid_response(
                                        "google",
                                        format!("Gemini stream chunk was not valid JSON: {error}"),
                                    )
                                })?;
                            emit_stream_event(
                                logger.as_ref(),
                                &context,
                                operation,
                                "stream-chunk",
                                serde_json::to_value(&parsed).unwrap_or_else(|_| json!({})),
                            )
                            .await;
                            Ok(parsed.into_text_stream_events())
                        }
                        Err(error) => Err(EngineAdapterRequestError::transport(
                            "google",
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
            "google",
        ))
    }
}

impl GoogleEngineAdapter {
    /// Composes the per-request URL: `{base}/v1beta/models/{model}:{action}?key={api_key}[&alt=sse]`.
    /// Splitting this out keeps the four call sites consistent and
    /// makes it easy to swap in a different API version later.
    fn build_request_url(&self, action: &str, sse: bool) -> String {
        let suffix = if sse { "&alt=sse" } else { "" };
        format!(
            "{}/v1beta/models/{}:{}?key={}{}",
            self.base_url, self.backend.model, action, self.api_key, suffix
        )
    }

    /// Runs the shared tool runner with non-streaming Gemini turns. Each
    /// completed turn emits its text, reasoning and usage before tool execution.
    async fn stream_text_with_local_tools(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        let executor = request.local_tool_executor.clone().ok_or_else(|| {
            EngineAdapterRequestError::new(
                "google",
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
        let url = self.build_request_url("generateContent", false);

        tokio::spawn(async move {
            let conversation = GoogleToolConversation {
                client,
                backend,
                url,
                logger: logger.clone(),
                context: context.clone(),
                contents: super::google_messages(&request.shared),
            };
            run_tool_loop(conversation, request, executor, logger, context, tx).await;
        });

        // NOTE: no `with_stream_idle_watchdog` here, unlike the SSE
        // `stream_text` path above. This tool loop is NOT streaming — it
        // calls non-streaming `:generateContent` once per round and emits
        // nothing to the channel until each round's full response has been
        // parsed. The idle watchdog measures the gap between emitted
        // events, so wrapping it here would abort a legitimately long
        // single round (e.g. a large-context, high-thinking-effort turn)
        // the moment it exceeds `streamIdleTimeoutMs`, even though the
        // per-request reqwest `timeout(timeoutMs)` still bounds each round
        // against a genuine hang. That HTTP timeout is the correct guard
        // for a non-streaming call, so we rely on it — matching the
        // Anthropic tool-loop path.
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

#[allow(clippy::too_many_arguments)]
struct GoogleToolConversation {
    client: reqwest::Client,
    backend: EngineBackendConfig,
    url: String,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
    context: EngineAdapterDebugContext,
    contents: Vec<Value>,
}

#[async_trait]
impl ToolConversation for GoogleToolConversation {
    async fn next_turn(
        &mut self,
        request: &EngineTextStreamRequest,
        results: Vec<ExecutedToolCall>,
        events: &ToolEventSender,
    ) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
        if !results.is_empty() {
            let parts = results
                .into_iter()
                .map(|item| {
                    let response = match item.result.content {
                        value @ Value::Object(_) => value,
                        value => json!({"result": value}),
                    };
                    let mut function_response =
                        json!({"name": item.call.name, "response": response});
                    let native_id = self
                        .contents
                        .last()
                        .and_then(|content| content.get("parts"))
                        .and_then(Value::as_array)
                        .is_some_and(|parts| {
                            parts.iter().any(|part| {
                                part.pointer("/functionCall/id").and_then(Value::as_str)
                                    == Some(item.call.id.as_str())
                            })
                        });
                    if native_id {
                        function_response["id"] = json!(item.call.id);
                    }
                    json!({"functionResponse": function_response})
                })
                .collect::<Vec<_>>();
            self.contents.push(json!({"role": "user", "parts": parts}));
        }
        let body = build_local_tool_body(&self.backend, request, self.contents.clone());
        battersea_model::adapter::payload::check_payload(&body, "google")?;
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
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|error| EngineAdapterRequestError::transport("google", error.to_string()))?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            return Err(normalize_google_http_error(
                status,
                response.bounded_text().await.ok(),
            ));
        }
        let raw: Value = response.bounded_json().await.map_err(|error| {
            EngineAdapterRequestError::invalid_response("google", error.to_string())
        })?;
        let payload: GeminiGenerateContentResponse =
            serde_json::from_value(raw.clone()).map_err(|error| {
                EngineAdapterRequestError::invalid_response("google", error.to_string())
            })?;
        if payload.first_finish_reason().as_deref() != Some("STOP") {
            return Err(EngineAdapterRequestError::invalid_response(
                "google",
                format!(
                    "Gemini turn did not complete: {:?}.",
                    payload.first_finish_reason()
                ),
            ));
        }
        let calls = collect_google_function_calls(&payload)?;
        emit_response(
            self.logger.as_ref(),
            &self.context,
            request.shared.operation,
            status,
            EngineAdapterResponseDetail {
                ok: true,
                request_id: None,
                response_id: payload.response_id.clone(),
                output_chars: payload.first_text().map(|text| text.chars().count()),
                usage: raw.get("usageMetadata").cloned(),
                streaming: Some(false),
                stop_reason: payload.first_finish_reason(),
            },
        )
        .await;
        // Preserve the complete content, including thought signatures and unknown fields.
        if let Some(content) = raw.pointer("/candidates/0/content") {
            self.contents.push(content.clone());
        }
        for event in payload.into_text_stream_events() {
            send_event(events, event).await?;
        }
        Ok(calls)
    }
}

/// Resolves the base URL for Gemini API calls. The engine.yaml
/// `endpoint` field carries the host root (default
/// `https://generativelanguage.googleapis.com`); per-request paths
/// (`v1beta/models/{model}:generateContent`) are appended by the
/// adapter. Trailing slashes are normalised so the splice produces a
/// well-formed URL.
fn resolve_google_base_url(endpoint: &str) -> String {
    // The endpoint is required and validated non-blank at construction, so
    // there is no host fallback here — a blank input yields a blank result.
    endpoint.trim().trim_end_matches('/').to_string()
}

/// Reads the Gemini API key from the env var named in
/// `backend.auth.api_key_env`, falling back to an inline key when the
/// env var is empty. Returns a configuration-class error if neither
/// source has a value; the engine surfaces this through the
/// `live_ready` check rather than waiting for a 401 mid-activation.
fn resolve_google_api_key(
    backend: &EngineBackendConfig,
) -> Result<String, EngineAdapterRequestError> {
    let env_var = backend.auth.api_key_env.as_str();
    if !env_var.is_empty() {
        if let Ok(value) = std::env::var(env_var) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }
    if let Some(inline) = backend.auth.api_key.as_deref() {
        let trimmed = inline.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    Err(EngineAdapterRequestError::transport(
        "google",
        format!(
            "Google Gemini backend \"{}\" has no API key. Set {} or configure auth.apiKey.",
            backend.id,
            if env_var.is_empty() {
                "GOOGLE_API_KEY"
            } else {
                env_var
            }
        ),
    ))
}

/// Translates an `EngineTextStreamRequest` into a Gemini
/// `streamGenerateContent` request body. The shape is documented at
/// <https://ai.google.dev/api/generate-content>; we set the fields
/// the engine actually steers and skip everything Gemini doesn't
/// honour (see `chat_parameters::GOOGLE_SUPPORTED_CHAT_PARAMETERS`).
fn build_stream_body(
    backend: &EngineBackendConfig,
    request: &EngineTextStreamRequest,
) -> Result<Value, EngineAdapterRequestError> {
    let mut body = json!({
        "contents": super::google_messages(&request.shared)
    });
    if !request
        .shared
        .text_for_role(battersea_model::Role::System)
        .trim()
        .is_empty()
    {
        body["systemInstruction"] = json!({
            "parts": [{ "text": request.shared.text_for_role(battersea_model::Role::System) }]
        });
    }
    body["generationConfig"] = build_generation_config(backend, &request.shared.chat);
    apply_tool_choice(&mut body, &request.shared.chat);
    Ok(body)
}

/// Builds the request body for one round of the engine-orchestrated
/// tool loop. Differs from `build_stream_body` in two ways:
///   1. `contents` carries the accumulated history (user prompt +
///      prior model functionCall turns + functionResponse parts), not
///      just the initial prompt.
///   2. The `tools` field is populated from `request.local_tools`,
///      and `toolConfig.functionCallingConfig` reflects
///      `chat.tool_choice`.
///
/// `model` lives in the URL path (not the body), same as the
/// streaming path.
fn build_local_tool_body(
    backend: &EngineBackendConfig,
    request: &EngineTextStreamRequest,
    contents: Vec<Value>,
) -> Value {
    let mut body = json!({ "contents": contents });
    if !request
        .shared
        .text_for_role(battersea_model::Role::System)
        .trim()
        .is_empty()
    {
        body["systemInstruction"] = json!({
            "parts": [{ "text": request.shared.text_for_role(battersea_model::Role::System) }]
        });
    }
    body["generationConfig"] = build_generation_config(backend, &request.shared.chat);

    // Wrap every local-tool definition in a Gemini functionDeclaration.
    // Gemini doesn't have a "strict" toggle the way OpenAI does — the
    // schema validation is always best-effort against the declared
    // shape — so `chat.strict_tool_inputs` is silently ignored here.
    //
    // Tool input schemas are authored as full JSON Schema (because
    // OpenAI's strict mode demands it), but Gemini accepts only a
    // narrow OpenAPI-3 subset and rejects unknown keys outright (e.g.
    // `additionalProperties`, `$schema`, `$ref`). Sanitise each
    // schema before forwarding.
    let function_declarations: Vec<Value> = request
        .local_tools
        .iter()
        .map(|tool| {
            json!({
                "name": super::local_tool_name(&tool.name),
                "description": tool.description,
                "parameters": sanitize_schema_for_gemini(&tool.input_schema),
            })
        })
        .collect();
    if !function_declarations.is_empty() {
        body["tools"] = json!([{ "functionDeclarations": function_declarations }]);
    }
    apply_tool_choice(&mut body, &request.shared.chat);
    body
}

/// JSON-Schema keywords Gemini's `functionDeclarations[*].parameters`
/// validator rejects with "Unknown name 'X' at … Cannot find field."
/// Gemini accepts a narrow OpenAPI-3 subset (`type`, `format`,
/// `description`, `nullable`, `enum`, `properties`, `required`,
/// `items`, `minItems`, `maxItems`, plus `anyOf` on recent versions).
/// Everything else needs to be dropped before posting. Listing the
/// known-bad keys (rather than allowlisting) keeps the sanitiser
/// forward-compatible if Gemini relaxes its schema in the future.
const GEMINI_REJECTED_SCHEMA_KEYS: &[&str] = &[
    "additionalProperties",
    "$schema",
    "$ref",
    "$id",
    "$defs",
    "$comment",
    "definitions",
    "title",
    "default",
    "examples",
    "patternProperties",
    "dependentRequired",
    "dependentSchemas",
    "unevaluatedProperties",
    "unevaluatedItems",
    "propertyOrdering",
    "if",
    "then",
    "else",
    "allOf",
    "oneOf",
    "not",
];

/// Recursively strips JSON-Schema keywords Gemini does not accept
/// from a tool's input schema. Walks `properties.*`, `items`, and
/// `anyOf[*]` so nested sub-schemas get the same treatment as the
/// root. Unrelated keys are preserved verbatim.
fn sanitize_schema_for_gemini(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut cleaned = serde_json::Map::with_capacity(map.len());
            for (key, value) in map {
                if GEMINI_REJECTED_SCHEMA_KEYS.contains(&key.as_str()) {
                    continue;
                }
                cleaned.insert(key.clone(), sanitize_schema_for_gemini(value));
            }
            Value::Object(cleaned)
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_schema_for_gemini).collect()),
        other => other.clone(),
    }
}

/// Preserve native call IDs. Older endpoints omit IDs; the adapter supplies
/// an internal correlation ID and omits it from the provider response.
fn collect_google_function_calls(
    payload: &GeminiGenerateContentResponse,
) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
    payload
        .candidates
        .first()
        .and_then(|candidate| candidate.content.as_ref())
        .into_iter()
        .flat_map(|content| content.parts.iter())
        .enumerate()
        .filter_map(|(index, part)| part.function_call.as_ref().map(|call| (index, call)))
        .map(|(index, call)| {
            let id = call
                .get("id")
                .cloned()
                .unwrap_or_else(|| json!(format!("call-{index}")));
            super::tool_loop::decode_tool_call(
                "google",
                Some(&id),
                call.get("name"),
                Some(call.get("args").cloned().unwrap_or_else(|| json!({}))),
            )
        })
        .collect()
}

/// Builds the request body for Gemini's `:countTokens` endpoint.
/// We send the same `contents` and `systemInstruction` shape as the
/// generation request so the count reflects exactly what would be
/// billed if we called `:streamGenerateContent` next.
fn build_count_tokens_body(request: &EngineTextStreamRequest) -> Value {
    let mut body = json!({
        "contents": super::google_messages(&request.shared)
    });
    if !request
        .shared
        .text_for_role(battersea_model::Role::System)
        .trim()
        .is_empty()
    {
        body["systemInstruction"] = json!({
            "parts": [{ "text": request.shared.text_for_role(battersea_model::Role::System) }]
        });
    }
    body
}

/// Builds the `generationConfig` object common to every Gemini
/// generation request. Mirrors what
/// `apply_anthropic_chat_parameters` does for Anthropic — only adds
/// fields the provider actually steers, leaving defaults to Gemini.
fn build_generation_config(backend: &EngineBackendConfig, chat: &EngineChatParameters) -> Value {
    let mut config = serde_json::Map::new();
    if let Some(max_output) = chat.max_output_tokens {
        config.insert("maxOutputTokens".to_string(), json!(max_output));
    }
    config.insert("temperature".to_string(), json!(chat.temperature));
    if let Some(top_p) = chat.top_p {
        config.insert("topP".to_string(), json!(top_p));
    }
    if let Some(top_k) = chat.top_k {
        config.insert("topK".to_string(), json!(top_k));
    }
    if !chat.stop_sequences.is_empty() {
        config.insert("stopSequences".to_string(), json!(chat.stop_sequences));
    }
    let mut thinking_config = serde_json::Map::new();
    if let Some(budget) = chat.thinking_budget_tokens {
        // Gemini's wire field is `thinkingBudget` (not `budgetTokens` —
        // the server silently ignores unknown keys here, which is how
        // an earlier typo went undetected). Explicit
        // `thinkingBudgetTokens` is an exact user cap, so it wins over
        // the generic `reasoningEffort` mapping below.
        thinking_config.insert("thinkingBudget".to_string(), json!(budget));
    } else if let Some(effort) = chat.reasoning_effort {
        if let Some(configured) = backend.capabilities.google_thinking.as_ref() {
            if let Some(value) = configured.effort_values.get(effort.as_str()) {
                thinking_config.insert(configured.effort_field.clone(), value.clone());
            }
        }
    }
    if let Some(visibility) = chat.thinking_visibility {
        // Gemini only surfaces thought parts in the response when
        // `includeThoughts` is explicitly true. Without this opt-in
        // the model may still reason internally but the stream
        // carries no `thought: true` parts for the engine's
        // reasoning_stream output to pick up.
        let include = matches!(visibility, EngineThinkingVisibility::Summarized);
        thinking_config.insert("includeThoughts".to_string(), json!(include));
    }
    if !thinking_config.is_empty() {
        config.insert("thinkingConfig".to_string(), Value::Object(thinking_config));
    }
    Value::Object(config)
}

/// Maps shared tool-choice policy to Gemini's function-calling configuration.
fn apply_tool_choice(body: &mut Value, chat: &battersea_model::engine::EngineChatParameters) {
    let mode = match chat.tool_choice.mode {
        EngineChatToolChoiceMode::Auto => "AUTO",
        EngineChatToolChoiceMode::None => "NONE",
        EngineChatToolChoiceMode::Required
        | EngineChatToolChoiceMode::Any
        | EngineChatToolChoiceMode::Tool => "ANY",
    };
    if mode == "AUTO" {
        return;
    }
    let mut config = json!({"mode": mode});
    if chat.tool_choice.mode == EngineChatToolChoiceMode::Tool {
        if let Some(name) = chat.tool_choice.tool_name.as_deref() {
            config["allowedFunctionNames"] = json!([super::local_tool_name(name)]);
        }
    }
    body["toolConfig"] = json!({"functionCallingConfig": config});
}

/// Maps a Gemini HTTP error response to the engine's neutral error
/// type. Gemini returns errors in `{ error: { code, message, status } }`
/// shape; we extract the message when present and fall back to the
/// raw body otherwise.
fn normalize_google_http_error(status: u16, body: Option<String>) -> EngineAdapterRequestError {
    let provider = "google";
    let message = body
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|value| {
            value
                .get("error")
                .and_then(|err| err.get("message"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| body.clone())
        .unwrap_or_else(|| format!("Gemini request failed with HTTP {status}."));
    EngineAdapterRequestError::new(provider, message, classify_status(status))
        .with_status_code(status)
}

/// Classifies Gemini HTTP status codes into the shared adapter error categories.
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

// ---------------------------------------------------------------------------
// Gemini response shapes — narrow Deserialize structs that capture the
// fields the engine actually consumes. Skipping the full schema keeps
// the adapter resilient to provider-side additions.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GeminiGenerateContentResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default, rename = "responseId")]
    response_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GeminiContent {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GeminiPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thought: Option<bool>,
    // The whole `functionCall` object (including its `id`) is captured
    // so it can be echoed back verbatim in the `contents` history for
    // the next tool-loop round.
    #[serde(
        default,
        rename = "functionCall",
        skip_serializing_if = "Option::is_none"
    )]
    function_call: Option<Value>,
    // Preserve every OTHER field Gemini attaches to a part so the
    // deserialize -> reserialize round-trip that rebuilds the model
    // turn in the tool loop is truly verbatim. Most importantly this
    // captures `thoughtSignature`: Gemini 3.x REST responses attach a
    // `thoughtSignature` string alongside each `functionCall` part, and
    // it MUST be sent back unchanged on the next function-calling turn
    // or the API rejects the request ("Function call is missing a
    // thought_signature"). Naming only the fields above and dropping
    // the rest silently stripped it. Flattening the remainder also
    // future-proofs against new part fields.
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GeminiUsageMetadata {
    #[serde(default, rename = "cachedContentTokenCount")]
    cached_content_token_count: Option<u64>,
    #[serde(default, rename = "thoughtsTokenCount")]
    thoughts_token_count: Option<u64>,
    #[serde(default, rename = "promptTokenCount")]
    prompt_token_count: Option<u64>,
    #[serde(default, rename = "candidatesTokenCount")]
    candidates_token_count: Option<u64>,
    #[serde(default, rename = "totalTokenCount")]
    total_token_count: Option<u64>,
}

impl GeminiGenerateContentResponse {
    fn first_text(&self) -> Option<String> {
        let mut joined = String::new();
        for candidate in &self.candidates {
            let Some(content) = &candidate.content else {
                continue;
            };
            for part in &content.parts {
                if part.thought == Some(true) {
                    continue;
                }
                if let Some(text) = part.text.as_deref() {
                    joined.push_str(text);
                }
            }
        }
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    fn first_finish_reason(&self) -> Option<String> {
        self.candidates
            .first()
            .and_then(|candidate| candidate.finish_reason.clone())
    }

    fn token_usage(&self) -> Option<EngineTokenUsage> {
        self.usage_metadata.as_ref().map(|usage| EngineTokenUsage {
            input_tokens: usage.prompt_token_count,
            output_tokens: usage
                .candidates_token_count
                .and_then(|n| n.checked_add(usage.thoughts_token_count.unwrap_or(0))),
            total_tokens: usage.total_token_count,
            cached_input_tokens: usage
                .cached_content_token_count
                .or(usage.prompt_token_count.map(|_| 0)),
            cache_write_input_tokens: usage.prompt_token_count.map(|_| 0),
            reasoning_output_tokens: usage.thoughts_token_count,
            ..Default::default()
        })
    }

    /// Translates one streaming chunk into provider-neutral engine
    /// events. Gemini sends a fully-formed `GenerateContentResponse`
    /// per SSE event with the new deltas in the `parts` array; we
    /// emit one `TextDelta` per text part and one `ReasoningDelta` per
    /// part flagged as `thought: true`. Token usage rides on the
    /// final chunk and is forwarded as a single `TokenUsage` event.
    fn into_text_stream_events(self) -> Vec<EngineTextStreamEvent> {
        let mut events = Vec::new();
        for candidate in &self.candidates {
            let Some(content) = &candidate.content else {
                continue;
            };
            for part in &content.parts {
                if let Some(text) = part.text.as_deref() {
                    if text.is_empty() {
                        continue;
                    }
                    if part.thought == Some(true) {
                        events.push(EngineTextStreamEvent::ReasoningDelta {
                            text: text.to_string(),
                        });
                    } else {
                        events.push(EngineTextStreamEvent::TextDelta {
                            text: text.to_string(),
                        });
                    }
                }
            }
        }
        if let Some(usage) = self.token_usage() {
            events.push(EngineTextStreamEvent::TokenUsage { usage });
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{EngineAdapterRequest, EngineOperation};
    use battersea_model::engine::EngineReasoningEffort;
    use battersea_model::engine::{
        EngineBackendCapabilities, EngineChatParameters, EngineChatToolChoice,
        EngineGoogleThinkingCapabilities, EngineTemperatureDispatch,
    };
    use std::collections::HashMap;

    fn empty_chat_params() -> EngineChatParameters {
        EngineChatParameters {
            stream: true,
            max_output_tokens: None,
            temperature: 1.0,
            top_p: None,
            top_k: None,
            stop_sequences: Vec::new(),
            tool_execution: battersea_model::engine::EngineToolExecutionMode::EngineOrchestrated,
            max_tool_rounds: 8,
            tool_choice: EngineChatToolChoice::default(),
            parallel_tool_calls: None,
            strict_tool_inputs: None,
            service_tier: None,
            safety_identifier: None,
            request_metadata: None,
            reasoning_effort: None,
            reasoning_summary: None,
            thinking_budget_tokens: None,
            thinking_visibility: None,
            response_verbosity: None,
            context_overflow: battersea_model::engine::EngineContextOverflow::Error,
            prompt_cache_key: None,
            prompt_cache_retention: None,
            store_response: None,
            max_provider_tool_calls: None,
            logprobs: None,
            top_logprobs: None,
        }
    }

    fn google_capabilities(
        effort_field: &str,
        effort_values: HashMap<String, Value>,
    ) -> EngineBackendCapabilities {
        EngineBackendCapabilities {
            supported_chat_parameters: vec![
                "stream".to_string(),
                "maxOutputTokens".to_string(),
                "temperature".to_string(),
                "topP".to_string(),
                "topK".to_string(),
                "stopSequences".to_string(),
                "toolExecution".to_string(),
                "maxToolRounds".to_string(),
                "toolChoice".to_string(),
                "reasoningEffort".to_string(),
                "thinkingBudgetTokens".to_string(),
                "thinkingVisibility".to_string(),
                "contextOverflow".to_string(),
            ],
            supported_tool_execution_modes: vec!["engine-orchestrated".to_string()],
            supported_tool_choices: battersea_model::engine::EngineBackendCapabilities::mock()
                .supported_tool_choices,
            supported_reasoning_efforts: battersea_model::engine::EngineBackendCapabilities::mock()
                .supported_reasoning_efforts,
            temperature_dispatch: EngineTemperatureDispatch::Always,
            reasoning_effort_when_unset: None,
            anthropic_thinking: None,
            google_thinking: Some(EngineGoogleThinkingCapabilities {
                effort_field: effort_field.to_string(),
                effort_values,
            }),
        }
    }

    fn thinking_level_capabilities() -> EngineBackendCapabilities {
        google_capabilities(
            "thinkingLevel",
            HashMap::from([
                ("none".to_string(), json!("minimal")),
                ("minimal".to_string(), json!("minimal")),
                ("low".to_string(), json!("low")),
                ("medium".to_string(), json!("medium")),
                ("high".to_string(), json!("high")),
                ("xhigh".to_string(), json!("high")),
            ]),
        )
    }

    fn thinking_budget_capabilities() -> EngineBackendCapabilities {
        google_capabilities(
            "thinkingBudget",
            HashMap::from([
                ("none".to_string(), json!(128)),
                ("minimal".to_string(), json!(512)),
                ("low".to_string(), json!(1024)),
                ("medium".to_string(), json!(-1)),
                ("high".to_string(), json!(8192)),
                ("xhigh".to_string(), json!(32768)),
            ]),
        )
    }

    fn backend_stub() -> EngineBackendConfig {
        EngineBackendConfig {
            id: "test".to_string(),
            provider: "google".to_string(),
            label: "Test".to_string(),
            enabled: true,
            endpoint: DEFAULT_GEMINI_HOST.to_string(),
            model: "synthetic-google-model".to_string(),
            display_order: None,
            chat: empty_chat_params(),
            capabilities: thinking_level_capabilities(),
            context_window_tokens: 1_000_000,
            options: Default::default(),
            auth: super::super::EngineAuthConfig {
                auth_type: "api-key".to_string(),
                api_key: None,
                api_key_env: "GOOGLE_API_KEY".to_string(),
                header: None,
                version_header: None,
                version: None,
                has_api_key: false,
            },
            short_description: String::new(),
            long_description: String::new(),
        }
    }

    fn stream_request(instructions: &str, user_content: &str) -> EngineTextStreamRequest {
        EngineTextStreamRequest {
            mock_response: None,
            shared: EngineAdapterRequest {
                operation: EngineOperation::FlowTextStream,
                messages: vec![
                    battersea_model::Message::text(
                        battersea_model::Role::System,
                        instructions.to_string(),
                    ),
                    battersea_model::Message::text(
                        battersea_model::Role::User,
                        user_content.to_string(),
                    ),
                ],
                chat: empty_chat_params(),
                debug_rules: None,
            },
            local_tools: Vec::new(),
            local_tool_executor: None,
            max_tool_rounds: 8,
        }
    }

    #[test]
    fn resolve_base_url_normalises_trailing_slash() {
        assert_eq!(
            resolve_google_base_url("https://generativelanguage.googleapis.com/"),
            "https://generativelanguage.googleapis.com"
        );
        assert_eq!(
            resolve_google_base_url("  https://my-proxy.example/api  "),
            "https://my-proxy.example/api"
        );
    }

    #[test]
    fn build_stream_body_carries_user_content_and_system_instruction_and_generation_config() {
        let backend = backend_stub();
        let mut request = stream_request("Stay terse.", "Hello?");
        request.shared.chat.max_output_tokens = Some(2048);
        request.shared.chat.temperature = 0.7;
        request.shared.chat.top_p = Some(0.95);
        request.shared.chat.top_k = Some(40);
        request.shared.chat.stop_sequences = vec!["END".to_string()];
        request.shared.chat.thinking_budget_tokens = Some(1024);

        let body = build_stream_body(&backend, &request).expect("body");
        assert_eq!(body["contents"][0]["parts"][0]["text"], json!("Hello?"));
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            json!("Stay terse.")
        );
        let config = &body["generationConfig"];
        assert_eq!(config["maxOutputTokens"], json!(2048));
        assert_eq!(config["temperature"], json!(0.7));
        assert_eq!(config["topP"], json!(0.95));
        assert_eq!(config["topK"], json!(40));
        assert_eq!(config["stopSequences"], json!(["END"]));
        assert_eq!(config["thinkingConfig"]["thinkingBudget"], json!(1024));
        // No tool config when mode is AUTO.
        assert!(body.get("toolConfig").is_none());
    }

    #[test]
    fn build_stream_body_sets_include_thoughts_when_visibility_is_summarized() {
        // Gemini only emits thought parts when `includeThoughts: true`
        // is explicitly set. The engine's `thinkingVisibility:
        // Summarized` is the contract that says "I want the trace."
        let backend = backend_stub();
        let mut request = stream_request("Reason it through.", "Solve x.");
        request.shared.chat.thinking_visibility =
            Some(battersea_model::engine::EngineThinkingVisibility::Summarized);

        let body = build_stream_body(&backend, &request).expect("body");
        let thinking = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(thinking["includeThoughts"], json!(true));
    }

    #[test]
    fn build_stream_body_sets_include_thoughts_false_when_visibility_omitted() {
        // `Omitted` is an explicit "don't return the trace" — distinct
        // from leaving thinking_visibility as None (in which case we
        // don't add the key at all and Gemini falls back to its
        // default, which is also no thought parts).
        let backend = backend_stub();
        let mut request = stream_request("Reason it through.", "Solve x.");
        request.shared.chat.thinking_visibility =
            Some(battersea_model::engine::EngineThinkingVisibility::Omitted);

        let body = build_stream_body(&backend, &request).expect("body");
        let thinking = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(thinking["includeThoughts"], json!(false));
    }

    #[test]
    fn build_stream_body_omits_thinking_config_when_no_thinking_fields_set() {
        // Neither budget nor visibility — no thinkingConfig should
        // appear in the body at all. The Gemini default takes over.
        let backend = backend_stub();
        let request = stream_request("Hello.", "Hi.");
        let body = build_stream_body(&backend, &request).expect("body");
        assert!(
            body["generationConfig"].get("thinkingConfig").is_none(),
            "thinkingConfig should be absent when no thinking fields are set"
        );
    }

    #[test]
    fn build_stream_body_combines_budget_and_visibility_into_one_thinking_config() {
        // The two thinking knobs share `thinkingConfig`; setting both
        // must produce a single merged object, not two competing
        // ones. This is the common configuration for a Gemini
        // reasoning backend that wants both an explicit budget cap
        // AND visible thoughts in the response stream.
        let backend = backend_stub();
        let mut request = stream_request("Reason carefully.", "Why?");
        request.shared.chat.thinking_budget_tokens = Some(2048);
        request.shared.chat.thinking_visibility =
            Some(battersea_model::engine::EngineThinkingVisibility::Summarized);

        let body = build_stream_body(&backend, &request).expect("body");
        let thinking = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(thinking["thinkingBudget"], json!(2048));
        assert_eq!(thinking["includeThoughts"], json!(true));
    }

    #[test]
    fn build_stream_body_maps_configured_reasoning_effort_to_thinking_level() {
        let backend = backend_stub();
        let mut request = stream_request("Reason carefully.", "Why?");
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::Medium);
        request.shared.chat.thinking_visibility = Some(EngineThinkingVisibility::Summarized);

        let body = build_stream_body(&backend, &request).expect("body");
        let thinking = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(thinking["thinkingLevel"], json!("medium"));
        assert_eq!(thinking["includeThoughts"], json!(true));
        assert!(thinking.get("thinkingBudget").is_none());
    }

    #[test]
    fn build_stream_body_maps_configured_minimal_effort_to_minimal_thinking_level() {
        let backend = backend_stub();
        let mut request = stream_request("Reason lightly.", "Why?");
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::Minimal);

        let body = build_stream_body(&backend, &request).expect("body");
        let thinking = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(thinking["thinkingLevel"], json!("minimal"));
        assert!(thinking.get("thinkingBudget").is_none());
    }

    #[test]
    fn build_stream_body_maps_configured_reasoning_effort_to_thinking_budget() {
        let mut backend = backend_stub();
        backend.capabilities = thinking_budget_capabilities();
        let mut request = stream_request("Reason deeply.", "Why?");
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::High);
        request.shared.chat.thinking_visibility = Some(EngineThinkingVisibility::Summarized);

        let body = build_stream_body(&backend, &request).expect("body");
        let thinking = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(thinking["thinkingBudget"], json!(8192));
        assert_eq!(thinking["includeThoughts"], json!(true));
        assert!(thinking.get("thinkingLevel").is_none());
    }

    #[test]
    fn build_stream_body_prefers_explicit_thinking_budget_over_reasoning_effort() {
        let backend = backend_stub();
        let mut request = stream_request("Use the exact budget.", "Why?");
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::High);
        request.shared.chat.thinking_budget_tokens = Some(2048);

        let body = build_stream_body(&backend, &request).expect("body");
        let thinking = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(thinking["thinkingBudget"], json!(2048));
        assert!(thinking.get("thinkingLevel").is_none());
    }

    #[test]
    fn build_stream_body_omits_system_instruction_when_blank() {
        let backend = backend_stub();
        let request = stream_request("   ", "ping");
        let body = build_stream_body(&backend, &request).expect("body");
        assert!(body.get("systemInstruction").is_none());
    }

    #[test]
    fn named_tool_choice_restricts_the_requested_function() {
        let mut request = stream_request("", "");
        let chosen = "session.get";
        request.shared.chat.tool_choice.mode = EngineChatToolChoiceMode::Tool;
        request.shared.chat.tool_choice.tool_name = Some(chosen.to_string());
        let body = build_stream_body(&backend_stub(), &request).unwrap();
        assert_eq!(
            body["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"],
            json!([crate::adapter::local_tool_name(chosen)])
        );
    }

    #[test]
    fn build_stream_body_translates_tool_choice_modes_to_function_calling_config() {
        let backend = backend_stub();
        for (mode, expected) in [
            (EngineChatToolChoiceMode::None, "NONE"),
            (EngineChatToolChoiceMode::Required, "ANY"),
            (EngineChatToolChoiceMode::Any, "ANY"),
            (EngineChatToolChoiceMode::Tool, "ANY"),
        ] {
            let mut request = stream_request("", "go");
            request.shared.chat.tool_choice.mode = mode;
            let body = build_stream_body(&backend, &request).expect("body");
            assert_eq!(
                body["toolConfig"]["functionCallingConfig"]["mode"],
                json!(expected),
                "mode {:?}",
                mode
            );
        }
    }

    #[test]
    fn into_text_stream_events_emits_text_then_token_usage_and_routes_thoughts_to_reasoning() {
        let chunk = GeminiGenerateContentResponse {
            candidates: vec![GeminiCandidate {
                content: Some(GeminiContent {
                    role: Some("model".to_string()),
                    parts: vec![
                        GeminiPart {
                            text: Some("hello ".to_string()),
                            thought: None,
                            function_call: None,
                            ..Default::default()
                        },
                        GeminiPart {
                            text: Some("(thinking aloud)".to_string()),
                            thought: Some(true),
                            function_call: None,
                            ..Default::default()
                        },
                        GeminiPart {
                            text: Some("world".to_string()),
                            thought: Some(false),
                            function_call: None,
                            ..Default::default()
                        },
                    ],
                }),
                finish_reason: None,
            }],
            usage_metadata: Some(GeminiUsageMetadata {
                cached_content_token_count: None,
                thoughts_token_count: None,
                prompt_token_count: Some(12),
                candidates_token_count: Some(3),
                total_token_count: Some(15),
            }),
            response_id: None,
        };

        let events = chunk.into_text_stream_events();
        assert_eq!(events.len(), 4);
        assert!(matches!(
            &events[0],
            EngineTextStreamEvent::TextDelta { text } if text == "hello "
        ));
        assert!(matches!(
            &events[1],
            EngineTextStreamEvent::ReasoningDelta { text } if text == "(thinking aloud)"
        ));
        assert!(matches!(
            &events[2],
            EngineTextStreamEvent::TextDelta { text } if text == "world"
        ));
        match &events[3] {
            EngineTextStreamEvent::TokenUsage { usage } => {
                assert_eq!(usage.input_tokens, Some(12));
                assert_eq!(usage.output_tokens, Some(3));
                assert_eq!(usage.total_tokens, Some(15));
            }
            other => panic!("expected TokenUsage as final event, got {other:?}"),
        }
    }

    #[test]
    fn into_text_stream_events_drops_empty_text_parts() {
        let chunk = GeminiGenerateContentResponse {
            candidates: vec![GeminiCandidate {
                content: Some(GeminiContent {
                    role: None,
                    parts: vec![
                        GeminiPart {
                            text: Some(String::new()),
                            thought: None,
                            function_call: None,
                            ..Default::default()
                        },
                        GeminiPart {
                            text: None,
                            thought: None,
                            function_call: None,
                            ..Default::default()
                        },
                    ],
                }),
                finish_reason: None,
            }],
            usage_metadata: None,
            response_id: None,
        };
        let events = chunk.into_text_stream_events();
        assert!(events.is_empty());
    }

    #[test]
    fn first_text_concatenates_all_text_parts_and_skips_thoughts() {
        let response = GeminiGenerateContentResponse {
            candidates: vec![GeminiCandidate {
                content: Some(GeminiContent {
                    role: None,
                    parts: vec![
                        GeminiPart {
                            text: Some("part one ".to_string()),
                            thought: None,
                            function_call: None,
                            ..Default::default()
                        },
                        GeminiPart {
                            text: Some("internal monologue".to_string()),
                            thought: Some(true),
                            function_call: None,
                            ..Default::default()
                        },
                        GeminiPart {
                            text: Some("part two".to_string()),
                            thought: None,
                            function_call: None,
                            ..Default::default()
                        },
                    ],
                }),
                finish_reason: Some("STOP".to_string()),
            }],
            usage_metadata: None,
            response_id: None,
        };
        assert_eq!(response.first_text(), Some("part one part two".to_string()));
        assert_eq!(response.first_finish_reason(), Some("STOP".to_string()));
    }

    #[test]
    fn normalize_http_error_extracts_message_from_gemini_error_envelope() {
        let body = serde_json::json!({
            "error": {
                "code": 429,
                "message": "Resource exhausted: quota exceeded.",
                "status": "RESOURCE_EXHAUSTED"
            }
        })
        .to_string();
        let err = normalize_google_http_error(429, Some(body));
        assert_eq!(err.status_code, Some(429));
        assert_eq!(err.classification.as_str(), "rate_limit");
        assert!(
            err.message.contains("quota exceeded"),
            "expected quota message, got {:?}",
            err.message
        );
    }

    #[test]
    fn classify_status_covers_provider_error_buckets() {
        assert_eq!(classify_status(401), "auth");
        assert_eq!(classify_status(403), "auth");
        assert_eq!(classify_status(429), "rate_limit");
        assert_eq!(classify_status(500), "server");
        assert_eq!(classify_status(503), "server");
        assert_eq!(classify_status(400), "request");
        assert_eq!(classify_status(404), "request");
    }

    #[test]
    fn build_local_tool_body_includes_function_declarations_and_history() {
        // The tool round-trip body must (1) carry the accumulated
        // conversation history, not just the latest user prompt, (2)
        // wrap every local tool's input schema in a Gemini
        // functionDeclaration, and (3) NOT carry the model id (model
        // lives in the URL).
        let backend = backend_stub();
        let mut request = stream_request("Operate the safe.", "Open it.");
        request.local_tools = vec![super::super::EngineLocalToolDefinition {
            name: "open_safe".to_string(),
            description: "Open the safe with a numeric code.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "code": { "type": "string" } },
                "required": ["code"]
            }),
        }];
        let history = vec![
            json!({ "role": "user", "parts": [{ "text": "Open it." }] }),
            json!({
                "role": "model",
                "parts": [{ "functionCall": { "name": "open_safe", "args": { "code": "1234" } } }]
            }),
            json!({
                "role": "user",
                "parts": [{ "functionResponse": { "name": "open_safe", "response": { "ok": true } } }]
            }),
        ];

        let body = build_local_tool_body(&backend, &request, history.clone());

        assert_eq!(body["contents"], json!(history));
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            json!("Operate the safe.")
        );
        // The functionDeclarations array carries the tool's name and
        // its input schema (parameters).
        let declarations = &body["tools"][0]["functionDeclarations"];
        assert_eq!(declarations[0]["name"], json!("open_safe"));
        assert_eq!(
            declarations[0]["description"],
            json!("Open the safe with a numeric code.")
        );
        assert_eq!(
            declarations[0]["parameters"],
            json!({
                "type": "object",
                "properties": { "code": { "type": "string" } },
                "required": ["code"]
            })
        );
        // Model is not in the body — it's part of the URL.
        assert!(body.get("model").is_none());
    }

    #[test]
    fn build_local_tool_body_strips_json_schema_keywords_gemini_rejects() {
        // Tool input schemas authored for OpenAI's strict mode carry
        // `additionalProperties`, `$schema`, etc. The Gemini API
        // responds with `Unknown name "additionalProperties" ...
        // Cannot find field` if those leak through. The body builder
        // must sanitise each schema before forwarding.
        let backend = backend_stub();
        let mut request = stream_request("Tool runner.", "Run it.");
        request.local_tools = vec![super::super::EngineLocalToolDefinition {
            name: "do_thing".to_string(),
            description: "Do the thing.".to_string(),
            input_schema: json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "title": "DoThingArgs",
                "additionalProperties": false,
                "properties": {
                    "target": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": { "id": { "type": "string" } }
                    },
                    "tags": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": { "label": { "type": "string" } }
                        }
                    }
                },
                "required": ["target"]
            }),
        }];

        let body = build_local_tool_body(&backend, &request, Vec::new());
        let params = &body["tools"][0]["functionDeclarations"][0]["parameters"];

        assert!(params.get("additionalProperties").is_none(), "root level");
        assert!(params.get("$schema").is_none());
        assert!(params.get("title").is_none());
        assert!(
            params["properties"]["target"]
                .get("additionalProperties")
                .is_none(),
            "nested object"
        );
        assert!(
            params["properties"]["tags"]["items"]
                .get("additionalProperties")
                .is_none(),
            "array items"
        );
        // Allowed keys preserved.
        assert_eq!(params["type"], json!("object"));
        assert_eq!(params["required"], json!(["target"]));
        assert_eq!(
            params["properties"]["target"]["properties"]["id"]["type"],
            json!("string")
        );
    }

    #[test]
    fn sanitize_schema_passes_through_non_object_values_unchanged() {
        assert_eq!(
            sanitize_schema_for_gemini(&json!("string")),
            json!("string")
        );
        assert_eq!(sanitize_schema_for_gemini(&json!(42)), json!(42));
        assert_eq!(sanitize_schema_for_gemini(&json!(null)), json!(null));
        assert_eq!(
            sanitize_schema_for_gemini(&json!(["enum-a", "enum-b"])),
            json!(["enum-a", "enum-b"])
        );
    }

    #[test]
    fn collect_function_calls_pulls_every_call_part_with_synthesised_ids() {
        // Multiple functionCall parts in one model turn — and a text
        // part interleaved — should produce one EngineLocalToolCall
        // per functionCall, ids synthesised as `{name}-{index}` so
        // the engine's call/result pairing has stable handles.
        let payload = GeminiGenerateContentResponse {
            candidates: vec![GeminiCandidate {
                content: Some(GeminiContent {
                    role: Some("model".to_string()),
                    parts: vec![
                        GeminiPart {
                            text: Some("Calling tools…".to_string()),
                            thought: None,
                            function_call: None,
                            ..Default::default()
                        },
                        GeminiPart {
                            text: None,
                            thought: None,
                            function_call: Some(json!({
                                "name": "open_safe",
                                "args": { "code": "1234" }
                            })),
                            ..Default::default()
                        },
                        GeminiPart {
                            text: None,
                            thought: None,
                            function_call: Some(json!({
                                "name": "log_event",
                                "args": { "event": "safe-opened" }
                            })),
                            ..Default::default()
                        },
                    ],
                }),
                finish_reason: None,
            }],
            usage_metadata: None,
            response_id: None,
        };

        let calls = collect_google_function_calls(&payload).expect("calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "open_safe");
        assert_eq!(calls[0].arguments, json!({ "code": "1234" }));
        assert!(!calls[0].id.is_empty());
        assert_eq!(calls[1].name, "log_event");
        assert_eq!(calls[1].arguments, json!({ "event": "safe-opened" }));
        assert_ne!(calls[0].id, calls[1].id);
    }

    #[test]
    fn collect_function_calls_accepts_optional_arguments_and_preserves_native_ids() {
        // FunctionCall.args and id are optional in the Gemini REST contract.
        let payload: GeminiGenerateContentResponse = serde_json::from_value(json!({
            "candidates": [{"content": {"parts": [{"functionCall": {"id":"native-id", "name":"no_parameters"}}]}}]
        })).unwrap();
        let calls = collect_google_function_calls(&payload).unwrap();
        assert_eq!(
            calls[0].id,
            payload.candidates[0].content.as_ref().unwrap().parts[0]
                .function_call
                .as_ref()
                .unwrap()["id"]
        );
        assert!(calls[0].arguments.as_object().unwrap().is_empty());
    }

    #[test]
    fn collect_function_calls_rejects_malformed_calls_before_execution() {
        for call in [
            json!({"args": {}}),
            json!({"name": "wrong_arguments", "args": []}),
        ] {
            let payload: GeminiGenerateContentResponse = serde_json::from_value(json!({
                "candidates": [{"content": {"parts": [{"functionCall": call}]}}]
            }))
            .unwrap();
            assert!(collect_google_function_calls(&payload).is_err());
        }
    }

    #[test]
    fn model_turn_round_trip_preserves_thought_signature_on_function_call_parts() {
        // Regression: the engine-orchestrated tool loop rebuilds the
        // model turn for the next round by deserialising the response
        // into `GeminiPart`s and re-serialising them into the
        // `contents` history. Gemini 3.x attaches a `thoughtSignature`
        // string next to each `functionCall` part and REQUIRES it be
        // echoed back unchanged, or the next request fails with
        // "Function call is missing a thought_signature". A struct that
        // only names text/thought/functionCall silently dropped it.
        // This asserts the exact deserialise -> re-serialise path keeps
        // the signature (and the functionCall `id`) verbatim.
        let raw = json!({
            "functionCall": { "name": "character_list", "args": {}, "id": "gg22q5bx" },
            "thoughtSignature": "EpsCCpgCAQw51se16quV7Djz-signature-blob"
        });

        let part: GeminiPart =
            serde_json::from_value(raw.clone()).expect("deserialise functionCall part");
        let round_tripped = serde_json::to_value(&part).expect("re-serialise functionCall part");

        assert_eq!(
            round_tripped, raw,
            "model-turn round-trip must preserve thoughtSignature verbatim"
        );
        assert_eq!(
            round_tripped.get("thoughtSignature"),
            raw.get("thoughtSignature")
        );
    }

    #[test]
    fn build_count_tokens_body_carries_contents_and_optional_system() {
        let mut request = stream_request("System.", "tokens please");
        request.shared.messages[0] =
            battersea_model::Message::text(battersea_model::Role::System, "System.".to_string());
        let body = build_count_tokens_body(&request);
        assert_eq!(
            body["contents"][0]["parts"][0]["text"],
            json!("tokens please")
        );
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            json!("System.")
        );
    }
}
