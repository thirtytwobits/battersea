//! Copyright (c) Scott A Dixon
use async_trait::async_trait;
use battersea_model::*;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
struct Store {
    bytes: Mutex<Option<Vec<u8>>>,
    fault: Mutex<Option<(u64, bool)>>,
}
#[async_trait]
impl ConversationStore for Store {
    async fn load(
        &self,
        _: &str,
        max: usize,
    ) -> Result<Option<ConversationCheckpoint>, ConversationStoreError> {
        self.bytes
            .lock()
            .unwrap()
            .as_ref()
            .map(|b| {
                if b.len() > max {
                    return Err(ConversationStoreError::Unavailable);
                }
                serde_json::from_slice(b).map_err(|_| ConversationStoreError::Unavailable)
            })
            .transpose()
    }
    async fn compare_exchange(
        &self,
        _: &str,
        expected: Option<u64>,
        next: &ConversationCheckpoint,
    ) -> Result<(), ConversationStoreError> {
        tokio::task::yield_now().await;
        let mut bytes = self.bytes.lock().unwrap();
        let revision = bytes.as_ref().map(|b| {
            serde_json::from_slice::<ConversationCheckpoint>(b)
                .unwrap()
                .revision
        });
        if revision != expected {
            return Err(ConversationStoreError::Conflict);
        }
        let fault = *self.fault.lock().unwrap();
        if fault == Some((next.revision, false)) {
            return Err(ConversationStoreError::Unavailable);
        }
        *bytes = Some(serde_json::to_vec(next).unwrap());
        if fault == Some((next.revision, true)) {
            return Err(ConversationStoreError::Unavailable);
        }
        Ok(())
    }
}
struct Provider {
    calls: AtomicUsize,
    pause: AtomicBool,
    bad_batch: bool,
    large: bool,
    delivered: Mutex<Vec<Vec<ConversationToolResult>>>,
    retained: Mutex<Option<ConversationTurn>>,
}
impl Default for Provider {
    fn default() -> Self {
        Self {
            calls: 0.into(),
            pause: false.into(),
            bad_batch: false,
            large: false,
            delivered: Mutex::new(vec![]),
            retained: Mutex::new(None),
        }
    }
}
#[async_trait]
impl EngineAdapter for Provider {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        EngineAdapterDebugContext {
            provider: "fixture".into(),
            backend: "fixture".into(),
            model: "fixture".into(),
            sdk: "fixture".into(),
            base_url: None,
            timeout_ms: Some(5000),
            max_retries: Some(0),
        }
    }
    fn validate_conversation(
        &self,
        _: &ConversationRequest,
        state: Option<&Value>,
    ) -> Result<ConversationBinding, EngineAdapterRequestError> {
        if state.is_some_and(|s| s["version"] != 1) {
            return Err(EngineAdapterRequestError::invalid_response(
                "fixture",
                "Invalid version",
            ));
        }
        Ok(ConversationBinding {
            origin: ProviderContentOrigin {
                provider: "fixture".into(),
                adapter: "fixture".into(),
                binding_revision: "binding-1".into(),
            },
            configuration_revision: "configuration-1".into(),
            timeout_ms: 5000,
        })
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
        unreachable!()
    }
    async fn conversation_turn(
        &self,
        _: ConversationRequest,
        state: Option<Value>,
        results: Vec<ConversationToolResult>,
    ) -> Result<ConversationTurn, EngineAdapterRequestError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.delivered.lock().unwrap().push(results);
        let first = state.is_none();
        let calls = if first {
            vec![
                call("a"),
                if self.bad_batch { call("a") } else { call("b") },
            ]
        } else {
            vec![]
        };
        let turn = ConversationTurn {
            state: json!({"version":1,"complete":!first}),
            calls,
            events: vec![
                EngineTextStreamEvent::MessageStart { turn_index: 0 },
                EngineTextStreamEvent::TextDelta {
                    text: if self.large {
                        "x".repeat(70000)
                    } else {
                        "answer".into()
                    },
                },
                EngineTextStreamEvent::MessageStop {
                    reason: if first {
                        StopReason::ToolUse
                    } else {
                        StopReason::Complete
                    },
                },
            ],
        };
        *self.retained.lock().unwrap() = Some(turn.clone());
        if self.pause.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        Ok(turn)
    }
}
fn call(id: &str) -> EngineLocalToolCall {
    EngineLocalToolCall {
        id: id.into(),
        name: "host_write".into(),
        arguments: json!({"value":1}),
    }
}
fn result(id: &str) -> ConversationToolResult {
    ConversationToolResult {
        call: call(id),
        result: EngineLocalToolResult::json(json!({"receipt":id})),
    }
}
fn scope() -> ConversationScope {
    ConversationScope {
        run_id: "run-1".into(),
        tool_binding_revision: "tools-1".into(),
        expires_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 4000,
    }
}
fn request() -> ConversationRequest {
    ConversationRequest {
        shared: EngineAdapterRequest {
            operation: EngineOperation::FlowTextStream,
            messages: vec![Message::text(Role::User, "Write".into())],
            chat: engine::EngineChatParameters::default_for_provider("mock"),
            debug_rules: None,
        },
        tools: vec![EngineLocalToolDefinition {
            name: "host.write".into(),
            description: "Write".into(),
            input_schema: json!({"type":"object","required":["value"]}),
        }],
        max_turns: 3,
    }
}
async fn create(
    store: Arc<Store>,
    provider: Arc<Provider>,
    scope: ConversationScope,
) -> DurableConversation {
    DurableConversation::create(
        store,
        provider,
        scope,
        request(),
        65536,
        Duration::from_secs(1),
    )
    .await
    .unwrap()
}
async fn open(
    store: Arc<Store>,
    provider: Arc<Provider>,
    scope: ConversationScope,
) -> DurableConversation {
    DurableConversation::open(
        store,
        provider,
        scope,
        request(),
        65536,
        Duration::from_secs(1),
    )
    .await
    .unwrap()
}
fn code(e: EngineAdapterRequestError, code: ToolCode) {
    assert_eq!(e.tool_code, Some(code));
}

