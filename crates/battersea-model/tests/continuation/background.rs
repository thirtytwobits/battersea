//! Copyright (c) Scott A Dixon
use super::*;

#[derive(Default)]
struct Remote {
    provider: Provider,
    creates: AtomicUsize,
    polls: AtomicUsize,
    cancels: AtomicUsize,
    pending: AtomicBool,
    pause_poll: AtomicBool,
    pause_cancel: AtomicBool,
    retained: Mutex<Option<Value>>,
}

#[async_trait]
impl EngineAdapter for Remote {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        self.provider.describe_debug_context()
    }
    fn validate_conversation(
        &self,
        request: &ConversationRequest,
        state: Option<&Value>,
    ) -> Result<ConversationBinding, EngineAdapterRequestError> {
        self.provider.validate_conversation(request, state)
    }
    fn validate_conversation_turn(
        &self,
        request: &ConversationRequest,
        previous: Option<&Value>,
        results: &[ConversationToolResult],
        turn: &ConversationTurn,
    ) -> Result<ConversationBinding, EngineAdapterRequestError> {
        self.provider
            .validate_conversation_turn(request, previous, results, turn)
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
    fn validate_conversation_step(
        &self,
        request: &ConversationRequest,
        previous: Option<&Value>,
        results: &[ConversationToolResult],
        command: &ConversationCommand,
        step: &ConversationStep,
    ) -> Result<ConversationBinding, EngineAdapterRequestError> {
        let invalid =
            || EngineAdapterRequestError::invalid_response("fixture", "Invalid remote evidence");
        let validate = |state: &Value| {
            if state["version"] != 1
                || !state["id"].is_u64()
                || state["previous"] != previous.cloned().unwrap_or(Value::Null)
                || state["results"] != serde_json::to_value(results).unwrap()
            {
                return Err(invalid());
            }
            Ok(())
        };
        match (command, step) {
            (ConversationCommand::Start, ConversationStep::Pending { state }) => validate(state)?,
            (ConversationCommand::Poll { state }, ConversationStep::Pending { state: next }) => {
                validate(state)?;
                if state != next {
                    return Err(invalid());
                }
            }
            (ConversationCommand::Poll { state }, ConversationStep::Completed { turn }) => {
                validate(state)?;
                if turn.state["remote_id"] != state["id"] {
                    return Err(invalid());
                }
                self.validate_conversation_turn(request, previous, results, turn)?;
            }
            (
                ConversationCommand::Cancel { state },
                ConversationStep::Cancelled { state: receipt },
            ) => {
                validate(state)?;
                if receipt != &json!({"id":state["id"],"terminal":true}) {
                    return Err(invalid());
                }
            }
            _ => return Err(invalid()),
        }
        self.validate_conversation(request, previous)
    }
    async fn conversation_step(
        &self,
        request: ConversationRequest,
        previous: Option<Value>,
        results: Vec<ConversationToolResult>,
        command: ConversationCommand,
    ) -> Result<ConversationStep, EngineAdapterRequestError> {
        Ok(match command {
            ConversationCommand::Start => {
                let id = self.creates.fetch_add(1, Ordering::SeqCst) + 1;
                let state = json!({"version":1,"id":id,"previous":previous,"results":results});
                *self.retained.lock().unwrap() = Some(state.clone());
                ConversationStep::Pending { state }
            }
            ConversationCommand::Poll { state } => {
                self.polls.fetch_add(1, Ordering::SeqCst);
                if self.pause_poll.load(Ordering::SeqCst) {
                    std::future::pending::<()>().await;
                }
                if self.pending.load(Ordering::SeqCst) {
                    ConversationStep::Pending { state }
                } else {
                    let mut turn = self
                        .provider
                        .conversation_turn(request, previous, results)
                        .await?;
                    turn.state["remote_id"] = state["id"].clone();
                    ConversationStep::Completed { turn }
                }
            }
            ConversationCommand::Cancel { state } => {
                self.cancels.fetch_add(1, Ordering::SeqCst);
                if self.pause_cancel.load(Ordering::SeqCst) {
                    std::future::pending::<()>().await;
                }
                ConversationStep::Cancelled {
                    state: json!({"id":state["id"],"terminal":true}),
                }
            }
        })
    }
}

