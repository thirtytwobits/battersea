use async_trait::async_trait;
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, Uri},
    response::Response,
    routing::post,
    Router,
};
use battersea_config::*;
use battersea_model::{engine::*, media::*, *};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    fmt::Write,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Wire {
    replies: Arc<Mutex<VecDeque<(u16, String, String)>>>,
    requests: Arc<Mutex<Vec<(String, HeaderMap, Value)>>>,
}
async fn handle(State(wire): State<Wire>, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    wire.requests.lock().unwrap().push((
        uri.to_string(),
        headers,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
    ));
    let (status, content_type, body) = wire
        .replies
        .lock()
        .unwrap()
        .pop_front()
        .expect("unexpected provider I/O");
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap()
}
async fn server(
    replies: Vec<(u16, String, String)>,
) -> (String, Wire, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/proxy/v1/", listener.local_addr().unwrap());
    let wire = Wire {
        replies: Arc::new(Mutex::new(replies.into())),
        requests: Default::default(),
    };
    let app = Router::new()
        .fallback(post(handle).get(handle).delete(handle))
        .with_state(wire.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, wire, task)
}
fn reply(value: Value) -> (u16, String, String) {
    (200, "application/json".into(), value.to_string())
}
fn sse(events: Vec<Value>, done: bool) -> (u16, String, String) {
    let mut body = String::new();
    for event in events {
        writeln!(body, "data: {event}\n").unwrap();
    }
    if done {
        body.push_str("data: [DONE]\n\n");
    }
    (200, "text/event-stream".into(), body)
}
fn protocols() -> ProtocolRegistry {
    let mut r = ProtocolRegistry::default();
    battersea_providers::register_builtin_adapters(&mut r).unwrap();
    r
}
fn configuration(protocol: &str, url: &str) -> ProviderConfiguration {
    let registry = protocols();
    let descriptor = registry.adapter(protocol).unwrap().chat.as_ref().unwrap();
    let mut file = parse_provider_configuration(include_str!("../providers.yaml")).unwrap();
    file.backends.remove("research-chat");
    file.providers.remove("research");
    let binding = file
        .providers
        .get_mut("office")
        .unwrap()
        .chat
        .as_mut()
        .unwrap();
    binding.adapter = protocol.into();
    binding.base_url = url.into();
    binding.auth = Authentication::Bearer {
        credential: CredentialReference::Host("office-key".into()),
    };
    if protocol == "anthropic-messages" {
        binding
            .headers
            .insert("anthropic-version".into(), "2023-06-01".into());
    }
    let backend = file.backends.get_mut("office-chat").unwrap();
    backend.capabilities.content = descriptor.content.clone();
    backend.capabilities.supported_chat_parameters = descriptor.parameters.clone();
    backend.capabilities.supported_tool_choices = vec![
        EngineChatToolChoiceMode::Auto,
        EngineChatToolChoiceMode::None,
        EngineChatToolChoiceMode::Required,
        EngineChatToolChoiceMode::Any,
        EngineChatToolChoiceMode::Tool,
    ];
    backend.options = Some(EngineBackendOptions {
        timeout_ms: Some(3000),
        max_retries: Some(0),
        stream_idle_timeout_ms: Some(1000),
        ..Default::default()
    });
    file
}
struct Secrets {
    calls: AtomicUsize,
}
impl Secrets {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}
impl CredentialResolver for Secrets {
    fn resolve(
        &self,
        reference: &CredentialReference,
    ) -> Result<ResolvedCredential, ProviderConfigurationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (CredentialReference::Host(name) | CredentialReference::Env(name)) = reference;
        Ok(ResolvedCredential {
            value: format!("secret-{name}").into(),
            scope_revision: format!("account-{name}-1"),
        })
    }
}
fn request(config: &ProviderConfiguration) -> EngineTextStreamRequest {
    EngineTextStreamRequest {
        shared: EngineAdapterRequest {
            operation: EngineOperation::FlowTextStream,
            messages: vec![Message::text(Role::User, "Hello".into())],
            chat: resolve_chat_parameters(
                config.backends["office-chat"].properties.clone().unwrap(),
            )
            .unwrap(),
            debug_rules: None,
        },
        local_tools: vec![],
        local_tool_executor: None,
        max_tool_rounds: 3,
        mock_response: None,
    }
}
async fn events(
    adapter: &Arc<dyn EngineAdapter>,
    request: EngineTextStreamRequest,
) -> Result<Vec<EngineTextStreamEvent>, EngineAdapterRequestError> {
    let mut stream = adapter.stream_text(request).await?;
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event?);
    }
    Ok(events)
}
fn completion(text: &str) -> Value {
    json!({"choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}})
}
fn streamed_completion(text: &str) -> (u16, String, String) {
    sse(
        vec![
            json!({"choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}),
        ],
        true,
    )
}

#[tokio::test]
async fn builtins_count_and_stream_through_prefixed_service_bindings() {
    for protocol in [
        "openai-responses",
        "anthropic-messages",
        "google-generate-content",
    ] {
        let text = "{\"ok\":true}";
        let count = if protocol == "google-generate-content" {
            json!({"totalTokens":3})
        } else {
            json!({"input_tokens":3})
        };
        let stream = match protocol {
            "openai-responses" => sse(
                vec![
                    json!({"type":"response.output_text.delta","sequence_number":1,"item_id":"message-1","output_index":0,"content_index":0,"delta":text,"logprobs":[]}),
                    json!({"type":"response.completed","sequence_number":2,"response":{"id":"response-1","created_at":1,"model":"example","object":"response","status":"completed","output":[{"type":"message","id":"message-1","status":"completed","role":"assistant","content":[{"type":"output_text","text":text,"annotations":[]}]}]}}),
                ],
                false,
            ),
            "anthropic-messages" => sse(
                vec![
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
                    json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":4}}),
                    json!({"type":"message_stop"}),
                ],
                false,
            ),
            _ => sse(
                vec![
                    json!({"candidates":[{"content":{"role":"model","parts":[{"text":text}]},"finishReason":"STOP"}]}),
                ],
                false,
            ),
        };
        let (url, wire, task) = server(vec![reply(count), stream]).await;
        let mut config = configuration(protocol, &url);
        config
            .providers
            .get_mut("office")
            .unwrap()
            .chat
            .as_mut()
            .unwrap()
            .auth = Authentication::Header {
            header: "x-service-key".into(),
            credential: CredentialReference::Host("office-key".into()),
        };
        config
            .backends
            .get_mut("office-chat")
            .unwrap()
            .properties
            .as_mut()
            .unwrap()
            .response_format = Some(ResponseFormat::JsonSchema {
            name: "answer".into(),
            schema: json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}),
        });
        let secrets = Secrets::new();
        let configured = config
            .validate(&protocols())
            .unwrap()
            .bind(&secrets, None)
            .unwrap();
        let adapter = configured.chat("office-chat").unwrap();
        assert_eq!(adapter.describe_debug_context().provider, "office");
        assert_eq!(
            adapter
                .count_text_stream_input_tokens(request(&config))
                .await
                .unwrap(),
            3
        );
        let output = events(adapter, request(&config)).await.unwrap();
        assert!(output
            .iter()
            .any(|e| matches!(e,EngineTextStreamEvent::TextDelta { text:t } if t==text)));
        let calls = wire.requests.lock().unwrap();
        assert_eq!(calls.len(), 2);
        for (path, headers, _) in calls.iter() {
            assert!(path.starts_with("/proxy/v1/"), "{path}");
            assert_eq!(headers["x-service-key"], "secret-office-key");
            assert!(!headers.contains_key("authorization"));
            assert!(!path.contains("secret"));
        }
        let suffix = match protocol {
            "openai-responses" => "responses/input_tokens",
            "anthropic-messages" => "messages/count_tokens",
            _ => "models/example:countTokens",
        };
        assert!(calls[0].0.ends_with(suffix), "{}", calls[0].0);
        assert_eq!(secrets.calls.load(Ordering::SeqCst), 1);
        task.abort();
    }
}

#[tokio::test]
async fn services_sharing_an_adapter_keep_authentication_continuation_and_revisions_separate() {
    let (url, wire, task) = server(vec![
        reply(completion("one")),
        reply(completion("two")),
        reply(completion("continued")),
    ])
    .await;
    let mut config = configuration("openai-chat-completions", &url);
    config
        .backends
        .get_mut("office-chat")
        .unwrap()
        .properties
        .as_mut()
        .unwrap()
        .stream = Some(false);
    let mut second = config.providers["office"].clone();
    second.chat.as_mut().unwrap().auth = Authentication::Query {
        parameter: "key".into(),
        credential: CredentialReference::Host("research-key".into()),
    };
    config.providers.insert("research".into(), second);
    let mut second = config.backends["office-chat"].clone();
    second.provider = "research".into();
    config.backends.insert("research-chat".into(), second);
    let secrets = Secrets::new();
    let configured = config
        .validate(&protocols())
        .unwrap()
        .bind(&secrets, None)
        .unwrap();
    let output = events(configured.chat("office-chat").unwrap(), request(&config))
        .await
        .unwrap();
    let native = output
        .into_iter()
        .find_map(|e| {
            if let EngineTextStreamEvent::ContentBlock {
                block: ContentBlock::NativeV2 { origin, payload },
            } = e
            {
                Some(ContentBlock::NativeV2 { origin, payload })
            } else {
                None
            }
        })
        .unwrap();
    let mut continuation = request(&config);
    continuation.shared.messages.push(Message {
        role: Role::Assistant,
        content: vec![native.clone()],
    });
    let error = events(
        configured.chat("research-chat").unwrap(),
        continuation.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(&*error.provider, "research");
    assert_eq!(error.dispatch, DispatchState::NotSent);
    events(configured.chat("research-chat").unwrap(), request(&config))
        .await
        .unwrap();
    events(
        configured.chat("office-chat").unwrap(),
        continuation.clone(),
    )
    .await
    .unwrap();
    {
        let calls = wire.requests.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].1["authorization"], "Bearer secret-office-key");
        assert!(calls[1].0.contains("key=secret-research-key"));
        assert!(!calls[1].1.contains_key("authorization"));
        assert_eq!(calls[2].2["messages"][1]["content"], "one");
    }
    assert!(!configured.snapshot().to_string().contains("secret-"));
    assert!(configured.verify_revision("another-config").is_err());
    config
        .providers
        .get_mut("office")
        .unwrap()
        .chat
        .as_mut()
        .unwrap()
        .base_url = format!("{url}different/");
    let changed = config
        .validate(&protocols())
        .unwrap()
        .bind(&secrets, None)
        .unwrap();
    assert_ne!(configured.revision(), changed.revision());
    assert!(events(changed.chat("office-chat").unwrap(), continuation)
        .await
        .is_err());
    task.abort();
}

#[tokio::test]
async fn completions_streams_validated_structured_output_and_refuses_counting_without_io() {
    let (url, wire, task) = server(vec![
        streamed_completion("{\"ok\":true}"),
        streamed_completion("not json"),
    ])
    .await;
    let mut config = configuration("openai-chat-completions", &url);
    config
        .backends
        .get_mut("office-chat")
        .unwrap()
        .properties
        .as_mut()
        .unwrap()
        .response_format = Some(ResponseFormat::JsonObject);
    let configured = config
        .validate(&protocols())
        .unwrap()
        .bind(&Secrets::new(), None)
        .unwrap();
    let adapter = configured.chat("office-chat").unwrap();
    assert!(adapter
        .count_text_stream_input_tokens(request(&config))
        .await
        .is_err());
    assert!(wire.requests.lock().unwrap().is_empty());
    events(adapter, request(&config)).await.unwrap();
    assert!(events(adapter, request(&config)).await.is_err());
    for (_, _, body) in wire.requests.lock().unwrap().iter() {
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["stream_options"]["include_usage"], true);
    }
    task.abort();
}

