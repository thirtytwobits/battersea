#[path = "../src/test_endpoints.rs"]
mod test_endpoints;

use async_trait::async_trait;
use axum::{
    extract::State,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use battersea_model::engine::{
    EngineBackendCapabilities, EngineChatParameters, EngineChatToolChoiceMode,
};
use battersea_providers::{
    adapter::EngineAuthConfig, EngineAdapter, EngineAdapterLogger, EngineAdapterRequest,
    EngineAdapterRequestError, EngineAdapterStreamLog, EngineBackendConfig, EngineBackendOptions,
    EngineLocalToolCall, EngineLocalToolDefinition, EngineLocalToolExecutor, EngineLocalToolResult,
    EngineOperation, EngineTextStreamEvent, EngineTextStreamRequest,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct RecordingExecutor(
    Mutex<Vec<EngineLocalToolCall>>,
    Option<EngineLocalToolResult>,
);
#[async_trait]
impl EngineLocalToolExecutor for RecordingExecutor {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        self.0.lock().unwrap().push(call.clone());
        Ok(self
            .1
            .clone()
            .unwrap_or_else(|| EngineLocalToolResult::json(json!({"found": call.arguments}))))
    }
}

#[derive(Default)]
struct StreamObserver(Mutex<Vec<String>>);
#[async_trait]
impl EngineAdapterLogger for StreamObserver {
    async fn on_stream_event(&self, entry: EngineAdapterStreamLog) {
        self.0.lock().unwrap().push(entry.event_type);
    }
}
impl StreamObserver {
    fn count(&self, event_type: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|observed| *observed == event_type)
            .count()
    }
}