#[tokio::test]
async fn restart_retains_complete_batch_and_partial_results_without_regeneration() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Provider::default());
    let scope = scope();
    let d = create(store.clone(), provider.clone(), scope.clone()).await;
    let pending = d.resume(1).await.unwrap();
    assert_eq!(pending.phase, ConversationPhase::AwaitingTools);
    assert_eq!(pending.turns[0].response.calls.len(), 2);
    code(
        d.resume(3).await.unwrap_err(),
        ToolCode::ToolApprovalPending,
    );
    drop(d);
    let d = open(store.clone(), provider.clone(), scope.clone()).await;
    d.record_tool_result(3, result("a")).await.unwrap();
    drop(d);
    let d = open(store.clone(), provider.clone(), scope.clone()).await;
    assert_eq!(
        d.record_tool_result(4, result("a")).await.unwrap().revision,
        4
    );
    code(
        d.resume(4).await.unwrap_err(),
        ToolCode::ToolApprovalPending,
    );
    d.record_tool_result(4, result("b")).await.unwrap();
    let done = d.resume(5).await.unwrap();
    assert_eq!(done.phase, ConversationPhase::Completed);
    drop(d);
    let d = open(store, provider.clone(), scope).await;
    assert_eq!(d.resume(7).await.unwrap(), done);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let delivered = provider.delivered.lock().unwrap();
    assert!(delivered[0].is_empty());
    assert_eq!(delivered[1], vec![result("a"), result("b")]);
}

#[tokio::test]
async fn failed_intent_and_turn_publication_require_inspection_before_recovery() {
    for revision in [2, 3, 6, 7] {
        for committed in [false, true] {
            let store = Arc::new(Store::default());
            let provider = Arc::new(Provider::default());
            let scope = scope();
            let d = create(store.clone(), provider.clone(), scope.clone()).await;
            let expected = if revision >= 6 {
                d.resume(1).await.unwrap();
                d.record_tool_result(3, result("a")).await.unwrap();
                d.record_tool_result(4, result("b")).await.unwrap();
                5
            } else {
                1
            };
            *store.fault.lock().unwrap() = Some((revision, committed));
            code(
                d.resume(expected).await.unwrap_err(),
                ToolCode::ToolContinuationStorage,
            );
            drop(d);
            *store.fault.lock().unwrap() = None;
            let d = open(store, provider.clone(), scope).await;
            let mut r = d.inspect().await.unwrap();
            if r.phase == ConversationPhase::ProviderIntent {
                code(
                    d.resume(r.revision).await.unwrap_err(),
                    ToolCode::ToolContinuationUnresolved,
                );
                let resolution = if revision == 2 || revision == 6 {
                    ConversationResolution::NotDispatched
                } else {
                    ConversationResolution::Completed {
                        turn: provider.retained.lock().unwrap().clone().unwrap(),
                    }
                };
                r = d
                    .reconcile(
                        r.revision,
                        resolution,
                        "operator",
                        "Original task stopped; retained provider exchange inspected",
                    )
                    .await
                    .unwrap();
            }
            if r.phase == ConversationPhase::Ready || (revision == 6 && !committed) {
                r = d.resume(r.revision).await.unwrap();
            }
            assert_eq!(
                r.phase,
                if revision >= 6 {
                    ConversationPhase::Completed
                } else {
                    ConversationPhase::AwaitingTools
                }
            );
            assert_eq!(
                provider.calls.load(Ordering::SeqCst),
                if revision >= 6 { 2 } else { 1 }
            );
        }
    }
}