#[tokio::test]
async fn completions_reject_unsupported_image_formats_before_dispatch() {
    let (url, wire, task) = server(vec![]).await;
    let config = configuration("openai-chat-completions", &url);
    let configured = config
        .validate(&protocols())
        .unwrap()
        .bind(&Secrets::new(), None)
        .unwrap();
    let adapter = configured.chat("office-chat").unwrap();
    for mime in ["image/svg+xml", "image/heic", "image/bmp"] {
        for url in [
            "https://images.example/input".into(),
            format!("data:{mime};base64,aW1hZ2U="),
        ] {
            let mut req = request(&config);
            req.shared.messages[0].content.push(ContentBlock::Image {
                url,
                mime_type: mime.into(),
            });
            let error = events(adapter, req).await.unwrap_err();
            assert_eq!(error.classification, ErrorKind::InvalidRequest);
            assert_eq!(error.dispatch, DispatchState::NotSent);
        }
    }
    assert!(wire.requests.lock().unwrap().is_empty());

    for mime in ["image/png", "image/jpeg", "image/webp", "image/gif"] {
        for image_url in [
            "https://images.example/input".into(),
            format!("data:{mime};base64,aW1hZ2U="),
        ] {
            wire.replies
                .lock()
                .unwrap()
                .push_back(streamed_completion("ok"));
            let mut req = request(&config);
            req.shared.messages[0].content.push(ContentBlock::Image {
                url: image_url.clone(),
                mime_type: mime.into(),
            });
            events(adapter, req).await.unwrap();
            assert_eq!(
                wire.requests.lock().unwrap().last().unwrap().2["messages"][0]["content"][1]
                    ["image_url"]["url"],
                image_url
            );
        }
    }
    assert_eq!(wire.requests.lock().unwrap().len(), 8);
    task.abort();
}