#[derive(Clone)]
struct ServerState {
    requests: Arc<Mutex<Vec<Value>>>,
    replies: Arc<Mutex<VecDeque<(&'static str, String)>>>,
}
async fn reply(State(state): State<ServerState>, Json(body): Json<Value>) -> impl IntoResponse {
    state.requests.lock().unwrap().push(body);
    let (kind, body) = state
        .replies
        .lock()
        .unwrap()
        .pop_front()
        .expect("unexpected provider request");
    ([("content-type", kind)], body)
}
async fn server(
    replies: Vec<(&'static str, String)>,
) -> (String, ServerState, tokio::task::JoinHandle<()>) {
    let state = ServerState {
        requests: Default::default(),
        replies: Arc::new(Mutex::new(replies.into())),
    };
    let listener = tokio::net::TcpListener::bind(test_endpoints::TEST_BIND_ADDRESS)
        .await
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .fallback(post(reply))
        .with_state(state.clone());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, state, handle)
}
fn backend(provider: &str, url: &str, mode: &str) -> EngineBackendConfig {
    EngineBackendConfig {
        binding: None,
        id: "fixture".into(),
        label: "Fixture".into(),
        provider: provider.into(),
        enabled: true,
        endpoint: match provider {
            "anthropic" => format!("{url}/v1/messages"),
            "openai" => format!("{url}/v1/responses"),
            _ => url.into(),
        },
        model: "fixture-model".into(),
        display_order: None,
        chat: EngineChatParameters {
            response_format: None,
            output_modalities: None,
            max_output_tokens: Some(1024),
            ..EngineChatParameters::default_for_provider(provider)
        },
        capabilities: EngineBackendCapabilities::mock(),
        context_window_tokens: 8192,
        options: EngineBackendOptions {
            timeout_ms: Some(5000),
            max_retries: Some(0),
            stream_idle_timeout_ms: Some(2000),
            background: Some(provider == "openai" && mode != "stream"),
            background_mode: Some(if mode == "poll" { "poll" } else { "stream" }.into()),
            background_poll_interval_ms: Some(1),
            background_max_wait_ms: Some(5000),
            background_reconnect_idle_ms: Some(1000),
            background_max_reconnects: Some(2),
            ..Default::default()
        },
        auth: EngineAuthConfig {
            auth_type: "api-key".into(),
            api_key_env: String::new(),
            header: Some(
                if provider == "anthropic" {
                    "x-api-key"
                } else {
                    "Authorization"
                }
                .into(),
            ),
            version_header: Some("anthropic-version".into()),
            version: Some("2023-06-01".into()),
            has_api_key: true,
            api_key: Some("fixture".into()),
        },
        short_description: String::new(),
        long_description: String::new(),
    }
}
fn adapter(backend: EngineBackendConfig) -> Arc<dyn EngineAdapter> {
    observed_adapter(backend, None)
}
fn observed_adapter(
    mut backend: EngineBackendConfig,
    observer: Option<Arc<StreamObserver>>,
) -> Arc<dyn EngineAdapter> {
    let registry = battersea_providers::builtin_registry();
    backend
        .capabilities
        .supported_chat_parameters
        .retain(|name| {
            registry
                .chat_provider(&backend.provider)
                .unwrap()
                .parameters
                .contains(name)
        });
    registry
        .create_chat(
            backend,
            observer.map(|observer| observer as Arc<dyn EngineAdapterLogger>),
        )
        .unwrap()
}
fn request(
    backend: &EngineBackendConfig,
    executor: Arc<RecordingExecutor>,
) -> EngineTextStreamRequest {
    EngineTextStreamRequest {
        mock_response: None,
        shared: EngineAdapterRequest {
            operation: EngineOperation::FlowTextStream,
            messages: vec![
                battersea_model::Message::text(
                    battersea_model::Role::System,
                    "Fixture instructions".into(),
                ),
                battersea_model::Message::text(battersea_model::Role::User, "Fixture input".into()),
            ],
            chat: backend.chat.clone(),
            debug_rules: None,
        },
        local_tools: vec![EngineLocalToolDefinition {
            name: "session_get".into(),
            description: "Fixture tool".into(),
            input_schema: json!({"type":"object", "properties":{"key":{"type":"string"}}, "required":["key"]}),
        }],
        local_tool_executor: Some(executor),
        max_tool_rounds: 3,
    }
}
fn openai_response(output: Value) -> Value {
    json!({"id":"response-fixture", "created_at":1, "model":"fixture-model", "object":"response", "status":"completed", "output":output})
}

#[tokio::test]
async fn durable_responses_reject_unsupported_modes_before_dispatch() {
    use battersea_model::{ConversationRequest, ErrorKind, ToolCode};
    let (url, state, server) = server(vec![]).await;
    for (mode, streaming) in [("background-stream", true), ("stream", false)] {
        let backend = backend("openai", &url, mode);
        let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
        request.shared.chat.stream = streaming;
        let request = ConversationRequest {
            shared: request.shared,
            tools: request.local_tools,
            max_turns: 2,
        };
        let adapter = adapter(backend);
        let error = adapter.validate_conversation(&request, None).unwrap_err();
        assert_eq!(error.classification, ErrorKind::InvalidRequest);
        if streaming {
            assert_eq!(error.tool_code, Some(ToolCode::UnsupportedToolContinuation));
        }
        assert!(adapter
            .conversation_turn(request, None, vec![])
            .await
            .is_err());
        assert!(state.requests.lock().unwrap().is_empty());
    }
    server.abort();
}

#[tokio::test]
async fn mock_stream_reports_cumulative_usage_as_text_arrives() {
    let mut backend = backend("mock", "", "stream");
    backend.options.background_mode = None;
    backend.options.mock_stream_delay_multiplier = Some(0.0);
    let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
    let input = "A first sentence. Another sentence. A final sentence.";
    request.shared.messages = vec![battersea_model::Message::text(
        battersea_model::Role::User,
        input.into(),
    )];
    let adapter = adapter(backend);
    let input_tokens = adapter
        .count_text_stream_input_tokens(request.clone())
        .await
        .unwrap();
    let mut stream = adapter.stream_text(request).await.unwrap();
    let mut output = String::new();
    let mut last_output_tokens = 0;
    let mut usage_updates = 0;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            EngineTextStreamEvent::TextDelta { text } => output.push_str(&text),
            EngineTextStreamEvent::TokenUsage { usage } => {
                let Some(output_tokens) = usage.output_tokens else {
                    assert!(
                        output.is_empty(),
                        "turn admission precedes streamed content"
                    );
                    assert!(usage.input_tokens.is_none());
                    continue;
                };
                assert!(output_tokens > last_output_tokens);
                assert!(!output.is_empty());
                assert_eq!(usage.input_tokens, Some(input_tokens));
                assert_eq!(usage.total_tokens, Some(input_tokens + output_tokens));
                last_output_tokens = output_tokens;
                usage_updates += 1;
            }
            _ => {}
        }
    }
    assert_eq!(output, input);
    assert!(usage_updates > 1, "usage must update during the stream");
}
/// Streams `response` with a keep-alive and an event type the SDK does not model before completion.
fn sse(response: &Value, terminal: bool) -> String {
    let mut events = Vec::new();
    if terminal {
        events.push(
            json!({"type":"response.output_text.delta", "item_id":"message-fixture",
            "output_index":0, "content_index":0, "delta":"Finished", "logprobs":[]}),
        );
    }
    events.push(json!({"type":"keepalive"}));
    events.push(json!({"type":"response.unmodelled", "detail":{"future":"retained"}}));
    events.push(json!({"type":"response.completed", "response":response}));
    events
        .into_iter()
        .enumerate()
        .map(|(index, mut event)| {
            event["sequence_number"] = json!(index + 1);
            format!("data: {event}\n\n")
        })
        .collect()
}
fn openai_usage() -> Value {
    json!({"input_tokens":5, "input_tokens_details":{"cached_tokens":0}, "output_tokens":3,
        "output_tokens_details":{"reasoning_tokens":0}, "total_tokens":8})
}

async fn round_trip(provider: &str, mode: &str) {
    round_trip_result(provider, mode, None).await;
}

