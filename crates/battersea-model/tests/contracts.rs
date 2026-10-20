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
    ) -> Result<u64, EngineAdapterRequestError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::pending().await
    }
    async fn stream_text(
        &self,
        _: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(stream::pending()))
    }
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
                factory: Arc::new(move |_, _| Ok(Arc::new(Transport(calls.clone())))),
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
    let adapter = registry.create_chat(backend.clone(), None).unwrap();
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
            .classification,
        ErrorKind::InvalidRequest
    ));
    assert!(matches!(
        adapter
            .count_text_stream_input_tokens(invalid)
            .await
            .unwrap_err()
            .classification,
        ErrorKind::InvalidRequest
    ));
    let mut invalid = request(&backend);
    invalid.shared.chat.stream = false;
    assert!(matches!(
        adapter
            .stream_text(invalid)
            .await
            .err()
            .unwrap()
            .classification,
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
                .create_chat(invalid, None)
                .err()
                .unwrap()
                .classification,
            ErrorKind::InvalidRequest
        ));
    }
}
#[tokio::test]
async fn request_deadline_covers_token_count_and_consumption_of_a_stalled_stream() {
    let adapter = registry(Default::default())
        .create_chat(backend(), None)
        .unwrap();
    let count = tokio::time::timeout(
        Duration::from_secs(3),
        adapter.count_text_stream_input_tokens(request(&backend())),
    )
    .await
    .expect("bounded token counting");
    assert!(matches!(
        count.unwrap_err().classification,
        ErrorKind::Timeout
    ));
    let mut stream = adapter.stream_text(request(&backend())).await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .expect("bounded stream")
        .unwrap()
        .unwrap_err();
    assert!(matches!(error.classification, ErrorKind::Timeout));
    assert!(tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .expect("terminal stream")
        .is_none());
}
#[tokio::test]
async fn idle_watchdog_reports_one_typed_timeout_then_ends() {
    let mut stream = with_stream_idle_watchdog(
        Box::pin(stream::pending()),
        Duration::from_millis(10),
        "external",
    );
    let error = tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(matches!(error.classification, ErrorKind::Timeout));
    assert!(tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .expect("terminal stream")
        .is_none());
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
    ) -> Result<u64, EngineAdapterRequestError> {
        Ok(0)
    }
    async fn stream_text(
        &self,
        _: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
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
            let mut stream = if idle {
                with_stream_idle_watchdog(
                    ToolEvents(events.clone())
                        .stream_text(request(&backend()))
                        .await
                        .unwrap(),
                    Duration::from_millis(10),
                    "external",
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
                            factory: Arc::new(move |_, _| Ok(Arc::new(ToolEvents(events.clone())))),
                        },
                    )
                    .unwrap();
                let mut backend = backend();
                backend.options.timeout_ms = Some(100);
                registry
                    .create_chat(backend.clone(), None)
                    .unwrap()
                    .stream_text(request(&backend))
                    .await
                    .unwrap()
            };
            for expected in &events {
                assert_eq!(&stream.next().await.unwrap().unwrap(), expected);
            }
            let error = tokio::time::timeout(Duration::from_secs(3), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert_eq!(error.classification, ErrorKind::Timeout);
            let evidence = error.tool_call.unwrap();
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
struct PendingJob(Arc<AtomicUsize>);
#[async_trait]
impl MediaJob for PendingJob {
    fn id(&self) -> &str {
        "accepted-provider-id"
    }
    async fn wait(
        &mut self,
        _: CancellationToken,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
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
    async fn poll(
        &mut self,
        _: CancellationToken,
    ) -> Result<MediaJobStatus, EngineAdapterRequestError> {
        std::future::pending().await
    }
    async fn retrieve(
        &mut self,
        _: CancellationToken,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        unreachable!()
    }
    async fn cancel(&mut self) -> Result<(), EngineAdapterRequestError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[tokio::test]
async fn pending_media_jobs_cancel_once_on_cancellation_or_deadline() {
    for cancelled in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let submission = MediaSubmission::Pending(Box::new(PendingJob(calls.clone())));
        let cancellation = CancellationToken::new();
        if cancelled {
            cancellation.cancel();
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(10);
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            submission.wait(cancellation, Some(deadline)),
        )
        .await
        .expect("bounded job")
        .unwrap_err();
        assert_eq!(
            result.classification,
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