struct Echo(Arc<AtomicUsize>);
#[async_trait]
impl EngineLocalToolExecutor for Echo {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(EngineLocalToolResult {
            content: call.arguments,
        })
    }
}
#[tokio::test]
async fn completions_retries_immutable_turns_and_executes_tools_once() {
    let first = json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"echo","arguments":"{\"value\":3}"}}]}}]});
    let (url, wire, task) = server(vec![
        (429, "application/json".into(), "{}".into()),
        reply(first),
        reply(completion("done")),
    ])
    .await;
    let mut config = configuration("openai-chat-completions", &url);
    let backend = config.backends.get_mut("office-chat").unwrap();
    backend.properties.as_mut().unwrap().stream = Some(false);
    let options = backend.options.as_mut().unwrap();
    options.max_retries = Some(1);
    options.retry_initial_delay_ms = Some(1);
    options.retry_max_delay_ms = Some(20);
    let configured = config
        .validate(&protocols())
        .unwrap()
        .bind(&Secrets::new(), None)
        .unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let mut req = request(&config);
    req.local_tools = vec![EngineLocalToolDefinition {
        name: "echo".into(),
        description: "Echo".into(),
        input_schema: json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"]}),
    }];
    req.local_tool_executor = Some(Arc::new(Echo(count.clone())));
    events(configured.chat("office-chat").unwrap(), req)
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let calls = wire.requests.lock().unwrap();
    assert_eq!(calls[0].2, calls[1].2);
    assert_eq!(calls[2].2["messages"][2]["tool_call_id"], "call-1");
    task.abort();
}