#[tokio::test]
async fn concurrent_commands_and_changed_results_cannot_advance_twice() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Provider::default());
    let scope = scope();
    let a = create(store.clone(), provider.clone(), scope.clone()).await;
    let b = open(store, provider.clone(), scope).await;
    let (x, y) = tokio::join!(a.resume(1), b.resume(1));
    assert_ne!(x.is_ok(), y.is_ok());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let mut altered = result("a");
    altered.call.arguments = json!({"value":2});
    assert!(a.record_tool_result(3, altered).await.is_err());
    let (x, y) = tokio::join!(
        a.record_tool_result(3, result("a")),
        b.record_tool_result(3, result("b"))
    );
    assert_ne!(x.is_ok(), y.is_ok());
    code(
        a.resume(3).await.unwrap_err(),
        ToolCode::ToolContinuationConflict,
    );
}

#[tokio::test]
async fn cancelled_provider_task_leaves_an_unresolved_intent() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Provider::default());
    let scope = scope();
    provider.pause.store(true, Ordering::SeqCst);
    let d = create(store.clone(), provider.clone(), scope.clone()).await;
    let task = tokio::spawn(async move { d.resume(1).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while provider.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let d = open(store, provider.clone(), scope).await;
    code(
        d.resume(2).await.unwrap_err(),
        ToolCode::ToolContinuationUnresolved,
    );
    let abandoned = d
        .reconcile(
            2,
            ConversationResolution::Abandon,
            "operator",
            "Executor fenced; provider outcome unknown",
        )
        .await
        .unwrap();
    assert_eq!(abandoned.phase, ConversationPhase::Abandoned);
    assert!(d.resume(3).await.is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn bad_batches_and_oversized_turns_never_expose_executable_calls() {
    for large in [false, true] {
        let provider = Arc::new(Provider {
            large,
            bad_batch: !large,
            ..Provider::default()
        });
        let d = create(Arc::new(Store::default()), provider.clone(), scope()).await;
        assert!(d.resume(1).await.is_err());
        let r = d.inspect().await.unwrap();
        assert!(r.turns.is_empty());
        assert_eq!(r.phase, ConversationPhase::ProviderIntent);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn corrupt_or_changed_restore_never_writes_or_dispatches() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Provider::default());
    let scope = scope();
    let d = create(store.clone(), provider.clone(), scope.clone()).await;
    d.resume(1).await.unwrap();
    let valid = store.bytes.lock().unwrap().clone().unwrap();
    for mutation in 0..5 {
        let mut value: Value = serde_json::from_slice(&valid).unwrap();
        match mutation {
            0 => value["version"] = json!(99),
            1 => value["request"]["shared"]["messages"] = json!([]),
            2 => value["phase"] = json!("ready"),
            3 => value["binding"]["configuration_revision"] = json!("changed"),
            _ => value["scope"]["expires_at_unix_ms"] = json!(u64::MAX),
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        *store.bytes.lock().unwrap() = Some(bytes.clone());
        assert!(DurableConversation::open(
            store.clone(),
            provider.clone(),
            scope.clone(),
            request(),
            65536,
            Duration::from_secs(1)
        )
        .await
        .is_err());
        assert_eq!(*store.bytes.lock().unwrap(), Some(bytes));
    }
    *store.bytes.lock().unwrap() = Some(valid);
    let mut changed = scope.clone();
    changed.tool_binding_revision = "other".into();
    assert!(DurableConversation::open(
        store.clone(),
        provider.clone(),
        changed,
        request(),
        65536,
        Duration::from_secs(1)
    )
    .await
    .is_err());
    let mut changed = request();
    changed.max_turns += 1;
    assert!(DurableConversation::open(
        store,
        provider.clone(),
        scope,
        changed,
        65536,
        Duration::from_secs(1)
    )
    .await
    .is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn expiry_and_attempt_budgets_survive_reopen_and_reconciliation() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Provider::default());
    let mut scope = scope();
    scope.expires_at_unix_ms -= 3800;
    let d = create(store.clone(), provider.clone(), scope.clone()).await;
    d.resume(1).await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let d = open(store, provider.clone(), scope).await;
    d.record_tool_result(3, result("a")).await.unwrap();
    d.record_tool_result(4, result("b")).await.unwrap();
    code(
        d.resume(5).await.unwrap_err(),
        ToolCode::ToolContinuationExpired,
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let store = Arc::new(Store::default());
    let provider = Arc::new(Provider::default());
    let d = create(store.clone(), provider.clone(), crate::scope()).await;
    let mut revision = 1;
    for _ in 0..3 {
        *store.fault.lock().unwrap() = Some((revision + 1, true));
        assert!(d.resume(revision).await.is_err());
        *store.fault.lock().unwrap() = None;
        revision = d
            .reconcile(
                revision + 1,
                ConversationResolution::NotDispatched,
                "operator",
                "Task fenced before request dispatch",
            )
            .await
            .unwrap()
            .revision;
    }
    code(
        d.resume(revision).await.unwrap_err(),
        ToolCode::ToolRoundsExceeded,
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn acceptance_and_result_copy_failures_resume_from_the_committed_revision() {
    for committed in [false, true] {
        let store = Arc::new(Store::default());
        let provider = Arc::new(Provider::default());
        let scope = scope();
        *store.fault.lock().unwrap() = Some((1, committed));
        assert!(DurableConversation::create(
            store.clone(),
            provider.clone(),
            scope.clone(),
            request(),
            65536,
            Duration::from_secs(1)
        )
        .await
        .is_err());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        *store.fault.lock().unwrap() = None;
        let d = if committed {
            open(store.clone(), provider.clone(), scope.clone()).await
        } else {
            create(store.clone(), provider.clone(), scope.clone()).await
        };
        d.resume(1).await.unwrap();
        *store.fault.lock().unwrap() = Some((4, committed));
        assert!(d.record_tool_result(3, result("a")).await.is_err());
        drop(d);
        *store.fault.lock().unwrap() = None;
        let d = open(store, provider.clone(), scope).await;
        let revision = d.inspect().await.unwrap().revision;
        assert_eq!(
            d.record_tool_result(revision, result("a"))
                .await
                .unwrap()
                .revision,
            4
        );
        let mut changed = result("a");
        changed.result = EngineLocalToolResult::json(json!({"changed":true}));
        assert!(d.record_tool_result(4, changed).await.is_err());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn a_deadline_during_generation_never_turns_an_intent_into_a_retry() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Provider::default());
    let mut scope = scope();
    scope.expires_at_unix_ms -= 3800;
    provider.pause.store(true, Ordering::SeqCst);
    let d = create(store.clone(), provider.clone(), scope.clone()).await;
    assert_eq!(
        d.resume(1).await.unwrap_err().classification,
        ErrorKind::Timeout
    );
    drop(d);
    let d = open(store, provider.clone(), scope).await;
    code(
        d.resume(2).await.unwrap_err(),
        ToolCode::ToolContinuationUnresolved,
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn recovered_turns_obey_final_output_admission() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Provider::default());
    let mut request = request();
    request.shared.chat.response_format = Some(ResponseFormat::JsonObject);
    let d = DurableConversation::create(
        store.clone(),
        provider.clone(),
        scope(),
        request,
        65536,
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    d.resume(1).await.unwrap();
    d.record_tool_result(3, result("a")).await.unwrap();
    d.record_tool_result(4, result("b")).await.unwrap();
    assert!(d.resume(5).await.is_err());
    assert_eq!(
        d.inspect().await.unwrap().phase,
        ConversationPhase::ProviderIntent
    );
    let invalid = provider.retained.lock().unwrap().clone().unwrap();
    assert!(d
        .reconcile(
            6,
            ConversationResolution::Completed { turn: invalid },
            "operator",
            "Retained reply from the fenced provider task"
        )
        .await
        .is_err());
    assert_eq!(d.inspect().await.unwrap().revision, 6);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}
