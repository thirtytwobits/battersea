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
    adapter::EngineAuthConfig, EngineAdapter, EngineAdapterRequest, EngineAdapterRequestError,
    EngineBackendConfig, EngineBackendOptions, EngineLocalToolCall, EngineLocalToolDefinition,
    EngineLocalToolExecutor, EngineLocalToolResult, EngineOperation, EngineTextStreamEvent,
    EngineTextStreamRequest,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct RecordingExecutor(Mutex<Vec<EngineLocalToolCall>>);
#[async_trait]
impl EngineLocalToolExecutor for RecordingExecutor {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        self.0.lock().unwrap().push(call.clone());
        Ok(EngineLocalToolResult {
            content: json!({"found": call.arguments}),
        })
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
fn adapter(mut backend: EngineBackendConfig) -> Arc<dyn EngineAdapter> {
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
    registry.create_chat(backend, None).unwrap()
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
fn sse(response: &Value, terminal: bool) -> String {
    let mut events = String::new();
    if terminal {
        events.push_str(&format!("data: {}\n\n", json!({"type":"response.output_text.delta", "sequence_number":1,
            "item_id":"message-fixture", "output_index":0, "content_index":0, "delta":"Finished", "logprobs":[]})));
    }
    events.push_str(&format!(
        "data: {}\n\n",
        json!({"type":"response.completed", "sequence_number":2, "response":response})
    ));
    events
}

async fn round_trip(provider: &str, mode: &str) {
    let arguments = json!({"key":"active-session"});
    let (first, last) = match provider {
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
                {"type":"function_call", "id":"function-fixture", "call_id":"call-a", "name":"session_get", "arguments":arguments.to_string()}
            ])),
            openai_response(
                json!([{"type":"message", "id":"message-fixture", "status":"completed", "role":"assistant",
            "content":[{"type":"output_text", "text":"Finished", "annotations":[]}]}]),
            ),
        ),
        _ => unreachable!(),
    };
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
    let executor = Arc::new(RecordingExecutor::default());
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
    let mut stream = adapter(backend).stream_text(request).await.unwrap();
    let mut events = Vec::new();
    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .unwrap()
    {
        events.push(event.unwrap());
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
                EngineTextStreamEvent::ToolCallCompleted { ok: true, .. }
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
                &actual["content"]
            },
            &json!(text)
        );
        assert_eq!(actual, &requests[1][key][index]);
    }
    let expected_result = json!({"found": arguments});
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
        assert!(events[..events.len()-1].iter().all(|event|matches!(event,Ok(EngineTextStreamEvent::TokenUsage { usage }) if usage.input_tokens.is_none() && usage.output_tokens.is_none())));
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