#[test]
fn invalid_declarations_fail_before_credentials_or_factories() {
    let protocols = protocols();
    let baseline = configuration("openai-responses", "http://127.0.0.1:1/prefix/v1/");
    type InvalidDeclaration = (ProviderCode, Box<dyn Fn(&mut ProviderConfiguration)>);
    let cases: Vec<InvalidDeclaration> = vec![
        (
            ProviderCode::UnknownAdapter,
            Box::new(|c| {
                c.providers
                    .get_mut("office")
                    .unwrap()
                    .chat
                    .as_mut()
                    .unwrap()
                    .adapter = "missing".into()
            }),
        ),
        (
            ProviderCode::InvalidEndpoint,
            Box::new(|c| {
                c.providers
                    .get_mut("office")
                    .unwrap()
                    .chat
                    .as_mut()
                    .unwrap()
                    .base_url = "https://key:secret@example.com/v1".into()
            }),
        ),
        (
            ProviderCode::UnsupportedCapability,
            Box::new(|c| {
                c.backends
                    .get_mut("office-chat")
                    .unwrap()
                    .capabilities
                    .supported_chat_parameters
                    .push("unknownParameter".into())
            }),
        ),
        (
            ProviderCode::InvalidOptions,
            Box::new(|c| {
                c.backends
                    .get_mut("office-chat")
                    .unwrap()
                    .options
                    .as_mut()
                    .unwrap()
                    .extra
                    .insert("unknownOption".into(), Value::Null);
            }),
        ),
        (
            ProviderCode::UnsupportedVersion,
            Box::new(|c| c.version = 999),
        ),
        (
            ProviderCode::UpgradeRequired,
            Box::new(|c| {
                c.backends.get_mut("office-chat").unwrap().endpoint =
                    Some("https://example.com/".into())
            }),
        ),
    ];
    for (code, change) in cases {
        let mut config = baseline.clone();
        change(&mut config);
        let error = config.validate(&protocols).err().unwrap();
        assert_eq!(error.code, code);
        assert!(!error.to_string().contains("secret"));
    }
    assert!(
        parse_provider_configuration("version: 1\nproviders:\n  same: {}\n  same: {}\n").is_err()
    );
    assert!(parse_provider_configuration("version: 1\nproviders:\n  ' ': {}\n").is_err());
}

#[test]
fn explicit_upgrades_preserve_source_and_require_complete_assignments() {
    let mut old = configuration("openai-responses", "https://example.com/v1/");
    old.backends.get_mut("office-chat").unwrap().endpoint =
        Some("https://example.com/v1/responses".into());
    let before = serde_json::to_value(&old).unwrap();
    assert!(upgrade_legacy_providers(&old.backends, &BTreeMap::new(), &BTreeMap::new()).is_err());
    let assignment = LegacyProviderAssignment {
        provider: "office".into(),
        binding: old.providers["office"].chat.clone().unwrap(),
    };
    let upgraded = upgrade_legacy_providers(
        &old.backends,
        &BTreeMap::new(),
        &BTreeMap::from([("chat/office-chat".into(), assignment)]),
    )
    .unwrap();
    upgraded.validate(&protocols()).unwrap();
    assert_eq!(serde_json::to_value(&old).unwrap(), before);
    let origin = ProviderContentOrigin {
        provider: "office".into(),
        adapter: "openai-responses".into(),
        binding_revision: "accepted".into(),
    };
    let message = Message {
        role: Role::Assistant,
        content: vec![ContentBlock::Native {
            provider: "openai".into(),
            payload: json!({"signature":"opaque"}),
        }],
    };
    let upgraded = upgrade_provider_content(&message, "openai", &origin).unwrap();
    assert!(matches!(&upgraded.content[0],ContentBlock::NativeV2 { origin:o,.. } if o==&origin));
    assert!(matches!(&message.content[0], ContentBlock::Native { .. }));
}

fn media_configuration(protocol: &str, url: &str) -> ProviderConfiguration {
    let protocols = protocols();
    let descriptor = protocols.adapter(protocol).unwrap().media.as_ref().unwrap();
    let mut config = configuration("mock", url);
    let mut binding = config.providers["office"].chat.clone().unwrap();
    binding.adapter = protocol.into();
    binding.auth = Authentication::Query {
        parameter: "token".into(),
        credential: CredentialReference::Host("media".into()),
    };
    if protocol == "runway-tasks" {
        binding
            .headers
            .insert("x-runway-version".into(), "2024-11-06".into());
    }
    config.providers.get_mut("office").unwrap().media = Some(binding);
    let options = match protocol {
        "openai-images" => {
            json!({"timeoutMs":3000,"defaultCount":1,"partialImages":0,"quality":"high","outputFormat":"png"})
        }
        "google-interactions" => {
            json!({"timeoutMs":3000,"defaultMimeType":"image/png","imageSize":"2K"})
        }
        "runway-tasks" => {
            json!({"timeoutMs":3000,"pollIntervalMs":1,"maxPollRetryCount":0,"maxPromptChars":1000,"defaultAspectRatio":"1024:1024"})
        }
        _ => {
            json!({"timeoutMs":3000,"pollIntervalMs":1,"maxPollRetryCount":0,"inputFields":{"prompt":"prompt"}})
        }
    };
    config.media_backends.insert("office-image".into(), serde_json::from_value(json!({
        "provider":"office","model":"owner/model","capability":"image-generation","options":options,
        "capabilities":{"supportedMediaParameters":descriptor.parameters,"aspectRatioOptions":[],"sizeOptions":[],"supportsPartialImageStreaming":false,"estimatedGenerationSeconds":1}
    })).unwrap());
    config
}