async fn round_trip_result(provider: &str, mode: &str, result: Option<EngineLocalToolResult>) {
    let is_error = result.as_ref().is_some_and(EngineLocalToolResult::is_error);
    let arguments = json!({"key":"active-session"});
    let (first, mut last) = match provider {
        "anthropic" => (
            json!({"id":"message-first", "stop_reason":"tool_use", "content":[
            {"type":"thinking", "thinking":"Fixture progress", "signature":"opaque-signature", "extra":"preserve"},
            {"type":"tool_use", "id":"call-a", "name":"session_get", "input":arguments}]}),
            json!({"id":"message-last", "stop_reason":"end_turn", "content":[{"type":"text","text":"Finished"}]}),
        ),
        "google" => (
            json!({"candidates":[{"finishReason":"STOP", "content":{"role":"model", "extra":"preserve", "parts":[
                {"thoughtSignature":"opaque-signature", "extra":"preserve", "functionCall":{"id":"call-a", "name":"session_get", "args":arguments}}
            ]}}]}),
            json!({"candidates":[{"finishReason":"STOP", "content":{"role":"model", "parts":[{"text":"Finished"}]}}]}),
        ),
        "openai" => (
            openai_response(json!([
                {"type":"reasoning", "id":"reasoning-fixture", "summary":[], "encrypted_content":"opaque-signature"},
                {"type":"function_call", "id":"function-fixture", "call_id":"call-a", "name":"session_get", "arguments":arguments.to_string(), "opaque_signature":{"future":"retained"}}
            ])),
            openai_response(
                json!([{"type":"message", "id":"message-fixture", "status":"completed", "role":"assistant",
            "content":[{"type":"output_text", "text":"Finished", "annotations":[]}]}]),
            ),
        ),
        _ => unreachable!(),
    };
    if provider == "openai" {
        last["usage"] = openai_usage();
    }
    let streamed = provider == "openai" && mode != "poll";
    let replies = if streamed {
        vec![
            ("text/event-stream", sse(&first, false)),
            ("text/event-stream", sse(&last, true)),
        ]
    } else {
        vec![
            ("application/json", first.to_string()),
            ("application/json", last.to_string()),
        ]
    };
    let (url, state, server) = server(replies).await;
    let backend = backend(provider, &url, mode);
    let executor = Arc::new(RecordingExecutor(Mutex::new(Vec::new()), result.clone()));
    let mut request = request(&backend, executor.clone());
    let history = [
        (battersea_model::Role::User, "Earlier user turn"),
        (battersea_model::Role::Assistant, "Earlier assistant turn"),
        (battersea_model::Role::User, "Latest user turn"),
    ];
    request.shared.messages.truncate(1);
    request.shared.messages.extend(
        history
            .iter()
            .map(|(role, text)| battersea_model::Message::text(*role, (*text).into())),
    );
    let history_len = history.len();
    let expected_schema = request.local_tools[0].input_schema.clone();
    let observer = Arc::new(StreamObserver::default());
    let mut stream = observed_adapter(backend, Some(observer.clone()))
        .stream_text(request)
        .await
        .unwrap();
    let mut events = Vec::new();
    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .unwrap()
    {
        events.push(event.unwrap());
    }
    if streamed {
        assert_eq!(observer.count("keepalive"), 2);
        assert_eq!(observer.count("response.unmodelled"), 2);
    }
    if provider == "openai" {
        assert!(events.iter().any(|event| matches!(
            event,
            EngineTextStreamEvent::TokenUsage { usage } if usage.total_tokens == Some(8)
        )));
    }
    let calls = executor.0.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments, arguments);
    assert_eq!(calls[0].id, "call-a");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, EngineTextStreamEvent::ToolCallStarted { .. }))
            .count(),
        calls.len()
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                EngineTextStreamEvent::ToolCallCompleted { ok, .. } if *ok != is_error
            ))
            .count(),
        calls.len()
    );
    assert!(events.iter().any(
        |event| matches!(event, EngineTextStreamEvent::TextDelta { text } if text == "Finished")
    ));
    let requests = state.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["tools"], requests[1]["tools"]);
    let key = match provider {
        "anthropic" => "messages",
        "google" => "contents",
        _ => "input",
    };
    for (index, (role, text)) in history.iter().enumerate() {
        let actual = &requests[0][key][index];
        let expected_role = if provider == "google" && *role == battersea_model::Role::Assistant {
            json!("model")
        } else {
            json!(role)
        };
        assert_eq!(actual["role"], expected_role);
        assert_eq!(
            if provider == "google" {
                &actual["parts"][0]["text"]
            } else {
                &actual["content"][0]["text"]
            },
            &json!(text)
        );
        assert_eq!(actual, &requests[1][key][index]);
    }
    assert!(!serde_json::to_string(&*requests)
        .unwrap()
        .contains("private-host-value"));
    let expected_result = result
        .map(|result| result.provider_content().unwrap())
        .unwrap_or_else(|| json!({"found": arguments}));
    match provider {
        "anthropic" => {
            assert_eq!(requests[0]["system"], requests[1]["system"]);
            assert_eq!(
                requests[1]["messages"][history_len]["content"],
                first["content"]
            );
            assert_eq!(requests[0]["tools"][0]["input_schema"], expected_schema);
            assert_eq!(
                requests[1]["messages"][history_len + 1]["content"][0]["tool_use_id"],
                calls[0].id
            );
            assert_eq!(
                requests[1]["messages"][history_len + 1]["content"][0]["content"],
                expected_result.to_string()
            );
            assert_eq!(requests[0]["tool_choice"]["type"], "auto");
            assert_eq!(
                requests[1]["messages"][history_len + 1]["content"][0]["is_error"],
                is_error
            );
        }
        "google" => {
            assert_eq!(
                requests[0]["systemInstruction"],
                requests[1]["systemInstruction"]
            );
            assert_eq!(
                requests[1]["contents"][history_len],
                first["candidates"][0]["content"]
            );
            assert_eq!(
                requests[0]["tools"][0]["functionDeclarations"][0]["parameters"],
                expected_schema
            );
            assert_eq!(
                requests[1]["contents"][history_len + 1]["parts"][0]["functionResponse"]["id"],
                calls[0].id
            );
            assert_eq!(
                requests[1]["contents"][history_len + 1]["parts"][0]["functionResponse"]
                    ["response"],
                expected_result
            );
        }
        "openai" => {
            assert_eq!(requests[0]["instructions"], requests[1]["instructions"]);
            assert_eq!(requests[1]["input"][history_len], first["output"][0]);
            assert_eq!(requests[0]["tools"][0]["parameters"], expected_schema);
            assert_eq!(
                requests[1]["input"][history_len + 2]["call_id"],
                calls[0].id
            );
            assert_eq!(
                requests[1]["input"][history_len + 2]["output"],
                expected_result.to_string()
            );
        }
        _ => unreachable!(),
    }
    server.abort();
}

