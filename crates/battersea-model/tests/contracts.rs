use async_trait::async_trait;
use battersea_model::{engine::*, media::*, *};
use futures_util::{stream, StreamExt};
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

fn backend() -> EngineBackendConfig {
    serde_json::from_value(json!({
        "id":"contract", "provider":"external", "label":"Contract", "enabled":true,
        "endpoint":"", "model":"configured", "chat":EngineChatParameters::default_for_provider("external"),
        "capabilities":EngineBackendCapabilities::mock(), "context_window_tokens":8192,
        "options":{"timeoutMs":20}, "auth":{"auth_type":"none","api_key_env":"","header":null,"version_header":null,"version":null,"has_api_key":false}
    })).unwrap()
}
fn request(backend: &EngineBackendConfig) -> EngineTextStreamRequest {
    EngineTextStreamRequest {
        shared: EngineAdapterRequest {
            operation: EngineOperation::FlowTextStream,
            messages: vec![Message::text(Role::User, "input".into())],
            chat: backend.chat.clone(),
            debug_rules: None,
        },
        mock_response: None,
        local_tools: vec![],
        local_tool_executor: None,
        max_tool_rounds: 2,
    }
}
struct Transport(Arc<AtomicUsize>);
#[async_trait]
impl EngineAdapter for Transport {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        EngineAdapterDebugContext {
            provider: "external".into(),
            backend: "contract".into(),
            model: "configured".into(),
            sdk: "fixture".into(),
            base_url: None,
            timeout_ms: Some(20),
            max_retries: Some(0),
        }
    }
    async fn count_text_stream_input_tokens(
        &self,
        _: EngineTextStreamRequest,
    ) -> Result<u64, Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::pending().await
    }
    async fn stream_text(&self, _: EngineTextStreamRequest) -> Result<EngineTextStream, Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(stream::pending()))
    }
}
/// A clock no test advances unless it exercises a deadline.
fn clock() -> Arc<dyn Clock> {
    Arc::new(ManualClock::new(0))
}
fn registry(calls: Arc<AtomicUsize>) -> ProviderRegistry {
    let mut registry = ProviderRegistry::default();
    registry
        .register_chat(
            "external",
            ChatProvider {
                content: crate::ContentCapabilities::text(),
                background_modes: vec![],
                parameters: backend().capabilities.supported_chat_parameters,
                factory: Arc::new(move |_, _, _| Ok(Arc::new(Transport(calls.clone())))),
            },
        )
        .unwrap();
    registry
}
#[tokio::test]
async fn open_provider_admission_rejects_modalities_and_modes_before_transport() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = registry(calls.clone());
    let backend = backend();
    let adapter = registry
        .create_chat(backend.clone(), None, clock())
        .unwrap();
    let mut invalid = request(&backend);
    invalid.shared.messages[0].content = vec![ContentBlock::Image {
        url: "fixture".into(),
        mime_type: "image/png".into(),
    }];
    assert!(matches!(
        adapter
            .stream_text(invalid.clone())
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::InvalidRequest
    ));
    assert!(matches!(
        adapter
            .count_text_stream_input_tokens(invalid)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    ));
    let mut invalid = request(&backend);
    invalid.shared.chat.stream = false;
    assert!(matches!(
        adapter.stream_text(invalid).await.err().unwrap().kind,
        ErrorKind::InvalidRequest
    ));
    let mut invalid = request(&backend);
    invalid.local_tools.push(EngineLocalToolDefinition {
        name: "fixture".into(),
        description: "Fixture".into(),
        input_schema: json!({"type":"not-a-schema-type"}),
    });
    assert!(adapter
        .count_text_stream_input_tokens(invalid)
        .await
        .is_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "admission must precede transport"
    );
}
#[test]
fn registry_rejects_unknown_or_inconsistent_backend_before_factory() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = registry(calls);
    let valid = backend();
    for mutate in [
        |b: &mut EngineBackendConfig| b.provider = "unregistered".into(),
        |b: &mut EngineBackendConfig| b.model.clear(),
        |b: &mut EngineBackendConfig| b.id.clear(),
        |b: &mut EngineBackendConfig| b.enabled = false,
        |b: &mut EngineBackendConfig| b.options.timeout_ms = Some(0),
        |b: &mut EngineBackendConfig| {
            b.options.background = Some(true);
            b.options.background_mode = Some("poll".into());
        },
        |b: &mut EngineBackendConfig| {
            b.capabilities
                .supported_chat_parameters
                .push("unimplemented".into())
        },
    ] {
        let mut invalid = valid.clone();
        mutate(&mut invalid);
        assert!(matches!(
            registry
                .create_chat(invalid, None, clock())
                .err()
                .unwrap()
                .kind,
            ErrorKind::InvalidRequest
        ));
    }
}
/// Replays its events, then ends the stream or, when `stall` is set, leaves it open.
struct Scripted {
    events: Vec<Result<EngineTextStreamEvent, Failure>>,
    stall: bool,
}
#[async_trait]
impl EngineAdapter for Scripted {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        Transport(Default::default()).describe_debug_context()
    }
    async fn count_text_stream_input_tokens(
        &self,
        _: EngineTextStreamRequest,
    ) -> Result<u64, Failure> {
        Ok(0)
    }
    fn validate_conversation(
        &self,
        _: &ConversationRequest,
        _: Option<&serde_json::Value>,
    ) -> Result<ConversationBinding, Failure> {
        Ok(serde_json::from_value(json!({
            "origin": {"provider": "external", "adapter": "fixture", "bindingRevision": "fixture"},
            "configuration_revision": "fixture",
            "timeout_ms": 20,
        }))
        .unwrap())
    }
    /// A completed turn holding the scripted events; a scripted failure fails the turn.
    async fn conversation_turn(
        &self,
        _: ConversationRequest,
        _: Option<serde_json::Value>,
        _: Vec<ConversationToolResult>,
    ) -> Result<ConversationTurn, Failure> {
        Ok(ConversationTurn {
            state: serde_json::Value::Null,
            calls: vec![],
            events: self.events.iter().cloned().collect::<Result<_, _>>()?,
        })
    }
    async fn stream_text(&self, _: EngineTextStreamRequest) -> Result<EngineTextStream, Failure> {
        let events = stream::iter(self.events.clone());
        Ok(if self.stall {
            Box::pin(events.chain(stream::pending()))
        } else {
            Box::pin(events)
        })
    }
}
/// contracts/failures.md, Identity: a failure carries the identifiers of its own exchange.
#[tokio::test]
async fn a_later_turns_failure_does_not_carry_an_earlier_turns_identifiers() {
    let opened = |request: &str, response: Option<&str>| {
        Ok(EngineTextStreamEvent::ResponseOpened {
            request_id: Some(request.into()),
            response_id: response.map(Into::into),
        })
    };
    let rejected = || Err(Failure::new("external", "Rejected.", ErrorKind::Server));
    let first = vec![
        Ok(EngineTextStreamEvent::MessageStart { turn_index: 0 }),
        opened("request-1", Some("response-1")),
    ];
    for (second, request_id) in [
        (vec![rejected()], None),
        (
            vec![opened("request-2", None), rejected()],
            Some("request-2"),
        ),
    ] {
        let mut script = first.clone();
        script.push(Ok(EngineTextStreamEvent::MessageStart { turn_index: 1 }));
        script.extend(second);
        let mut registry = ProviderRegistry::default();
        registry
            .register_chat(
                "external",
                ChatProvider {
                    content: crate::ContentCapabilities::text(),
                    background_modes: vec![],
                    parameters: backend().capabilities.supported_chat_parameters,
                    factory: Arc::new(move |_, _, _| {
                        Ok(Arc::new(Scripted {
                            events: script.clone(),
                            stall: false,
                        }))
                    }),
                },
            )
            .unwrap();
        let events: Vec<_> = registry
            .create_chat(backend(), None, clock())
            .unwrap()
            .stream_text(request(&backend()))
            .await
            .unwrap()
            .collect()
            .await;
        let failure = events.last().unwrap().as_ref().unwrap_err();
        assert_eq!(failure.exchange.request_id.as_deref(), request_id);
        assert_eq!(failure.exchange.response_id, None);
    }
}
#[derive(Default)]
struct Reported(std::sync::Mutex<Vec<Failure>>);
#[async_trait]
impl EngineAdapterLogger for Reported {
    async fn on_error(&self, entry: EngineAdapterErrorLog) {
        self.0.lock().unwrap().push(entry.detail);
    }
}
/// contracts/failures.md, Identity and Partial output: however a text stream fails after its
/// response headers, the failure carries the origin, the issued identifiers and the structured
/// text Battersea held, and the observer receives that failure once.
#[tokio::test]
async fn every_stream_failure_carries_origin_identifiers_and_held_text() {
    enum Ends {
        Script,
        Deadline,
        Idle,
    }
    struct Case {
        name: &'static str,
        format: Option<ResponseFormat>,
        modalities: Option<Vec<Modality>>,
        /// Text the stream delivers before `tail`; Battersea holds it under a format.
        text: &'static str,
        tail: Vec<Result<EngineTextStreamEvent, Failure>>,
        ends: Ends,
        kind: ErrorKind,
    }
    let schema = || {
        Some(ResponseFormat::JsonSchema {
            name: "result".into(),
            schema: json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]}),
        })
    };
    let stop = |class| {
        Ok(EngineTextStreamEvent::MessageStop {
            class,
            native: json!({"stop_reason": "fixture"}),
        })
    };
    let case = |name, text, tail, kind| Case {
        name,
        format: schema(),
        modalities: None,
        text,
        tail,
        ends: Ends::Script,
        kind,
    };
    let cases = vec![
        case(
            "provider error",
            "{\"ok\":",
            vec![Err(Failure::new(
                "external",
                "Rejected.",
                ErrorKind::Server,
            ))],
            ErrorKind::Server,
        ),
        case(
            "stream ends",
            "{\"ok\":",
            vec![],
            ErrorKind::InvalidResponse,
        ),
        case(
            "refusal",
            "{\"ok\":",
            vec![stop(StopClass::Refusal)],
            ErrorKind::Refusal,
        ),
        case(
            "truncation",
            "{\"ok\":",
            vec![stop(StopClass::Length)],
            ErrorKind::Incomplete,
        ),
        case(
            "unrecognised stop",
            "{\"ok\":true}",
            vec![stop(StopClass::Other)],
            ErrorKind::Incomplete,
        ),
        case(
            "malformed JSON",
            "{\"ok\":",
            vec![stop(StopClass::Complete)],
            ErrorKind::InvalidResponse,
        ),
        case(
            "schema violation",
            "{\"ok\":3}",
            vec![stop(StopClass::Complete)],
            ErrorKind::InvalidResponse,
        ),
        Case {
            format: Some(ResponseFormat::JsonObject),
            ..case(
                "not an object",
                "[1]",
                vec![stop(StopClass::Complete)],
                ErrorKind::InvalidResponse,
            )
        },
        case(
            "byte limit",
            "{\"ok\":",
            vec![Ok(EngineTextStreamEvent::TextDelta {
                text: " ".repeat(battersea_model::adapter::payload::PROVIDER_PAYLOAD_BYTES),
            })],
            ErrorKind::InvalidResponse,
        ),
        case(
            "unrequested modality",
            "{\"ok\":",
            vec![Ok(EngineTextStreamEvent::ContentBlock {
                block: ContentBlock::Image {
                    url: "data:image/png;base64,AA==".into(),
                    mime_type: "image/png".into(),
                },
            })],
            ErrorKind::InvalidResponse,
        ),
        Case {
            format: None,
            modalities: Some(vec![Modality::Image]),
            ..case(
                "unrequested text",
                "text",
                vec![],
                ErrorKind::InvalidResponse,
            )
        },
        Case {
            ends: Ends::Deadline,
            ..case("request deadline", "{\"ok\":", vec![], ErrorKind::Timeout)
        },
        Case {
            ends: Ends::Idle,
            ..case("idle watchdog", "{\"ok\":", vec![], ErrorKind::Timeout)
        },
    ];
    for case in cases {
        let name = case.name;
        let mut backend = backend();
        let capabilities = &mut backend.capabilities;
        capabilities.content.structured_output = vec![
            StructuredOutputMode::JsonObject,
            StructuredOutputMode::JsonSchema,
        ];
        capabilities.content.output_modalities = vec![Modality::Text, Modality::Image];
        for parameter in ["responseFormat", "outputModalities"] {
            if !capabilities
                .supported_chat_parameters
                .iter()
                .any(|held| held == parameter)
            {
                capabilities
                    .supported_chat_parameters
                    .push(parameter.into());
            }
        }
        if matches!(case.ends, Ends::Idle) {
            backend.options.stream_idle_timeout_ms = Some(10);
        }
        let mut events = vec![
            Ok(EngineTextStreamEvent::MessageStart { turn_index: 0 }),
            Ok(EngineTextStreamEvent::ResponseOpened {
                request_id: Some("request-1".into()),
                response_id: Some("response-1".into()),
            }),
            Ok(EngineTextStreamEvent::TextDelta {
                text: case.text.into(),
            }),
        ];
        events.extend(case.tail);
        let stall = !matches!(case.ends, Ends::Script);
        let mut registry = ProviderRegistry::default();
        registry
            .register_chat(
                "external",
                ChatProvider {
                    content: backend.capabilities.content.clone(),
                    background_modes: vec![],
                    parameters: backend.capabilities.supported_chat_parameters.clone(),
                    factory: Arc::new(move |_, _, _| {
                        Ok(Arc::new(Scripted {
                            events: events.clone(),
                            stall,
                        }))
                    }),
                },
            )
            .unwrap();
        let reported = Arc::new(Reported::default());
        let clock = ManualClock::new(0);
        let mut request = request(&backend);
        request.shared.chat.response_format = case.format.clone();
        request.shared.chat.output_modalities = case.modalities;
        let stream = registry
            .create_chat(backend, Some(reported.clone()), Arc::new(clock.clone()))
            .unwrap()
            .stream_text(request)
            .await
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let window = Duration::from_millis(match case.ends {
            Ends::Idle => 10,
            _ => 20,
        });
        let events: Vec<_> = if stall {
            let (events, remaining) =
                tokio::join!(stream.collect(), clock.advance_to_timer_within(window));
            assert_eq!(remaining, window, "{name}");
            events
        } else {
            stream.collect().await
        };
        let failure = events.last().unwrap().as_ref().expect_err(name).clone();
        assert_eq!(failure.kind, case.kind, "{name}");
        assert_eq!(
            failure.origin,
            Origin {
                operation: Some("stream_text".into()),
                provider: Some("external".into()),
                backend: Some("contract".into()),
                model: Some("configured".into()),
            },
            "{name}"
        );
        assert_eq!(
            failure.exchange.request_id.as_deref(),
            Some("request-1"),
            "{name}"
        );
        assert_eq!(
            failure.exchange.response_id.as_deref(),
            Some("response-1"),
            "{name}"
        );
        assert_eq!(
            failure
                .partial
                .present()
                .map(|partial| partial.text.as_str()),
            case.format.is_some().then_some(case.text),
            "{name}"
        );
        assert_eq!(*reported.0.lock().unwrap(), vec![failure], "{name}");
    }
}
/// contracts/failures.md, Identity and Partial output: a completed conversation turn that fails
/// Battersea's validation carries the origin, the identifiers the adapter announced and the
/// structured text of the turn, on the turn and the step interface alike. The announcement
/// leaves a turn that passes.
#[tokio::test]
async fn a_completed_turns_failure_carries_origin_identifiers_and_held_text() {
    let mut backend = backend();
    backend.capabilities.content.structured_output = vec![StructuredOutputMode::JsonSchema];
    if !backend
        .capabilities
        .supported_chat_parameters
        .iter()
        .any(|held| held == "responseFormat")
    {
        backend
            .capabilities
            .supported_chat_parameters
            .push("responseFormat".into());
    }
    let turn = |text: &str, class| {
        vec![
            Ok(EngineTextStreamEvent::ResponseOpened {
                request_id: Some("request-1".into()),
                response_id: Some("response-1".into()),
            }),
            Ok(EngineTextStreamEvent::MessageStart { turn_index: 0 }),
            Ok(EngineTextStreamEvent::TextDelta { text: text.into() }),
            Ok(EngineTextStreamEvent::MessageStop {
                class,
                native: json!({"stop_reason": "fixture"}),
            }),
        ]
    };
    let adapter = |events: Vec<Result<EngineTextStreamEvent, Failure>>, reported| {
        let mut registry = ProviderRegistry::default();
        registry
            .register_chat(
                "external",
                ChatProvider {
                    content: backend.capabilities.content.clone(),
                    background_modes: vec![],
                    parameters: backend.capabilities.supported_chat_parameters.clone(),
                    factory: Arc::new(move |_, _, _| {
                        Ok(Arc::new(Scripted {
                            events: events.clone(),
                            stall: false,
                        }))
                    }),
                },
            )
            .unwrap();
        registry
            .create_chat(backend.clone(), reported, clock())
            .unwrap()
    };
    let mut shared = request(&backend).shared;
    shared.chat.response_format = Some(ResponseFormat::JsonSchema {
        name: "result".into(),
        schema: json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]}),
    });
    let request = ConversationRequest {
        shared,
        tools: vec![],
        max_turns: 2,
    };
    for (name, text, class, kind) in [
        (
            "schema violation",
            "{\"ok\":3}",
            StopClass::Complete,
            ErrorKind::InvalidResponse,
        ),
        (
            "malformed JSON",
            "{\"ok\":",
            StopClass::Complete,
            ErrorKind::InvalidResponse,
        ),
        (
            "refusal",
            "{\"ok\":",
            StopClass::Refusal,
            ErrorKind::Refusal,
        ),
        (
            "truncation",
            "{\"ok\":",
            StopClass::Length,
            ErrorKind::Incomplete,
        ),
    ] {
        for operation in ["conversation_turn", "conversation_step"] {
            let name = format!("{name}, {operation}");
            let reported = Arc::new(Reported::default());
            let adapter = adapter(turn(text, class), Some(reported.clone() as _));
            let failure = if operation == "conversation_turn" {
                adapter
                    .conversation_turn(request.clone(), None, vec![])
                    .await
                    .err()
            } else {
                adapter
                    .conversation_step(request.clone(), None, vec![], ConversationCommand::Start)
                    .await
                    .err()
            }
            .unwrap_or_else(|| panic!("{name}: the turn passed"));
            assert_eq!(failure.kind, kind, "{name}");
            assert_eq!(
                failure.origin,
                Origin {
                    operation: Some(operation.into()),
                    provider: Some("external".into()),
                    backend: Some("contract".into()),
                    model: Some("configured".into()),
                },
                "{name}"
            );
            assert_eq!(
                failure.exchange.request_id.as_deref(),
                Some("request-1"),
                "{name}"
            );
            assert_eq!(
                failure.exchange.response_id.as_deref(),
                Some("response-1"),
                "{name}"
            );
            assert_eq!(
                failure
                    .partial
                    .present()
                    .map(|partial| partial.text.as_str()),
                Some(text),
                "{name}"
            );
            assert_eq!(*reported.0.lock().unwrap(), vec![failure], "{name}");
        }
    }
    let passed = adapter(turn("{\"ok\":true}", StopClass::Complete), None)
        .conversation_turn(request, None, vec![])
        .await
        .unwrap_or_else(|failure| panic!("{failure:?}"));
    assert!(!passed
        .events
        .iter()
        .any(|event| matches!(event, EngineTextStreamEvent::ResponseOpened { .. })));
}
#[tokio::test]
async fn request_deadline_covers_token_count_and_consumption_of_a_stalled_stream() {
    let clock = ManualClock::new(0);
    let adapter = registry(Default::default())
        .create_chat(backend(), None, Arc::new(clock.clone()))
        .unwrap();
    // The configured 20ms deadline is the only timer, and it elapses only when advanced.
    let window = Duration::from_millis(20);
    let (count, remaining) = tokio::join!(
        adapter.count_text_stream_input_tokens(request(&backend())),
        clock.advance_to_timer_within(window)
    );
    assert_eq!(remaining, window);
    assert!(matches!(count.unwrap_err().kind, ErrorKind::Timeout));
    let mut stream = adapter.stream_text(request(&backend())).await.unwrap();
    let (error, remaining) = tokio::join!(stream.next(), clock.advance_to_timer_within(window));
    assert_eq!(remaining, window);
    assert!(matches!(
        error.unwrap().unwrap_err().kind,
        ErrorKind::Timeout
    ));
    assert!(stream.next().await.is_none());
}
#[tokio::test]
async fn idle_watchdog_reports_one_typed_timeout_then_ends() {
    let clock = ManualClock::new(0);
    let mut stream = with_stream_idle_watchdog(
        Box::pin(stream::pending()),
        Duration::from_millis(10),
        "external",
        Arc::new(clock.clone()),
    );
    let (error, remaining) = tokio::join!(
        stream.next(),
        clock.advance_to_timer_within(Duration::from_millis(10))
    );
    assert_eq!(remaining, Duration::from_millis(10));
    assert!(matches!(
        error.unwrap().unwrap_err().kind,
        ErrorKind::Timeout
    ));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn each_idle_watchdog_event_restarts_the_idle_deadline() {
    let clock = ManualClock::new(0);
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let events = tokio_stream::wrappers::UnboundedReceiverStream::new(receiver).map(Ok);
    let mut stream =
        with_stream_idle_watchdog(Box::pin(events), Duration::from_millis(10), "external", {
            Arc::new(clock.clone())
        });
    let delta = || EngineTextStreamEvent::TextDelta { text: "x".into() };
    // Nine milliseconds into each idle period an event arrives and restarts the period.
    for _ in 0..3 {
        let next = stream.next();
        tokio::pin!(next);
        let send = async {
            assert_eq!(
                clock.timer_within(Duration::from_millis(10)).await,
                Duration::from_millis(10)
            );
            clock.advance(Duration::from_millis(9));
            sender.send(delta()).unwrap();
        };
        let (event, ()) = tokio::join!(next, send);
        assert_eq!(event.unwrap().unwrap(), delta());
    }
    let (error, remaining) = tokio::join!(
        stream.next(),
        clock.advance_to_timer_within(Duration::from_millis(10))
    );
    assert_eq!(remaining, Duration::from_millis(10));
    assert!(error.unwrap().is_err());
}

struct ToolEvents(Vec<EngineTextStreamEvent>);
#[async_trait]
impl EngineAdapter for ToolEvents {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        Transport(Default::default()).describe_debug_context()
    }
    async fn count_text_stream_input_tokens(
        &self,
        _: EngineTextStreamRequest,
    ) -> Result<u64, Failure> {
        Ok(0)
    }
    async fn stream_text(&self, _: EngineTextStreamRequest) -> Result<EngineTextStream, Failure> {
        Ok(Box::pin(
            stream::iter(self.0.clone().into_iter().map(Ok)).chain(stream::pending()),
        ))
    }
}