#[test]
fn media_output_byte_limit_is_validated_before_credentials_and_factories() {
    let builtins = protocols();
    let limit = battersea_model::adapter::payload::PROVIDER_PAYLOAD_BYTES as u64;
    for protocol in ["runway-tasks", "replicate-predictions"] {
        let constructions = Arc::new(AtomicUsize::new(0));
        let mut registry = ProtocolRegistry::default();
        for id in ["mock", protocol] {
            let mut descriptor = builtins.adapter(id).unwrap().as_ref().clone();
            if id == "mock" {
                let original = descriptor.chat.as_ref().unwrap().factory.clone();
                let count = constructions.clone();
                descriptor.chat.as_mut().unwrap().factory = Arc::new(move |backend, logger| {
                    count.fetch_add(1, Ordering::SeqCst);
                    original(backend, logger)
                });
            } else {
                let original = descriptor.media.as_ref().unwrap().factory.clone();
                let count = constructions.clone();
                descriptor.media.as_mut().unwrap().factory = Arc::new(move |backend| {
                    count.fetch_add(1, Ordering::SeqCst);
                    original(backend)
                });
            }
            registry.register(descriptor).unwrap();
        }
        let secrets = Secrets::new();
        let mut config = media_configuration(protocol, "https://media.example/v1/");
        for bytes in [0, limit + 1, u64::MAX] {
            config
                .media_backends
                .get_mut("office-image")
                .unwrap()
                .options
                .as_mut()
                .unwrap()
                .extra
                .insert("maxOutputBytes".into(), json!(bytes));
            let result = config
                .validate(&registry)
                .and_then(|valid| valid.bind(&secrets, None));
            let error = result.err().unwrap();
            assert_eq!(error.code, ProviderCode::InvalidOptions);
            assert!(error.path.starts_with("mediaBackends.office-image"));
            assert_eq!(secrets.calls.load(Ordering::SeqCst), 0);
            assert_eq!(constructions.load(Ordering::SeqCst), 0);
        }
        config
            .media_backends
            .get_mut("office-image")
            .unwrap()
            .options
            .as_mut()
            .unwrap()
            .extra
            .insert("maxOutputBytes".into(), json!(limit));
        config
            .validate(&registry)
            .unwrap()
            .bind(&secrets, None)
            .unwrap();
        assert_eq!(secrets.calls.load(Ordering::SeqCst), 2);
        assert_eq!(constructions.load(Ordering::SeqCst), 2);
    }
}

fn image_request() -> MediaRenderRequest {
    MediaRenderRequest {
        kind: MediaKind::Image,
        prompt_text: "Test image".into(),
        negative_prompt: None,
        references: vec![],
        options: MediaGenerationHints::default(),
    }
}

#[tokio::test]
async fn media_uses_its_own_binding_for_submission_polling_retrieval_and_cancellation() {
    for protocol in [
        "openai-images",
        "google-interactions",
        "runway-tasks",
        "replicate-predictions",
    ] {
        let (url, wire, task) = server(vec![]).await;
        let remote = matches!(protocol, "runway-tasks" | "replicate-predictions");
        let replies = match protocol {
            "openai-images" => vec![reply(json!({"created":1,"data":[{"b64_json":"aW1hZ2U="}]}))],
            "google-interactions" => vec![reply(
                json!({"id":"image-1","status":"completed","output_image":{"mime_type":"image/png","data":"aW1hZ2U="}}),
            )],
            "runway-tasks" => vec![
                reply(json!({"id":"job-1"})),
                reply(json!({"id":"job-1","status":"SUCCEEDED","output":[format!("{url}asset")]})),
                (200, "image/png".into(), "image".into()),
                reply(json!({"id":"job-2"})),
                (204, "application/json".into(), String::new()),
            ],
            _ => vec![
                reply(json!({"id":"job-1","status":"starting"})),
                reply(json!({"id":"job-1","status":"succeeded","output":[format!("{url}asset")]})),
                (200, "image/png".into(), "image".into()),
                reply(json!({"id":"job-2","status":"starting"})),
                reply(json!({"id":"job-2","status":"canceled"})),
            ],
        };
        *wire.replies.lock().unwrap() = replies.into();
        let config = media_configuration(protocol, &url);
        let credentials = Secrets::new();
        let configured = config
            .validate(&protocols())
            .unwrap()
            .bind(&credentials, None)
            .unwrap();
        let adapter = configured.media("office-image").unwrap();
        assert_eq!(adapter.backend().provider, "office");
        let result = adapter
            .generate(image_request(), CancellationToken::new(), None, None)
            .await
            .unwrap();
        assert_eq!(result.assets.len(), 1);
        assert!(result.assets[0].url.starts_with("data:image/png;base64,"));
        if remote {
            let MediaSubmission::Pending(mut job) = adapter
                .submit(
                    adapter.prepare(image_request()).unwrap(),
                    CancellationToken::new(),
                    None,
                    None,
                )
                .await
                .unwrap()
            else {
                panic!("expected remote job")
            };
            assert_eq!(job.provider(), "office");
            assert_eq!(job.id(), "job-2");
            let rotated = config
                .validate(&protocols())
                .unwrap()
                .bind(
                    &FixedCredential {
                        value: "rotated",
                        scope: "account-2",
                    },
                    None,
                )
                .unwrap();
            assert_ne!(rotated.revision(), configured.revision());
            let stale = adapter.prepare(image_request()).unwrap();
            assert!(rotated
                .media("office-image")
                .unwrap()
                .submit(stale, CancellationToken::new(), None, None)
                .await
                .is_err());
            job.cancel().await.unwrap();
        }
        let calls = wire.requests.lock().unwrap();
        assert_eq!(calls.len(), if remote { 5 } else { 1 });
        for (path, headers, _) in calls.iter() {
            assert!(path.starts_with("/proxy/v1/"), "{path}");
            assert!(path.contains("token=secret-media"), "{path}");
            assert!(!headers.contains_key("authorization"));
        }
        assert_eq!(credentials.calls.load(Ordering::SeqCst), 2);
        task.abort();
    }
}