#[tokio::test]
async fn anthropic_tools_use_shared_executor_and_preserve_signed_content() {
    round_trip("anthropic", "stream").await;
}
#[tokio::test]
async fn google_tools_use_shared_executor_and_preserve_signatures_and_ids() {
    round_trip("google", "stream").await;
}
#[tokio::test]
async fn openai_stream_tools_use_shared_executor_and_preserve_reasoning() {
    round_trip("openai", "stream").await;
}
#[tokio::test]
async fn openai_background_poll_tools_use_shared_executor() {
    round_trip("openai", "poll").await;
}
#[tokio::test]
async fn openai_background_stream_tools_use_shared_executor() {
    round_trip("openai", "background-stream").await;
}

#[tokio::test]
async fn configured_tool_choice_restrictions_fail_before_counting_or_generation() {
    for provider in ["anthropic", "google", "openai"] {
        let mut backend = backend(provider, "http://127.0.0.1:1", "stream");
        backend.capabilities.supported_tool_choices = vec![
            EngineChatToolChoiceMode::Auto,
            EngineChatToolChoiceMode::None,
        ];
        let adapter = adapter(backend.clone());
        for mode in [
            EngineChatToolChoiceMode::Required,
            EngineChatToolChoiceMode::Any,
            EngineChatToolChoiceMode::Tool,
        ] {
            let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
            request.shared.chat.tool_choice.mode = mode;
            request.shared.chat.tool_choice.tool_name = Some("session_get".into());
            let count_error = adapter
                .count_text_stream_input_tokens(request.clone())
                .await
                .unwrap_err();
            let generation_error = match adapter.stream_text(request).await {
                Err(error) => error,
                Ok(_) => panic!("unsupported choice accepted"),
            };
            assert_eq!(count_error.classification.as_str(), "invalid_request");
            assert_eq!(generation_error.classification, count_error.classification);
        }
    }
}

#[tokio::test]
async fn truncation_and_malformed_calls_never_reach_the_executor() {
    for (provider, response) in [
        (
            "anthropic",
            json!({"stop_reason":"max_tokens", "content":[{"type":"tool_use", "id":"call-a", "name":"session_get", "input":{"key":"partial"}}]}),
        ),
        (
            "anthropic",
            json!({"stop_reason":"tool_use", "content":[{"type":"tool_use", "name":"session_get", "input":{}}]}),
        ),
        (
            "google",
            json!({"candidates":[{"finishReason":"MAX_TOKENS", "content":{"role":"model","parts":[{"functionCall":{"name":"session_get","args":{"key":"partial"}}}]}}]}),
        ),
        (
            "google",
            json!({"candidates":[{"finishReason":"STOP", "content":{"role":"model","parts":[{"functionCall":{"name":"session_get","args":[]}}]}}]}),
        ),
        (
            "openai",
            openai_response(
                json!([{"type":"function_call","call_id":"call-a","name":"session_get","arguments":"{broken"}]),
            ),
        ),
    ] {
        let (url, state, server) = server(vec![("application/json", response.to_string())]).await;
        let backend = backend(provider, &url, "poll");
        let executor = Arc::new(RecordingExecutor::default());
        let request = request(&backend, executor.clone());
        let events: Vec<_> = adapter(backend)
            .stream_text(request)
            .await
            .unwrap()
            .collect()
            .await;
        assert!(
            matches!(events.last(), Some(Err(error)) if error.classification.as_str() == "invalid_response")
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            Ok(EngineTextStreamEvent::ToolCallStarted { .. }
                | EngineTextStreamEvent::ToolCallCompleted { .. })
        )));
        assert!(executor.0.lock().unwrap().is_empty());
        assert_eq!(state.requests.lock().unwrap().len(), 1);
        server.abort();
    }
}