#[tokio::test]
async fn stream_watchdogs_retain_started_and_completed_tool_evidence() {
    for completion in [
        None,
        Some(ToolCallOutcome::Completed),
        Some(ToolCallOutcome::NotDispatched),
    ] {
        let mut events = vec![EngineTextStreamEvent::ToolCallStarted {
            id: "effect-1".into(),
            name: "remote.write".into(),
            arguments: json!({}),
            started_at: String::new(),
        }];
        if let Some(outcome) = completion {
            events.push(EngineTextStreamEvent::ToolCallCompleted {
                id: "effect-1".into(),
                name: "remote.write".into(),
                ok: outcome == ToolCallOutcome::Completed,
                result: json!(null),
                error_message: None,
                error_classification: None,
                tool_result: None,
                tool_call: Some(ToolCallEvidence {
                    call_id: "effect-1".into(),
                    tool_name: "remote.write".into(),
                    outcome,
                    request_id: Some("remote-request".into()),
                }),
                finished_at: String::new(),
            });
        }
        for idle in [true, false] {
            let clock = ManualClock::new(0);
            let mut stream = if idle {
                with_stream_idle_watchdog(
                    ToolEvents(events.clone())
                        .stream_text(request(&backend()))
                        .await
                        .unwrap(),
                    Duration::from_millis(10),
                    "external",
                    Arc::new(clock.clone()),
                )
            } else {
                let mut registry = ProviderRegistry::default();
                let events = events.clone();
                registry
                    .register_chat(
                        "external",
                        ChatProvider {
                            content: ContentCapabilities::text(),
                            background_modes: vec![],
                            parameters: backend().capabilities.supported_chat_parameters,
                            factory: Arc::new(move |_, _, _| {
                                Ok(Arc::new(ToolEvents(events.clone())))
                            }),
                        },
                    )
                    .unwrap();
                let mut backend = backend();
                backend.options.timeout_ms = Some(100);
                registry
                    .create_chat(backend.clone(), None, Arc::new(clock.clone()))
                    .unwrap()
                    .stream_text(request(&backend))
                    .await
                    .unwrap()
            };
            for expected in &events {
                assert_eq!(&stream.next().await.unwrap().unwrap(), expected);
            }
            // The idle watchdog allows 10ms; the request deadline allows 100ms.
            let (error, remaining) = tokio::join!(
                stream.next(),
                clock.advance_to_timer_within(Duration::from_millis(100))
            );
            assert_eq!(
                remaining,
                Duration::from_millis(if idle { 10 } else { 100 })
            );
            let error = error.unwrap().unwrap_err();
            assert_eq!(error.kind, ErrorKind::Timeout);
            let evidence = error.tool_call.clone().unwrap();
            assert_eq!(evidence.call_id, "effect-1");
            assert_eq!(
                evidence.outcome,
                completion.unwrap_or(ToolCallOutcome::Uncertain)
            );
            assert_eq!(
                evidence.request_id,
                completion.map(|_| "remote-request".into())
            );
            assert!(stream.next().await.is_none());
        }
    }
}
struct PendingJob(Arc<AtomicUsize>, ManualClock);
#[async_trait]
impl MediaJob for PendingJob {
    fn id(&self) -> &str {
        "accepted-provider-id"
    }
    fn clock(&self) -> Arc<dyn Clock> {
        Arc::new(self.1.clone())
    }
    async fn wait(&mut self, _: CancellationToken) -> Result<MediaRenderResult, Failure> {
        std::future::pending().await
    }
    fn snapshot(&self) -> MediaJobSnapshot {
        MediaJobSnapshot {
            id: self.id().into(),
            status: MediaJobStatus::Queued,
            output_expires_at: None,
            expiry_is_estimate: false,
            outputs_retrieved: false,
        }
    }
    fn poll_policy(&self) -> MediaPollPolicy {
        MediaPollPolicy {
            interval_ms: 1,
            retry_delay_ms: 1,
            max_retry_delay_ms: 1,
            max_retries: 0,
        }
    }
    async fn poll(&mut self, _: CancellationToken) -> Result<MediaJobStatus, Failure> {
        std::future::pending().await
    }
    async fn retrieve(&mut self, _: CancellationToken) -> Result<MediaRenderResult, Failure> {
        unreachable!()
    }
    async fn cancel(&mut self) -> Result<(), Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[tokio::test]
async fn pending_media_jobs_cancel_once_on_cancellation_or_deadline() {
    for cancelled in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let clock = ManualClock::new(0);
        let submission =
            MediaSubmission::Pending(Box::new(PendingJob(calls.clone(), clock.clone())));
        let cancellation = CancellationToken::new();
        if cancelled {
            cancellation.cancel();
        }
        let deadline = clock.instant() + Duration::from_millis(10);
        let wait = submission.wait(cancellation, Some(deadline));
        let result = if cancelled {
            wait.await
        } else {
            let (result, remaining) = tokio::join!(
                wait,
                clock.advance_to_timer_within(Duration::from_millis(10))
            );
            assert_eq!(remaining, Duration::from_millis(10));
            result
        }
        .unwrap_err();
        assert_eq!(
            result.kind,
            if cancelled {
                ErrorKind::Cancelled
            } else {
                ErrorKind::Timeout
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn synchronous_media_completion_preserves_result_without_fabricating_a_job() {
    let result = MediaRenderResult::without_envelopes(None, vec![]);
    assert_eq!(
        MediaSubmission::Complete(result.clone())
            .wait(CancellationToken::new(), None)
            .await
            .unwrap(),
        result
    );
}