#[test]
fn authored_configuration_and_safe_export_match_the_schema() {
    let schema = compose_backend_schema(json!({"$ref":"#/$defs/providerConfiguration"})).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let config = parse_provider_configuration(include_str!("../providers.yaml")).unwrap();
    assert!(validator.is_valid(&serde_json::to_value(&config).unwrap()));
    let configured = config
        .validate(&protocols())
        .unwrap()
        .bind(&Secrets::new(), None)
        .unwrap();
    assert!(validator.is_valid(&configured.snapshot()["configuration"]));
    let reloaded: ProviderConfiguration =
        serde_json::from_value(configured.snapshot()["configuration"].clone()).unwrap();
    let rebound = reloaded
        .validate(&protocols())
        .unwrap()
        .bind(&Secrets::new(), None)
        .unwrap();
    assert_eq!(rebound.revision(), configured.revision());
}

#[tokio::test]
async fn application_adapter_registration_is_pinned_and_validated_before_construction() {
    let builtins = protocols();
    let mut custom = builtins.adapter("mock").unwrap().as_ref().clone();
    custom.id = "application-text".into();
    let constructions = Arc::new(AtomicUsize::new(0));
    let count = constructions.clone();
    let original = custom.chat.as_ref().unwrap().factory.clone();
    custom.chat.as_mut().unwrap().factory = Arc::new(move |backend, logger| {
        count.fetch_add(1, Ordering::SeqCst);
        original(backend, logger)
    });
    let mut protocols = ProtocolRegistry::default();
    protocols.register(custom.clone()).unwrap();
    assert_eq!(
        protocols.register(custom).unwrap_err().code,
        ProviderCode::DuplicateIdentity
    );
    let mut config = configuration("mock", "");
    let binding = config
        .providers
        .get_mut("office")
        .unwrap()
        .chat
        .as_mut()
        .unwrap();
    binding.adapter = "application-text".into();
    binding.auth = Authentication::None;
    let valid = config.validate(&protocols).unwrap();
    assert_eq!(constructions.load(Ordering::SeqCst), 0);
    drop(protocols);
    let secrets = Secrets::new();
    let configured = valid.bind(&secrets, None).unwrap();
    assert_eq!(constructions.load(Ordering::SeqCst), 1);
    assert_eq!(secrets.calls.load(Ordering::SeqCst), 0);
    let mut req = request(&config);
    req.mock_response = Some(Arc::new(|_| "custom reply".into()));
    assert!(events(configured.chat("office-chat").unwrap(), req)
        .await
        .unwrap()
        .iter()
        .any(|e| matches!(e,EngineTextStreamEvent::TextDelta { text } if text.contains("custom"))));
}

#[test]
fn application_adapter_required_headers_match_case_insensitively() {
    let builtins = protocols();
    let mut custom = builtins.adapter("mock").unwrap().as_ref().clone();
    custom.id = "application-text".into();
    custom.required_headers = vec!["X-Protocol-Version".into()];
    let constructions = Arc::new(AtomicUsize::new(0));
    let count = constructions.clone();
    let original = custom.chat.as_ref().unwrap().factory.clone();
    custom.chat.as_mut().unwrap().factory = Arc::new(move |backend, logger| {
        count.fetch_add(1, Ordering::SeqCst);
        original(backend, logger)
    });
    let mut registry = ProtocolRegistry::default();
    registry.register(custom).unwrap();
    let secrets = Secrets::new();
    let mut config = configuration("mock", "");
    config
        .providers
        .get_mut("office")
        .unwrap()
        .chat
        .as_mut()
        .unwrap()
        .adapter = "application-text".into();
    for header in [
        "x-protocol-version",
        "X-Protocol-Version",
        "X-PROTOCOL-VERSION",
    ] {
        config
            .providers
            .get_mut("office")
            .unwrap()
            .chat
            .as_mut()
            .unwrap()
            .headers = BTreeMap::from([(header.into(), "1".into())]);
        config
            .validate(&registry)
            .unwrap()
            .bind(&secrets, None)
            .unwrap();
    }
    assert_eq!(secrets.calls.load(Ordering::SeqCst), 3);
    assert_eq!(constructions.load(Ordering::SeqCst), 3);
    config
        .providers
        .get_mut("office")
        .unwrap()
        .chat
        .as_mut()
        .unwrap()
        .headers
        .clear();
    let result = config
        .validate(&registry)
        .and_then(|valid| valid.bind(&secrets, None));
    assert_eq!(
        result.err().unwrap().code,
        ProviderCode::InvalidAuthentication
    );
    assert_eq!(secrets.calls.load(Ordering::SeqCst), 3);
    assert_eq!(constructions.load(Ordering::SeqCst), 3);
}