async fn remote(
    store: Arc<Store>,
    provider: Arc<Remote>,
    scope: ConversationScope,
    reopen: bool,
) -> DurableConversation {
    let mut request = request();
    request.max_turns = 2;
    if reopen {
        DurableConversation::open(
            store,
            provider,
            scope,
            request,
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap()
    } else {
        DurableConversation::create(
            store,
            provider,
            scope,
            request,
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn polling_reopens_without_spending_turns_or_repeating_generation() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Remote::default());
    let scope = scope();
    let d = remote(store.clone(), provider.clone(), scope.clone(), false).await;
    let accepted = d.resume(1).await.unwrap();
    assert_eq!(accepted.phase, ConversationPhase::ProviderPending);
    assert_eq!(accepted.attempts, 1);
    assert!(accepted.turns.is_empty());
    let d = remote(store.clone(), provider.clone(), scope.clone(), true).await;
    provider.pending.store(true, Ordering::SeqCst);
    let waiting = d.resume(3).await.unwrap();
    assert_eq!(waiting.pending, accepted.pending);
    assert_eq!(waiting.attempts, 1);
    provider.pending.store(false, Ordering::SeqCst);
    let called = d.resume(5).await.unwrap();
    assert_eq!(called.phase, ConversationPhase::AwaitingTools);
    assert_eq!(called.turns[0].pending, accepted.pending);
    d.record_tool_result(7, result("a")).await.unwrap();
    d.record_tool_result(8, result("b")).await.unwrap();
    let next = d.resume(9).await.unwrap();
    assert_eq!(next.attempts, 2);
    let d = remote(store.clone(), provider.clone(), scope.clone(), true).await;
    let completed = d.resume(11).await.unwrap();
    assert_eq!(completed.phase, ConversationPhase::Completed);
    assert_eq!(completed.attempts, 2);
    let d = remote(store, provider.clone(), scope, true).await;
    assert_eq!(d.resume(13).await.unwrap(), completed);
    assert_eq!(provider.creates.load(Ordering::SeqCst), 2);
    assert_eq!(provider.polls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn pending_publication_failure_requires_reconciliation_without_resubmission() {
    for committed in [false, true] {
        let store = Arc::new(Store::default());
        let provider = Arc::new(Remote::default());
        let scope = scope();
        let d = remote(store.clone(), provider.clone(), scope.clone(), false).await;
        *store.fault.lock().unwrap() = Some((3, committed));
        assert!(d.resume(1).await.is_err());
        *store.fault.lock().unwrap() = None;
        let d = remote(store, provider.clone(), scope, true).await;
        let r = d.inspect().await.unwrap();
        if !committed {
            code(
                d.resume(2).await.unwrap_err(),
                ToolCode::ToolContinuationUnresolved,
            );
            let state = provider.retained.lock().unwrap().clone().unwrap();
            d.reconcile(
                2,
                ConversationResolution::Pending { state },
                "operator",
                "Original task fenced; native create response retained",
            )
            .await
            .unwrap();
        }
        assert_eq!(
            d.resume(3).await.unwrap().phase,
            ConversationPhase::AwaitingTools
        );
        assert_eq!(r.attempts, 1);
        assert_eq!(provider.creates.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn interrupted_poll_and_cancel_resume_the_same_remote_operation() {
    for cancel in [false, true] {
        let store = Arc::new(Store::default());
        let provider = Arc::new(Remote::default());
        let scope = scope();
        let d = remote(store.clone(), provider.clone(), scope.clone(), false).await;
        d.resume(1).await.unwrap();
        provider.pause_poll.store(!cancel, Ordering::SeqCst);
        provider.pause_cancel.store(cancel, Ordering::SeqCst);
        let task = tokio::spawn(async move {
            if cancel {
                d.cancel(3).await
            } else {
                d.resume(3).await
            }
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while provider.polls.load(Ordering::SeqCst) + provider.cancels.load(Ordering::SeqCst)
                == 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        provider.pause_poll.store(false, Ordering::SeqCst);
        provider.pause_cancel.store(false, Ordering::SeqCst);
        let d = remote(store.clone(), provider.clone(), scope.clone(), true).await;
        let completed = if cancel {
            code(
                d.resume(4).await.unwrap_err(),
                ToolCode::ToolContinuationUnresolved,
            );
            d.cancel(4).await.unwrap()
        } else {
            d.resume(4).await.unwrap()
        };
        assert_eq!(completed.attempts, 1);
        assert_eq!(provider.creates.load(Ordering::SeqCst), 1);
        if cancel {
            let d = remote(store, provider.clone(), scope, true).await;
            assert_eq!(d.cancel(completed.revision).await.unwrap(), completed);
            assert_eq!(completed.phase, ConversationPhase::Abandoned);
            assert_eq!(provider.cancels.load(Ordering::SeqCst), 2);
        } else {
            assert_eq!(completed.phase, ConversationPhase::AwaitingTools);
        }
    }
}

#[tokio::test]
async fn expired_remote_work_can_be_cancelled_but_cannot_be_polled() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Remote::default());
    let mut scope = scope();
    scope.expires_at_unix_ms -= 3800;
    let d = remote(store.clone(), provider.clone(), scope.clone(), false).await;
    d.resume(1).await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let d = remote(store, provider.clone(), scope, true).await;
    code(
        d.resume(3).await.unwrap_err(),
        ToolCode::ToolContinuationExpired,
    );
    assert_eq!(
        d.cancel(3).await.unwrap().phase,
        ConversationPhase::Abandoned
    );
    assert_eq!(provider.polls.load(Ordering::SeqCst), 0);
    assert_eq!(provider.cancels.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn version_one_records_remain_unchanged_on_inspection_and_keep_their_version_on_resume() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Provider::default());
    let scope = scope();
    let d = create(store.clone(), provider.clone(), scope.clone()).await;
    let mut value = serde_json::to_value(d.inspect().await.unwrap()).unwrap();
    value["version"] = json!(1);
    value.as_object_mut().unwrap().remove("checksum");
    value["checksum"] = json!(configuration_digest(&value));
    let bytes = serde_json::to_vec(&value).unwrap();
    *store.bytes.lock().unwrap() = Some(bytes.clone());
    let d = open(store.clone(), provider, scope).await;
    assert_eq!(d.inspect().await.unwrap().version, 1);
    assert_eq!(*store.bytes.lock().unwrap(), Some(bytes));
    assert_eq!(d.resume(1).await.unwrap().version, 1);
}

#[tokio::test]
async fn poll_and_cancel_publication_failures_recover_from_the_committed_revision() {
    for cancel in [false, true] {
        for committed in [false, true] {
            for revision in [4, 5] {
                let store = Arc::new(Store::default());
                let provider = Arc::new(Remote::default());
                let scope = scope();
                let d = remote(store.clone(), provider.clone(), scope.clone(), false).await;
                d.resume(1).await.unwrap();
                *store.fault.lock().unwrap() = Some((revision, committed));
                let outcome = if cancel {
                    d.cancel(3).await
                } else {
                    d.resume(3).await
                };
                assert!(outcome.is_err());
                *store.fault.lock().unwrap() = None;
                let d = remote(store, provider.clone(), scope, true).await;
                let retained = d.inspect().await.unwrap();
                let result = if cancel {
                    d.cancel(retained.revision).await.unwrap()
                } else if retained.phase == ConversationPhase::AwaitingTools {
                    retained
                } else {
                    d.resume(retained.revision).await.unwrap()
                };
                assert_eq!(
                    result.phase,
                    if cancel {
                        ConversationPhase::Abandoned
                    } else {
                        ConversationPhase::AwaitingTools
                    }
                );
                assert_eq!(provider.creates.load(Ordering::SeqCst), 1);
                let operations =
                    provider.polls.load(Ordering::SeqCst) + provider.cancels.load(Ordering::SeqCst);
                assert_eq!(operations, if revision == 5 && !committed { 2 } else { 1 });
            }
        }
    }
}

#[tokio::test]
async fn competing_poll_and_cancel_commands_dispatch_only_the_revision_winner() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Remote::default());
    let d = remote(store.clone(), provider.clone(), scope(), false).await;
    d.resume(1).await.unwrap();
    let (poll, cancel) = tokio::join!(d.resume(3), d.cancel(3));
    assert_ne!(poll.is_ok(), cancel.is_ok());
    let error = poll.err().or_else(|| cancel.err()).unwrap();
    code(error, ToolCode::ToolContinuationConflict);
    assert_eq!(
        provider.polls.load(Ordering::SeqCst) + provider.cancels.load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn abandoned_remote_work_can_be_cancelled_after_expiry_and_reopened() {
    let store = Arc::new(Store::default());
    let provider = Arc::new(Remote::default());
    let mut scope = scope();
    scope.expires_at_unix_ms -= 3800;
    let d = remote(store.clone(), provider.clone(), scope.clone(), false).await;
    let pending = d.resume(1).await.unwrap();
    let abandoned = d
        .reconcile(
            pending.revision,
            ConversationResolution::Abandon,
            "operator",
            "Original provider task fenced",
        )
        .await
        .unwrap();
    assert_eq!(abandoned.pending, pending.pending);
    assert!(d.resume(abandoned.revision).await.is_err());
    tokio::time::sleep(Duration::from_millis(250)).await;
    let d = remote(store.clone(), provider.clone(), scope.clone(), true).await;
    *store.fault.lock().unwrap() = Some((6, false));
    assert!(d.cancel(abandoned.revision).await.is_err());
    *store.fault.lock().unwrap() = None;
    assert_eq!(provider.cancels.load(Ordering::SeqCst), 1);
    let d = remote(store.clone(), provider.clone(), scope.clone(), true).await;
    let cancelling = d.inspect().await.unwrap();
    assert_eq!(cancelling.phase, ConversationPhase::ProviderCancelling);
    assert!(d.resume(cancelling.revision).await.is_err());
    let cancelled = d.cancel(cancelling.revision).await.unwrap();
    assert_eq!(cancelled.phase, ConversationPhase::Abandoned);
    assert_eq!(cancelled.pending, pending.pending);
    assert_eq!(cancelled.resolutions.len(), 1);
    let d = remote(store, provider.clone(), scope, true).await;
    assert_eq!(d.cancel(cancelled.revision).await.unwrap(), cancelled);
    assert_eq!(provider.creates.load(Ordering::SeqCst), 1);
    assert_eq!(provider.polls.load(Ordering::SeqCst), 0);
    assert_eq!(provider.cancels.load(Ordering::SeqCst), 2);
}