#[tokio::test]
async fn dropping_a_background_tool_stream_cancels_the_remote_response() {
    let cancelled = Arc::new(tokio::sync::Notify::new());
    let polling = Arc::new(tokio::sync::Notify::new());
    let poll_signal = polling.clone();
    let cancel_signal = cancelled.clone();
    let app = Router::new()
        .route(
            "/v1/responses",
            post(|| async { Json(json!({"id":"background-job", "status":"queued"})) }),
        )
        .route(
            "/v1/responses/background-job",
            get(move || {
                let signal = poll_signal.clone();
                async move {
                    signal.notify_one();
                    std::future::pending::<Json<Value>>().await
                }
            }),
        )
        .route(
            "/v1/responses/background-job/cancel",
            post(move || {
                let signal = cancel_signal.clone();
                async move {
                    signal.notify_one();
                    Json(json!({"status":"cancelled"}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind(test_endpoints::TEST_BIND_ADDRESS)
        .await
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut backend = backend("openai", &url, "poll");
    backend.options.background_poll_interval_ms = Some(1);
    let request = request(&backend, Arc::new(RecordingExecutor::default()));
    let stream = adapter(backend).stream_text(request).await.unwrap();
    polling.notified().await;
    drop(stream);
    tokio::time::timeout(std::time::Duration::from_secs(2), cancelled.notified())
        .await
        .expect("remote cancellation during blocked poll");
    server.abort();
}

#[tokio::test]
async fn unsupported_effort_fails_before_counting_or_generation_for_every_provider() {
    use battersea_model::engine::EngineReasoningEffort;
    for provider in ["openai", "anthropic", "google"] {
        let mut backend = backend(provider, "http://127.0.0.1:1", "stream");
        backend.capabilities.supported_reasoning_efforts = vec![EngineReasoningEffort::High];
        let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
        request.shared.chat.reasoning_effort = Some(EngineReasoningEffort::Minimal);
        let adapter = adapter(backend);
        let error = adapter
            .count_text_stream_input_tokens(request.clone())
            .await
            .unwrap_err();
        assert_eq!(error.classification.as_str(), "invalid_request");
        assert!(error.message.contains("reasoning effort"));
        match adapter.stream_text(request).await {
            Err(error) => assert_eq!(error.classification.as_str(), "invalid_request"),
            Ok(_) => panic!("unsupported effort accepted by {provider}"),
        }
    }
}

// M6.3 acceptance fixtures: expected wire shapes and effect counts are protocol contracts.
type PolicyReply = (u16, &'static str, String, Option<&'static str>);
#[derive(Clone, Default)]
struct PolicyServer {
    requests: Arc<Mutex<Vec<Value>>>,
    replies: Arc<Mutex<VecDeque<PolicyReply>>>,
}
async fn policy_reply(
    State(state): State<PolicyServer>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    state.requests.lock().unwrap().push(body);
    let (status, kind, body, after) = state
        .replies
        .lock()
        .unwrap()
        .pop_front()
        .expect("unexpected attempt");
    let mut response = (
        axum::http::StatusCode::from_u16(status).unwrap(),
        [("content-type", kind)],
        body,
    )
        .into_response();
    if let Some(after) = after {
        response
            .headers_mut()
            .insert("retry-after", after.parse().unwrap());
    }
    response
}
async fn policy_server(
    replies: Vec<PolicyReply>,
) -> (String, PolicyServer, tokio::task::JoinHandle<()>) {
    let state = PolicyServer {
        replies: Arc::new(Mutex::new(replies.into())),
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind(test_endpoints::TEST_BIND_ADDRESS)
        .await
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .fallback(post(policy_reply))
        .with_state(state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, state, task)
}
fn content_backend(provider: &str, url: &str) -> EngineBackendConfig {
    let mut backend = backend(provider, url, "stream");
    backend.capabilities.content = battersea_providers::builtin_registry()
        .chat_provider(provider)
        .unwrap()
        .content
        .clone();
    backend.options.max_retries = Some(2);
    backend.options.retry_initial_delay_ms = Some(1);
    backend.options.retry_max_delay_ms = Some(20);
    backend
}
fn completion(provider: &str, text: &str) -> String {
    match provider {
        "openai" => format!(
            "data: {}\n\ndata: {}\n\n",
            json!({"type":"response.output_text.delta", "sequence_number":1, "item_id":"message-fixture", "output_index":0, "content_index":0, "delta":text, "logprobs":[]}),
            json!({"type":"response.completed", "sequence_number":2, "response":openai_response(json!([{"type":"message", "id":"message-fixture", "status":"completed", "role":"assistant", "content":[{"type":"output_text", "text":text, "annotations":[]}]}]))})
        ),
        "anthropic" => format!(
            "data: {}\n\ndata: {}\n\ndata: {}\n\n",
            json!({"type":"content_block_delta", "index":0, "delta":{"type":"text_delta", "text":text}}),
            json!({"type":"message_delta", "delta":{"stop_reason":"end_turn"}, "usage":{"output_tokens":12}}),
            json!({"type":"message_stop"})
        ),
        "google" => format!(
            "data: {}\n\n",
            json!({"candidates":[{"content":{"role":"model", "parts":[{"text":text}]}, "finishReason":"STOP"}]})
        ),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn multimodal_counting_and_generation_share_ordered_content_and_validate_structured_results()
{
    use battersea_model::{ContentBlock, Message, ResponseFormat, Role};
    let expected = json!({"answer": ["first", "second"]});
    let schema = json!({"type":"object", "properties":{"answer":{"type":"array", "items":{"type":"string"}}}, "required":["answer"], "additionalProperties":false});
    for provider in ["openai", "anthropic", "google"] {
        let count = if provider == "google" {
            json!({"totalTokens":17})
        } else {
            json!({"input_tokens":17})
        };
        let (url, state, server) = policy_server(vec![
            (200, "application/json", count.to_string(), None),
            (
                200,
                "text/event-stream",
                completion(provider, &expected.to_string()),
                None,
            ),
        ])
        .await;
        let backend = content_backend(provider, &url);
        let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
        request.local_tools.clear();
        request.shared.chat.response_format = Some(ResponseFormat::JsonSchema {
            name: "contract_result".into(),
            schema: schema.clone(),
        });
        let mut content = vec![
            ContentBlock::Text {
                text: "before".into(),
            },
            ContentBlock::Image {
                url: "data:image/png;base64,AQID".into(),
                mime_type: "image/png".into(),
            },
            ContentBlock::Document {
                url: "data:application/pdf;base64,BAUG".into(),
                mime_type: "application/pdf".into(),
                filename: "evidence.pdf".into(),
            },
            ContentBlock::Text {
                text: "after".into(),
            },
        ];
        if provider == "google" {
            content.push(ContentBlock::Audio {
                url: "data:audio/wav;base64,BwgJ".into(),
                mime_type: "audio/wav".into(),
            });
            content.push(ContentBlock::Video {
                url: "https://example.test/clip.mp4".into(),
                mime_type: "video/mp4".into(),
            });
        }
        request.shared.messages = vec![Message {
            role: Role::User,
            content,
        }];
        let adapter = adapter(backend);
        assert!(
            adapter
                .count_text_stream_input_tokens(request.clone())
                .await
                .unwrap()
                > 0
        );
        let events = adapter
            .stream_text(request)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        let events = events.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
        assert!(
            matches!(events.iter().rev().nth(1), Some(EngineTextStreamEvent::StructuredOutput {value}) if value == &expected)
        );
        let calls = state.requests.lock().unwrap();
        let key = match provider {
            "openai" => "input",
            "anthropic" => "messages",
            _ => "contents",
        };
        let counted = if provider == "google" {
            &calls[0]["generateContentRequest"]
        } else {
            &calls[0]
        };
        assert_eq!(
            counted[key], calls[1][key],
            "counting must include the exact generation content"
        );
        let blocks = &calls[1][key][0][if provider == "google" {
            "parts"
        } else {
            "content"
        }];
        assert_eq!(blocks[0]["text"], "before");
        assert_eq!(blocks[3]["text"], "after");
        match provider {
            "openai" => {
                assert_eq!(blocks[1]["image_url"], "data:image/png;base64,AQID");
                assert_eq!(blocks[2]["file_data"], "data:application/pdf;base64,BAUG");
                assert_eq!(calls[1]["text"]["format"]["schema"], schema);
            }
            "anthropic" => {
                assert_eq!(blocks[1]["source"]["data"], "AQID");
                assert_eq!(blocks[2]["source"]["media_type"], "application/pdf");
                assert_eq!(calls[1]["output_config"]["format"]["schema"], schema);
            }
            _ => {
                assert_eq!(blocks[1]["inlineData"]["data"], "AQID");
                assert_eq!(blocks[4]["inlineData"]["mimeType"], "audio/wav");
                assert_eq!(
                    blocks[5]["fileData"]["fileUri"],
                    "https://example.test/clip.mp4"
                );
                assert_eq!(calls[1]["generationConfig"]["responseJsonSchema"], schema);
            }
        }
        server.abort();
    }
}

#[tokio::test]
async fn rejected_tool_continuation_retries_its_body_without_reexecuting_the_tool() {
    for provider in ["openai", "anthropic", "google"] {
        let arguments = json!({"key":"durable-effect"});
        let (kind, first, last) = match provider {
            "openai" => ("text/event-stream", sse(&openai_response(json!([{"type":"function_call", "id":"item", "call_id":"once", "name":"session_get", "arguments":arguments.to_string(), "opaque_signature":{"future":"retained"}}])), false), completion(provider, "done")),
            "anthropic" => ("application/json", json!({"stop_reason":"tool_use", "content":[{"type":"tool_use", "id":"once", "name":"session_get", "input":arguments}]}).to_string(), json!({"stop_reason":"end_turn", "content":[{"type":"text", "text":"done"}]}).to_string()),
            _ => ("application/json", json!({"candidates":[{"finishReason":"STOP", "content":{"role":"model", "parts":[{"functionCall":{"id":"once", "name":"session_get", "args":arguments}}]}}]}).to_string(), json!({"candidates":[{"finishReason":"STOP", "content":{"role":"model", "parts":[{"text":"done"}]}}]}).to_string()),
        };
        let (url, state, server) = policy_server(vec![
            (200, kind, first, None),
            (429, "application/json", "{}".into(), Some("0")),
            (200, kind, last, None),
        ])
        .await;
        let backend = content_backend(provider, &url);
        let executor = Arc::new(RecordingExecutor::default());
        let events = adapter(backend.clone())
            .stream_text(request(&backend, executor.clone()))
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(events.iter().all(Result::is_ok), "{provider}: {events:?}");
        assert_eq!(
            executor.0.lock().unwrap().len(),
            1,
            "accepted tool effect must not repeat"
        );
        let calls = state.requests.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(
            calls[1], calls[2],
            "retry must preserve the prepared continuation body"
        );
        if provider == "openai" {
            assert!(
                calls[1]["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["opaque_signature"] == json!({"future":"retained"})),
                "native provider fields survive SDK decoding and continuation"
            );
        }
        server.abort();
    }
}

#[tokio::test]
async fn ambiguous_server_failure_and_partial_stream_never_replay_generation() {
    for provider in ["openai", "anthropic", "google"] {
        for (status, body) in [
            (500, "{}".to_string()),
            (
                200,
                match provider {
                    "openai" => format!(
                        "data: {}\n\n",
                        json!({"type":"response.output_text.delta", "sequence_number":1,"item_id":"message-fixture", "output_index":0,"content_index":0,"delta":"partial", "logprobs":[]})
                    ),
                    "anthropic" => format!(
                        "data: {}\n\n",
                        json!({"type":"content_block_delta", "delta":{"type":"text_delta","text":"partial"}})
                    ),
                    _ => format!(
                        "data: {}\n\n",
                        json!({"candidates":[{"content":{"parts":[{"text":"partial"}]}}]})
                    ),
                },
            ),
        ] {
            let (url, state, server) =
                policy_server(vec![(status, "text/event-stream", body, None)]).await;
            let backend = content_backend(provider, &url);
            let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
            request.local_tools.clear();
            let result = adapter(backend).stream_text(request).await;
            let error = match result {
                Err(error) => error,
                Ok(stream) => stream
                    .collect::<Vec<_>>()
                    .await
                    .into_iter()
                    .find_map(Result::err)
                    .expect("truncation must fail"),
            };
            assert_eq!(error.dispatch, battersea_model::DispatchState::Unknown);
            assert_eq!(state.requests.lock().unwrap().len(), 1);
            server.abort();
        }
    }
}

fn stream_body(events: &[Value]) -> String {
    events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect()
}

async fn plain_stream(
    provider: &str,
    body: String,
) -> (
    Vec<Result<EngineTextStreamEvent, EngineAdapterRequestError>>,
    Arc<StreamObserver>,
) {
    let (url, _, server) = server(vec![("text/event-stream", body)]).await;
    let backend = backend(provider, &url, "stream");
    let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
    request.local_tools.clear();
    let observer = Arc::new(StreamObserver::default());
    let events = observed_adapter(backend, Some(observer.clone()))
        .stream_text(request)
        .await
        .unwrap()
        .collect()
        .await;
    server.abort();
    (events, observer)
}

#[tokio::test]
async fn unmodelled_stream_events_are_observed_and_skipped() {
    let delta = |sequence: u64, text: &str| {
        json!({"type":"response.output_text.delta", "sequence_number":sequence, "item_id":"message-fixture",
            "output_index":0, "content_index":0, "delta":text, "logprobs":[]})
    };
    let mut response = openai_response(json!([{"type":"message", "id":"message-fixture",
        "status":"completed", "role":"assistant", "content":[{"type":"output_text", "text":"Finished", "annotations":[]}]}]));
    response["usage"] = openai_usage();
    let anthropic_delta = |text: &str| json!({"type":"content_block_delta", "index":0, "delta":{"type":"text_delta", "text":text}});
    for (provider, keepalive, body) in [
        (
            "openai",
            "keepalive",
            stream_body(&[
                delta(1, "Fin"),
                json!({"type":"keepalive", "sequence_number":2}),
                json!({"type":"response.unmodelled", "sequence_number":3, "detail":{}}),
                delta(4, "ished"),
                json!({"type":"response.completed", "sequence_number":5, "response":response}),
            ]),
        ),
        (
            "anthropic",
            "ping",
            stream_body(&[
                json!({"type":"message_start", "message":{"usage":{"input_tokens":5, "output_tokens":0}}}),
                anthropic_delta("Fin"),
                json!({"type":"ping"}),
                json!({"type":"response.unmodelled", "detail":{}}),
                anthropic_delta("ished"),
                json!({"type":"message_delta", "delta":{"stop_reason":"end_turn"}, "usage":{"output_tokens":3}}),
                json!({"type":"message_stop"}),
            ]),
        ),
    ] {
        let (events, observer) = plain_stream(provider, body).await;
        let events = events
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|error| panic!("{provider}: {error:?}"));
        let text = events
            .iter()
            .filter_map(|event| match event {
                EngineTextStreamEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(text, "Finished", "{provider}");
        assert!(
            events.iter().any(|event| matches!(
                event,
                EngineTextStreamEvent::TokenUsage { usage } if usage.output_tokens == Some(3)
            )),
            "{provider}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, EngineTextStreamEvent::MessageStop { .. })),
            "{provider}"
        );
        assert_eq!(observer.count(keepalive), 1, "{provider}");
        assert_eq!(observer.count("response.unmodelled"), 1, "{provider}");
    }
}

#[tokio::test]
async fn malformed_modelled_stream_events_fail_as_invalid_response() {
    for (provider, event) in [
        (
            "openai",
            json!({"type":"response.output_text.delta", "sequence_number":1}),
        ),
        (
            "openai",
            json!({"type":"response.output_item.added", "sequence_number":1, "output_index":0,
                "item":{"type":"unmodelled_item"}}),
        ),
        ("anthropic", json!({"type":"content_block_start"})),
    ] {
        let (events, _) = plain_stream(provider, stream_body(std::slice::from_ref(&event))).await;
        let error = events
            .into_iter()
            .find_map(Result::err)
            .unwrap_or_else(|| panic!("{event} must fail"));
        assert_eq!(
            error.classification,
            battersea_model::ErrorKind::InvalidResponse,
            "{event}"
        );
    }
}

#[tokio::test]
async fn retry_after_beyond_the_configured_delay_bound_stops_without_retrying_early() {
    let (url, state, server) =
        policy_server(vec![(429, "application/json", "{}".into(), Some("3600"))]).await;
    let backend = content_backend("openai", &url);
    let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
    request.local_tools.clear();
    let error = adapter(backend)
        .stream_text(request)
        .await
        .err()
        .expect("rate limit");
    assert_eq!(error.dispatch, battersea_model::DispatchState::Rejected);
    assert_eq!(error.retry_after_ms, Some(3_600_000));
    assert_eq!(state.requests.lock().unwrap().len(), 1);
    server.abort();
}

#[tokio::test]
async fn the_operation_deadline_covers_backoff_and_retry_does_not_reset_it() {
    let (url, state, server) =
        policy_server(vec![(429, "application/json", "{}".into(), Some("1"))]).await;
    let mut backend = content_backend("openai", &url);
    backend.options.timeout_ms = Some(50);
    backend.options.retry_initial_delay_ms = Some(1000);
    backend.options.retry_max_delay_ms = Some(1000);
    let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
    request.local_tools.clear();
    let result = adapter(backend).stream_text(request).await;
    assert!(
        matches!(result,Err(error) if error.classification==battersea_model::ErrorKind::Timeout)
    );
    assert_eq!(state.requests.lock().unwrap().len(), 1);
    server.abort();
}

#[tokio::test]
async fn dropping_a_tool_consumer_interrupts_retry_backoff() {
    let (url, state, server) =
        policy_server(vec![(429, "application/json", "{}".into(), Some("1"))]).await;
    let mut backend = content_backend("anthropic", &url);
    backend.options.retry_initial_delay_ms = Some(1000);
    backend.options.retry_max_delay_ms = Some(1000);
    let executor = Arc::new(RecordingExecutor::default());
    let stream = adapter(backend.clone())
        .stream_text(request(&backend, executor.clone()))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while state.requests.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(stream);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(state.requests.lock().unwrap().len(), 1);
    assert!(executor.0.lock().unwrap().is_empty());
    server.abort();
}

#[tokio::test]
async fn invalid_media_and_unsupported_native_output_are_rejected_before_io() {
    use battersea_model::{ContentBlock, Message, ResponseFormat, Role};
    let (url, state, server) = policy_server(vec![]).await;
    for provider in ["openai", "anthropic", "google"] {
        for block in [
            ContentBlock::Image {
                url: "https://example.test/picture".into(),
                mime_type: "image/unsupported".into(),
            },
            ContentBlock::Document {
                url: "https://example.test/archive".into(),
                mime_type: "application/zip".into(),
                filename: "archive.zip".into(),
            },
            ContentBlock::Image {
                url: "file:///private/image.png".into(),
                mime_type: "image/png".into(),
            },
            ContentBlock::Image {
                url: "data:image/png;base64,not base64".into(),
                mime_type: "image/png".into(),
            },
            ContentBlock::Native {
                provider: "foreign-provider".into(),
                payload: json!({"signature":"opaque"}),
            },
        ] {
            let backend = content_backend(provider, &url);
            let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
            request.local_tools.clear();
            request.shared.messages = vec![Message {
                role: Role::User,
                content: vec![block],
            }];
            let adapter = adapter(backend);
            assert!(adapter
                .count_text_stream_input_tokens(request.clone())
                .await
                .is_err());
            assert!(adapter.stream_text(request).await.is_err());
        }
    }
    let backend = content_backend("anthropic", &url);
    let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
    request.shared.chat.response_format = Some(ResponseFormat::JsonObject);
    assert!(adapter(backend).stream_text(request).await.is_err());
    assert!(state.requests.lock().unwrap().is_empty());
    server.abort();
}

#[tokio::test]
async fn gemini_media_outputs_retain_bytes_and_require_the_requested_modality() {
    use battersea_model::{ContentBlock, Modality};
    for (mime, mode) in [
        ("image/png", Modality::Image),
        ("audio/wav", Modality::Audio),
    ] {
        let bytes = "AQIDBA==";
        let body = format!(
            "data: {}\n\n",
            json!({"candidates":[{"content":{"role":"model","parts":[{"inlineData":{"mimeType":mime,"data":bytes}}]},"finishReason":"STOP"}]})
        );
        let (url, state, server) = policy_server(vec![
            (200, "text/event-stream", body.clone(), None),
            (200, "text/event-stream", body, None),
        ])
        .await;
        let backend = content_backend("google", &url);
        let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
        request.local_tools.clear();
        request.shared.chat.output_modalities = Some(vec![mode]);
        let adapter = adapter(backend);
        let events = adapter
            .stream_text(request.clone())
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let expected_url = format!("data:{mime};base64,{bytes}");
        assert!(events.iter().any(|event|matches!(event,EngineTextStreamEvent::ContentBlock {block:ContentBlock::Image {url,mime_type}|ContentBlock::Audio {url,mime_type}} if url==&expected_url && mime_type==mime)));
        assert_eq!(
            state.requests.lock().unwrap()[0]["generationConfig"]["responseModalities"],
            json!([if mode == Modality::Image {
                "IMAGE"
            } else {
                "AUDIO"
            }])
        );
        request.shared.chat.output_modalities = None;
        let events = adapter
            .stream_text(request)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(
            events.last().unwrap().is_err(),
            "unsolicited media cannot bypass output admission"
        );
        server.abort();
    }
}

#[tokio::test]
async fn connection_refusal_retries_only_within_the_explicit_attempt_bound() {
    let listener = tokio::net::TcpListener::bind(test_endpoints::TEST_BIND_ADDRESS)
        .await
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    for provider in ["openai", "anthropic", "google"] {
        let backend = content_backend(provider, &url);
        let attempts = backend.options.max_retries.unwrap() + 1;
        let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
        request.local_tools.clear();
        let error = adapter(backend)
            .stream_text(request)
            .await
            .err()
            .expect("connection refused");
        assert_eq!(error.dispatch, battersea_model::DispatchState::NotSent);
        assert_eq!(error.attempts, attempts);
    }
}

#[tokio::test]
async fn accepted_connection_loss_does_not_recreate_generation() {
    use tokio::io::AsyncReadExt;
    for provider in ["openai", "anthropic", "google"] {
        let listener = tokio::net::TcpListener::bind(test_endpoints::TEST_BIND_ADDRESS)
            .await
            .unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 8192];
            assert!(stream.read(&mut buffer).await.unwrap() > 0);
            drop(stream);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let backend = content_backend(provider, &url);
        let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
        request.local_tools.clear();
        let error = adapter(backend)
            .stream_text(request)
            .await
            .err()
            .expect("connection lost");
        assert_eq!(error.dispatch, battersea_model::DispatchState::Unknown);
        assert_eq!(error.attempts, 1);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn audio_only_output_rejects_unrequested_text() {
    let (url, _, server) = policy_server(vec![(
        200,
        "text/event-stream",
        completion("google", "unexpected text"),
        None,
    )])
    .await;
    let mut backend = content_backend("google", &url);
    backend.chat.output_modalities = Some(vec![battersea_model::Modality::Audio]);
    let mut request = request(&backend, Arc::new(RecordingExecutor::default()));
    request.local_tools.clear();
    let events = adapter(backend)
        .stream_text(request)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(events.iter().any(Result::is_err));
    assert!(!events
        .iter()
        .any(|event| matches!(event, Ok(EngineTextStreamEvent::TextDelta { .. }))));
    server.abort();
}

#[tokio::test]
async fn content_results_and_tool_errors_reach_each_provider_without_host_metadata() {
    for provider in ["anthropic", "google", "openai"] {
        for is_error in [false, true] {
            let result = EngineLocalToolResult::Content(battersea_model::ToolContentResult {
                content_blocks: vec![battersea_model::ToolContentBlock::Text {
                    text: "answer".into(),
                }],
                structured_content: Some(json!({"answer": 42})),
                is_error,
                meta: serde_json::Map::from_iter([("host".into(), json!("private-host-value"))]),
            });
            round_trip_result(provider, "stream", Some(result)).await;
        }
    }
}