#[derive(Default)]
struct Logs(Mutex<Vec<Value>>);
#[async_trait]
impl EngineAdapterLogger for Logs {
    async fn on_request(&self, entry: EngineAdapterRequestLog) {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::to_value(entry).unwrap());
    }
    async fn on_response(&self, entry: EngineAdapterResponseLog) {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::to_value(entry).unwrap());
    }
    async fn on_error(&self, entry: EngineAdapterErrorLog) {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::to_value(entry).unwrap());
    }
}
struct FixedCredential {
    value: &'static str,
    scope: &'static str,
}
impl CredentialResolver for FixedCredential {
    fn resolve(
        &self,
        _: &CredentialReference,
    ) -> Result<ResolvedCredential, ProviderConfigurationError> {
        Ok(ResolvedCredential {
            value: self.value.into(),
            scope_revision: self.scope.into(),
        })
    }
}
#[tokio::test]
async fn credentials_are_redacted_and_rotation_requires_a_new_binding() {
    let (url, wire, task) = server(vec![
        (
            401,
            "application/json".into(),
            json!({"error":{"message":"rejected key A+b/ c or A%2Bb%2F+c"}}).to_string(),
        ),
        reply(completion("ok")),
    ])
    .await;
    let mut config = configuration("openai-chat-completions", &url);
    config
        .backends
        .get_mut("office-chat")
        .unwrap()
        .properties
        .as_mut()
        .unwrap()
        .stream = Some(false);
    let logs = Arc::new(Logs::default());
    let configured = config
        .validate(&protocols())
        .unwrap()
        .bind(
            &FixedCredential {
                value: "A+b/ c",
                scope: "account-1",
            },
            Some(logs.clone()),
        )
        .unwrap();
    let error = events(configured.chat("office-chat").unwrap(), request(&config))
        .await
        .unwrap_err();
    assert_eq!(&*error.provider, "office");
    assert_eq!(error.classification, ErrorKind::Authentication);
    let diagnostics = format!(
        "{error:?} {:?} {:?}",
        configured.snapshot(),
        logs.0.lock().unwrap()
    );
    for secret in ["A+b/ c", "A%2Bb%2F+c"] {
        assert!(!diagnostics.contains(secret), "{diagnostics}");
    }
    assert!(logs
        .0
        .lock()
        .unwrap()
        .iter()
        .all(|entry| entry["context"]["provider"] == "office"));
    let rotated = config
        .validate(&protocols())
        .unwrap()
        .bind(
            &FixedCredential {
                value: "new-token",
                scope: "account-2",
            },
            None,
        )
        .unwrap();
    assert_ne!(rotated.revision(), configured.revision());
    events(configured.chat("office-chat").unwrap(), request(&config))
        .await
        .unwrap();
    assert_eq!(
        wire.requests.lock().unwrap()[1].1["authorization"],
        "Bearer A+b/ c"
    );
    assert_eq!(
        config
            .validate(&protocols())
            .unwrap()
            .bind(
                &FixedCredential {
                    value: "invalid\nheader",
                    scope: "account-1"
                },
                None
            )
            .err()
            .unwrap()
            .code,
        ProviderCode::InvalidAuthentication
    );
    task.abort();
}

#[tokio::test]
async fn incomplete_streams_and_server_failures_do_not_replay_generation() {
    for response in [
        (500, "application/json".into(), "{}".into()),
        sse(
            vec![
                json!({"choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":null}]}),
            ],
            false,
        ),
    ] {
        let (url, wire, task) = server(vec![response]).await;
        let mut config = configuration("openai-chat-completions", &url);
        let options = config
            .backends
            .get_mut("office-chat")
            .unwrap()
            .options
            .as_mut()
            .unwrap();
        options.max_retries = Some(2);
        options.retry_initial_delay_ms = Some(1);
        options.retry_max_delay_ms = Some(10);
        let configured = config
            .validate(&protocols())
            .unwrap()
            .bind(&Secrets::new(), None)
            .unwrap();
        let error = events(configured.chat("office-chat").unwrap(), request(&config))
            .await
            .unwrap_err();
        assert_ne!(error.dispatch, DispatchState::NotSent);
        assert_eq!(wire.requests.lock().unwrap().len(), 1);
        task.abort();
    }
}

#[tokio::test]
async fn background_cleanup_keeps_query_authentication_and_prefix() {
    let (url, wire, task) = server(vec![
        reply(json!({"id":"response-1","status":"queued"})),
        reply(json!({"id":"response-1","status":"in_progress"})),
        reply(json!({"id":"response-1","status":"cancelled"})),
    ])
    .await;
    let mut config = configuration("openai-responses", &url);
    config
        .providers
        .get_mut("office")
        .unwrap()
        .chat
        .as_mut()
        .unwrap()
        .auth = Authentication::Query {
        parameter: "token".into(),
        credential: CredentialReference::Host("office-key".into()),
    };
    let options = config
        .backends
        .get_mut("office-chat")
        .unwrap()
        .options
        .as_mut()
        .unwrap();
    options.background = Some(true);
    options.background_mode = Some("poll".into());
    options.background_poll_interval_ms = Some(200);
    options.background_max_wait_ms = Some(3000);
    let configured = config
        .validate(&protocols())
        .unwrap()
        .bind(&Secrets::new(), None)
        .unwrap();
    let stream = configured
        .chat("office-chat")
        .unwrap()
        .stream_text(request(&config))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while wire.requests.lock().unwrap().len() < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    drop(stream);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while wire.requests.lock().unwrap().len() < 3 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        wire.requests.lock().unwrap()[2].0,
        "/proxy/v1/responses/response-1/cancel?token=secret-office-key"
    );
    task.abort();
}

#[tokio::test]
async fn fragmented_completion_tools_are_validated_before_execution_and_retained_on_retry() {
    for argument in ["3", "\"wrong type\""] {
        let first = sse(
            vec![
                json!({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"echo","arguments":"{\"value\":"}}]},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":format!("{argument}}}")}}]},"finish_reason":"tool_calls"}]}),
            ],
            true,
        );
        let mut replies = vec![first];
        if argument == "3" {
            replies.extend([
                (429, "application/json".into(), "{}".into()),
                streamed_completion("done"),
            ]);
        }
        let (url, wire, task) = server(replies).await;
        let mut config = configuration("openai-chat-completions", &url);
        let options = config
            .backends
            .get_mut("office-chat")
            .unwrap()
            .options
            .as_mut()
            .unwrap();
        options.max_retries = Some(1);
        options.retry_initial_delay_ms = Some(1);
        options.retry_max_delay_ms = Some(20);
        let configured = config
            .validate(&protocols())
            .unwrap()
            .bind(&Secrets::new(), None)
            .unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let mut req = request(&config);
        req.local_tools = vec![EngineLocalToolDefinition {
            name: "echo".into(),
            description: "Echo".into(),
            input_schema: json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"]}),
        }];
        req.local_tool_executor = Some(Arc::new(Echo(count.clone())));
        let result = events(configured.chat("office-chat").unwrap(), req).await;
        let calls = wire.requests.lock().unwrap();
        if argument == "3" {
            result.unwrap();
            assert_eq!(count.load(Ordering::SeqCst), 1);
            assert_eq!(calls.len(), 3);
            assert_eq!(calls[1].2, calls[2].2);
        } else {
            assert!(result.is_err());
            assert_eq!(count.load(Ordering::SeqCst), 0);
            assert_eq!(calls.len(), 1);
        }
        task.abort();
    }
}

#[tokio::test]
async fn configured_google_service_accepts_protocol_owned_storage_references() {
    let (url, wire, task) = server(vec![reply(json!({"totalTokens":8}))]).await;
    let config = configuration("google-generate-content", &url);
    let configured = config
        .validate(&protocols())
        .unwrap()
        .bind(&Secrets::new(), None)
        .unwrap();
    let mut req = request(&config);
    req.shared.messages[0].content.push(ContentBlock::Image {
        url: "gs://application-bucket/input.png".into(),
        mime_type: "image/png".into(),
    });
    assert_eq!(
        configured
            .chat("office-chat")
            .unwrap()
            .count_text_stream_input_tokens(req)
            .await
            .unwrap(),
        8
    );
    assert!(wire.requests.lock().unwrap()[0]
        .2
        .to_string()
        .contains("gs://application-bucket/input.png"));
    task.abort();
}

#[test]
fn missing_credentials_and_unsupported_modes_fail_during_binding_or_validation() {
    let mut config = configuration("openai-chat-completions", "https://service.example/v1/");
    let binding = config
        .providers
        .get_mut("office")
        .unwrap()
        .chat
        .as_mut()
        .unwrap();
    binding.auth = Authentication::Bearer {
        credential: CredentialReference::Env("BATTERSEA_ISSUE_5_MISSING_CREDENTIAL_7F1A".into()),
    };
    let error = config
        .validate(&protocols())
        .unwrap()
        .bind(
            &EnvironmentCredentials {
                scope_revision: "account-1".into(),
            },
            None,
        )
        .err()
        .unwrap();
    assert_eq!(error.code, ProviderCode::MissingCredential);
    assert_eq!(error.provider.as_deref(), Some("office"));
    config
        .backends
        .get_mut("office-chat")
        .unwrap()
        .properties
        .as_mut()
        .unwrap()
        .top_k = Some(3);
    assert_eq!(
        config.validate(&protocols()).err().unwrap().code,
        ProviderCode::UnsupportedCapability
    );
    config
        .backends
        .get_mut("office-chat")
        .unwrap()
        .properties
        .as_mut()
        .unwrap()
        .top_k = None;
    config
        .backends
        .get_mut("office-chat")
        .unwrap()
        .properties
        .as_mut()
        .unwrap()
        .context_overflow = Some(EngineContextOverflow::ProviderTruncate);
    assert_eq!(
        config.validate(&protocols()).err().unwrap().code,
        ProviderCode::UnsupportedCapability
    );
}

#[test]
fn yaml_credentials_use_the_documented_mapping_form() {
    for (kind, expected) in [
        ("env", CredentialReference::Env("account-key".into())),
        ("host", CredentialReference::Host("account-key".into())),
    ] {
        let source=format!("version: 1\nproviders:\n  service:\n    chat:\n      adapter: mock\n      baseUrl: ''\n      auth:\n        type: bearer\n        credential: {{{kind}: account-key}}\n");
        let config = parse_provider_configuration(&source).unwrap();
        assert_eq!(
            config.providers["service"]
                .chat
                .as_ref()
                .unwrap()
                .auth
                .credential(),
            Some(&expected)
        );
    }
}
