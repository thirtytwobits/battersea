//! Copyright (c) Scott A Dixon
mod support;

use async_trait::async_trait;
use battersea_model::*;
use battersea_tools::*;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
struct Store {
    bytes: Mutex<Option<Vec<u8>>>,
    // Fail the acknowledgement for this revision, optionally after committing it.
    fault: Mutex<Option<(u64, bool)>>,
}
#[async_trait]
impl ToolApprovalStore for Store {
    async fn load(
        &self,
        _: &str,
        max: usize,
    ) -> Result<Option<ToolApprovalJournal>, ToolApprovalStoreError> {
        self.bytes
            .lock()
            .unwrap()
            .as_ref()
            .map(|b| {
                if b.len() > max {
                    return Err(ToolApprovalStoreError::Unavailable);
                }
                serde_json::from_slice(b).map_err(|_| ToolApprovalStoreError::Unavailable)
            })
            .transpose()
    }
    async fn compare_exchange(
        &self,
        _: &str,
        expected: Option<u64>,
        next: &ToolApprovalJournal,
    ) -> Result<(), ToolApprovalStoreError> {
        tokio::task::yield_now().await;
        let mut bytes = self.bytes.lock().unwrap();
        let previous = bytes.as_ref().map(|b| {
            serde_json::from_slice::<ToolApprovalJournal>(b)
                .unwrap()
                .revision
        });
        if previous != expected {
            return Err(ToolApprovalStoreError::Conflict);
        }
        let fault = *self.fault.lock().unwrap();
        if fault == Some((next.revision, false)) {
            return Err(ToolApprovalStoreError::Unavailable);
        }
        *bytes = Some(serde_json::to_vec(next).unwrap());
        if fault == Some((next.revision, true)) {
            return Err(ToolApprovalStoreError::Unavailable);
        }
        Ok(())
    }
}
#[derive(Default)]
struct Effect {
    calls: AtomicUsize,
    wait: bool,
    output: Option<serde_json::Value>,
}
#[async_trait]
impl EngineLocalToolExecutor for Effect {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.wait {
            std::future::pending::<()>().await;
        }
        Ok(EngineLocalToolResult::json(
            self.output.clone().unwrap_or(call.arguments),
        ))
    }
}
fn write() -> clapham::Operation {
    let mut operation = support::operation(
        "host.write",
        support::object(
            json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"]}),
        ),
    );
    operation.description = "Write a value".into();
    operation
}
fn operator(operation: clapham::Operation, effect: Arc<Effect>) -> ToolRegistry {
    support::registry(vec![(operation, effect)])
        .with_profile("operator", ["host.write"])
        .unwrap()
        .for_profile("operator")
}
fn registry(effect: Arc<Effect>) -> ToolRegistry {
    operator(write(), effect)
}
fn scope() -> DurableToolApprovalScope {
    DurableToolApprovalScope {
        run_id: "run-1".into(),
        binding_revision: "binding-1".into(),
        expires_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 60000,
    }
}
fn call(id: &str) -> EngineLocalToolCall {
    EngineLocalToolCall {
        id: id.into(),
        name: "host_write".into(),
        arguments: json!({"value":1}),
    }
}
async fn create(
    store: Arc<Store>,
    effect: Arc<Effect>,
    scope: DurableToolApprovalScope,
) -> DurableToolExecutor {
    DurableToolExecutor::create(
        store,
        registry(effect),
        scope,
        ToolApprovalJournalLimits {
            max_calls: 2,
            max_bytes: 8192,
        },
        Duration::from_secs(1),
    )
    .await
    .unwrap()
}
async fn open(
    store: Arc<Store>,
    effect: Arc<Effect>,
    scope: DurableToolApprovalScope,
) -> DurableToolExecutor {
    DurableToolExecutor::open(store, registry(effect), scope, 8192, Duration::from_secs(1))
        .await
        .unwrap()
}
async fn approve(executor: &DurableToolExecutor) -> u64 {
    let r = executor.submit(call("one"), 1).await.unwrap();
    executor
        .decide(
            "one",
            &r.calls["one"].request_revision,
            r.revision,
            ToolApprovalAction::Allow,
            "operator",
        )
        .await
        .unwrap()
        .revision
}
fn code(error: EngineAdapterRequestError, code: ToolCode) {
    assert_eq!(error.tool_code, Some(code));
}

#[tokio::test]
async fn restart_pending_approved_and_completed_requires_explicit_commands() {
    let store = Arc::new(Store::default());
    let effect = Arc::new(Effect::default());
    let scope = scope();
    let e = create(store.clone(), effect.clone(), scope.clone()).await;
    let error = e.call(call("one")).await.unwrap_err();
    assert_eq!(
        error.tool_call.as_ref().unwrap().outcome,
        ToolCallOutcome::NotDispatched
    );
    code(error, ToolCode::ToolApprovalPending);
    drop(e);
    let e = open(store.clone(), effect.clone(), scope.clone()).await;
    let r = e.inspect().await.unwrap();
    e.decide(
        "one",
        &r.calls["one"].request_revision,
        2,
        ToolApprovalAction::Allow,
        "operator",
    )
    .await
    .unwrap();
    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
    drop(e);
    let e = open(store.clone(), effect.clone(), scope.clone()).await;
    let result = e.resume("one", 3).await.unwrap();
    drop(e);
    let e = open(store, effect.clone(), scope).await;
    assert_eq!(e.resume("one", 5).await.unwrap(), result);
    assert_eq!(effect.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn write_failures_before_and_after_commit_never_replay_effects() {
    for revision in [4, 5] {
        for committed in [false, true] {
            let store = Arc::new(Store::default());
            let effect = Arc::new(Effect::default());
            let scope = scope();
            let e = create(store.clone(), effect.clone(), scope.clone()).await;
            approve(&e).await;
            *store.fault.lock().unwrap() = Some((revision, committed));
            code(
                e.resume("one", 3).await.unwrap_err(),
                ToolCode::ToolApprovalStorage,
            );
            drop(e);
            *store.fault.lock().unwrap() = None;
            let e = open(store, effect.clone(), scope).await;
            let r = e.inspect().await.unwrap();
            let revision = match r.calls["one"].state {
                StoredToolCallState::Approved => {
                    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
                    r.revision
                }
                StoredToolCallState::Intent => {
                    code(
                        e.resume("one", r.revision).await.unwrap_err(),
                        ToolCode::ToolApprovalUnresolved,
                    );
                    let resolution = if effect.calls.load(Ordering::SeqCst) == 0 {
                        ToolApprovalReconciliation::NotApplied
                    } else {
                        ToolApprovalReconciliation::Completed {
                            result: EngineLocalToolResult::json(json!({"value":1})),
                        }
                    };
                    e.reconcile(
                        "one",
                        r.revision,
                        resolution,
                        "operator",
                        "Original execution fenced; effect ledger inspected",
                    )
                    .await
                    .unwrap()
                    .revision
                }
                StoredToolCallState::Completed { .. } => r.revision,
                _ => panic!("unexpected phase"),
            };
            e.resume("one", revision).await.unwrap();
            assert_eq!(effect.calls.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn concurrent_restarts_fence_dispatch_and_stale_decisions() {
    let store = Arc::new(Store::default());
    let effect = Arc::new(Effect::default());
    let scope = scope();
    let a = create(store.clone(), effect.clone(), scope.clone()).await;
    approve(&a).await;
    let b = open(store, effect.clone(), scope).await;
    let (x, y) = tokio::join!(a.resume("one", 3), b.resume("one", 3));
    assert_ne!(x.is_ok(), y.is_ok());
    assert_eq!(effect.calls.load(Ordering::SeqCst), 1);
    code(
        a.decide("one", "wrong", 2, ToolApprovalAction::Allow, "operator")
            .await
            .unwrap_err(),
        ToolCode::ToolApprovalConflict,
    );
}

#[tokio::test]
async fn cancellation_after_dispatch_preserves_intent_across_restart() {
    let store = Arc::new(Store::default());
    let effect = Arc::new(Effect {
        wait: true,
        ..Effect::default()
    });
    let scope = scope();
    let e = create(store.clone(), effect.clone(), scope.clone()).await;
    approve(&e).await;
    let task = tokio::spawn(async move { e.resume("one", 3).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while effect.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let e = open(store, effect.clone(), scope).await;
    code(
        e.resume("one", 4).await.unwrap_err(),
        ToolCode::ToolApprovalUnresolved,
    );
    code(
        e.submit(call("two"), 4).await.unwrap_err(),
        ToolCode::ToolApprovalUnresolved,
    );
    e.reconcile(
        "one",
        4,
        ToolApprovalReconciliation::Abandon,
        "operator",
        "Executor fenced; effect cannot be recovered",
    )
    .await
    .unwrap();
    let error = e.resume("one", 5).await.unwrap_err();
    assert_eq!(
        error.tool_call.as_ref().unwrap().outcome,
        ToolCallOutcome::Uncertain
    );
    code(error, ToolCode::ToolApprovalDenied);
    assert_eq!(effect.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn denial_cancellation_and_pending_do_not_dispatch() {
    for action in [ToolApprovalAction::Deny, ToolApprovalAction::Cancel] {
        let effect = Arc::new(Effect::default());
        let e = create(Arc::new(Store::default()), effect.clone(), scope()).await;
        let r = e.submit(call("one"), 1).await.unwrap();
        code(
            e.resume("one", 2).await.unwrap_err(),
            ToolCode::ToolApprovalPending,
        );
        code(
            e.decide("one", "changed", 2, action, "operator")
                .await
                .unwrap_err(),
            ToolCode::ToolApprovalConflict,
        );
        let r = e
            .decide(
                "one",
                &r.calls["one"].request_revision,
                2,
                action,
                "operator",
            )
            .await
            .unwrap();
        assert_eq!(
            e.decide(
                "one",
                &r.calls["one"].request_revision,
                3,
                action,
                "operator"
            )
            .await
            .unwrap()
            .revision,
            3
        );
        code(
            e.resume("one", 3).await.unwrap_err(),
            ToolCode::ToolApprovalDenied,
        );
        assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
    }
    let effect = Arc::new(Effect::default());
    let e = create(Arc::new(Store::default()), effect.clone(), scope()).await;
    approve(&e).await;
    let r = e.inspect().await.unwrap();
    e.decide(
        "one",
        &r.calls["one"].request_revision,
        3,
        ToolApprovalAction::Cancel,
        "operator",
    )
    .await
    .unwrap();
    assert!(e.resume("one", 4).await.is_err());
    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn invalid_calls_capacity_and_changed_bindings_never_dispatch() {
    let store = Arc::new(Store::default());
    let effect = Arc::new(Effect::default());
    let scope = scope();
    let e = create(store.clone(), effect.clone(), scope.clone()).await;
    let mut bad = call("one");
    bad.arguments = json!({"value":"wrong"});
    code(
        e.submit(bad, 1).await.unwrap_err(),
        ToolCode::InvalidToolArguments,
    );
    assert_eq!(e.inspect().await.unwrap().revision, 1);
    e.submit(call("one"), 1).await.unwrap();
    code(
        e.submit(call("one"), 2).await.unwrap_err(),
        ToolCode::InvalidToolCallId,
    );
    e.submit(call("two"), 2).await.unwrap();
    code(
        e.submit(call("three"), 3).await.unwrap_err(),
        ToolCode::ToolApprovalCapacity,
    );
    let mut changed = scope;
    changed.binding_revision = "other".into();
    assert!(DurableToolExecutor::open(
        store,
        registry(effect.clone()),
        changed,
        8192,
        Duration::from_secs(1)
    )
    .await
    .is_err());
    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn incompatible_or_corrupt_reads_do_not_write_or_upgrade() {
    for mutation in 0..4 {
        let store = Arc::new(Store::default());
        let effect = Arc::new(Effect::default());
        let scope = scope();
        let e = create(store.clone(), effect.clone(), scope.clone()).await;
        approve(&e).await;
        let mut r = e.inspect().await.unwrap();
        match mutation {
            0 => r.version += 1,
            1 => r.calls.get_mut("one").unwrap().call.arguments = json!({"value":2}),
            2 => r.limits.max_bytes = 1,
            _ => r.calls.get_mut("one").unwrap().request_revision = "corrupt".into(),
        }
        let bytes = serde_json::to_vec(&r).unwrap();
        *store.bytes.lock().unwrap() = Some(bytes.clone());
        assert!(DurableToolExecutor::open(
            store.clone(),
            registry(effect.clone()),
            scope,
            8192,
            Duration::from_secs(1)
        )
        .await
        .is_err());
        assert_eq!(*store.bytes.lock().unwrap(), Some(bytes));
        assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn persisted_expiry_cannot_be_renewed_but_completed_results_remain_readable() {
    for completed in [false, true] {
        let store = Arc::new(Store::default());
        let effect = Arc::new(Effect::default());
        let mut scope = scope();
        scope.expires_at_unix_ms -= 59500;
        let e = create(store.clone(), effect.clone(), scope.clone()).await;
        approve(&e).await;
        if completed {
            e.resume("one", 3).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(550)).await;
        let e = open(store.clone(), effect.clone(), scope.clone()).await;
        if completed {
            e.resume("one", 5).await.unwrap();
        } else {
            code(
                e.resume("one", 3).await.unwrap_err(),
                ToolCode::ToolApprovalExpired,
            );
        }
        scope.expires_at_unix_ms += 60000;
        assert!(DurableToolExecutor::open(
            store,
            registry(effect.clone()),
            scope,
            8192,
            Duration::from_secs(1)
        )
        .await
        .is_err());
        assert_eq!(effect.calls.load(Ordering::SeqCst), usize::from(completed));
    }
}

struct ParkIntent {
    store: Arc<Store>,
    committed: tokio::sync::Notify,
}
#[async_trait]
impl ToolApprovalStore for ParkIntent {
    async fn load(
        &self,
        run: &str,
        max: usize,
    ) -> Result<Option<ToolApprovalJournal>, ToolApprovalStoreError> {
        self.store.load(run, max).await
    }
    async fn compare_exchange(
        &self,
        run: &str,
        expected: Option<u64>,
        next: &ToolApprovalJournal,
    ) -> Result<(), ToolApprovalStoreError> {
        self.store.compare_exchange(run, expected, next).await?;
        if next.revision == 4 {
            self.committed.notify_one();
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}
#[tokio::test]
async fn cancelled_intent_acknowledgement_is_reconciled_without_dispatch() {
    let store = Arc::new(Store::default());
    let effect = Arc::new(Effect::default());
    let scope = scope();
    let parked = Arc::new(ParkIntent {
        store: store.clone(),
        committed: tokio::sync::Notify::new(),
    });
    let e = DurableToolExecutor::create(
        parked.clone(),
        registry(effect.clone()),
        scope.clone(),
        ToolApprovalJournalLimits {
            max_calls: 2,
            max_bytes: 8192,
        },
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    approve(&e).await;
    let task = tokio::spawn(async move { e.resume("one", 3).await });
    tokio::time::timeout(Duration::from_secs(2), parked.committed.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let e = open(store, effect.clone(), scope).await;
    code(
        e.resume("one", 4).await.unwrap_err(),
        ToolCode::ToolApprovalUnresolved,
    );
    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
    e.reconcile(
        "one",
        4,
        ToolApprovalReconciliation::NotApplied,
        "operator",
        "Original executor stopped before dispatch",
    )
    .await
    .unwrap();
    e.resume("one", 5).await.unwrap();
    assert_eq!(effect.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn changed_definition_and_profile_reject_previously_approved_dispatch() {
    for profile in [false, true] {
        let store = Arc::new(Store::default());
        let effect = Arc::new(Effect::default());
        let scope = scope();
        let e = create(store.clone(), effect.clone(), scope.clone()).await;
        approve(&e).await;
        let mut changed = registry(effect.clone());
        if profile {
            changed = changed.for_profile("untrusted");
        } else {
            let mut operation = write();
            operation.description = "Changed contract".into();
            changed = operator(operation, effect.clone());
        }
        let e = DurableToolExecutor::open(store, changed, scope, 8192, Duration::from_secs(1))
            .await
            .unwrap();
        assert!(e.resume("one", 3).await.is_err());
        assert!(matches!(
            e.inspect().await.unwrap().calls["one"].state,
            StoredToolCallState::Approved
        ));
        assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn oversized_results_retain_intent_and_reconciliation_enforces_output_schema() {
    let store = Arc::new(Store::default());
    let effect = Arc::new(Effect {
        output: Some(json!({"value":"x".repeat(10000)})),
        ..Effect::default()
    });
    let scope = scope();
    let e = create(store.clone(), effect.clone(), scope.clone()).await;
    approve(&e).await;
    let error = e.resume("one", 3).await.unwrap_err();
    assert_eq!(
        error.tool_call.as_ref().unwrap().outcome,
        ToolCallOutcome::Completed
    );
    code(error, ToolCode::ToolApprovalCapacity);
    let e = open(store, effect.clone(), scope).await;
    assert!(matches!(
        e.inspect().await.unwrap().calls["one"].state,
        StoredToolCallState::Intent
    ));
    code(
        e.reconcile(
            "one",
            4,
            ToolApprovalReconciliation::Completed {
                result: EngineLocalToolResult::json(json!("x".repeat(10000))),
            },
            "operator",
            "Retained effect result",
        )
        .await
        .unwrap_err(),
        ToolCode::ToolApprovalCapacity,
    );
    assert_eq!(effect.calls.load(Ordering::SeqCst), 1);

    let store = Arc::new(Store::default());
    let effect = Arc::new(Effect::default());
    let mut operation = support::operation("host.write", support::object(json!({"type":"object"})));
    operation.description = "Write".into();
    let registry = operator(
        support::with_output(operation, json!({"type":"object","required":["receipt"]})),
        effect.clone(),
    );
    let e = DurableToolExecutor::create(
        store.clone(),
        registry,
        crate::scope(),
        ToolApprovalJournalLimits {
            max_calls: 2,
            max_bytes: 8192,
        },
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    approve(&e).await;
    *store.fault.lock().unwrap() = Some((4, true));
    assert!(e.resume("one", 3).await.is_err());
    *store.fault.lock().unwrap() = None;
    code(
        e.reconcile(
            "one",
            4,
            ToolApprovalReconciliation::Completed {
                result: EngineLocalToolResult::json(json!({"value":1})),
            },
            "operator",
            "Retained effect result",
        )
        .await
        .unwrap_err(),
        ToolCode::InvalidToolResult,
    );
    assert_eq!(e.inspect().await.unwrap().revision, 4);
    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn acceptance_pending_and_decision_write_errors_are_recovered_by_inspection() {
    for failed_revision in [1, 2, 3] {
        for committed in [false, true] {
            let store = Arc::new(Store::default());
            let effect = Arc::new(Effect::default());
            let scope = scope();
            *store.fault.lock().unwrap() = Some((failed_revision, committed));
            let created = DurableToolExecutor::create(
                store.clone(),
                registry(effect.clone()),
                scope.clone(),
                ToolApprovalJournalLimits {
                    max_calls: 2,
                    max_bytes: 8192,
                },
                Duration::from_secs(1),
            )
            .await;
            if failed_revision == 1 {
                assert!(created.is_err());
            } else {
                let e = created.unwrap();
                let submitted = e.submit(call("one"), 1).await;
                if failed_revision == 2 {
                    assert!(submitted.is_err());
                } else {
                    let r = submitted.unwrap();
                    assert!(e
                        .decide(
                            "one",
                            &r.calls["one"].request_revision,
                            2,
                            ToolApprovalAction::Allow,
                            "operator"
                        )
                        .await
                        .is_err());
                }
            }
            assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
            *store.fault.lock().unwrap() = None;
            let empty = store.bytes.lock().unwrap().is_none();
            let e = if empty {
                create(store.clone(), effect.clone(), scope.clone()).await
            } else {
                open(store.clone(), effect.clone(), scope.clone()).await
            };
            let mut r = e.inspect().await.unwrap();
            if r.calls.is_empty() {
                r = e.submit(call("one"), r.revision).await.unwrap();
            }
            if matches!(r.calls["one"].state, StoredToolCallState::Pending) {
                r = e
                    .decide(
                        "one",
                        &r.calls["one"].request_revision,
                        r.revision,
                        ToolApprovalAction::Allow,
                        "operator",
                    )
                    .await
                    .unwrap();
            }
            e.resume("one", r.revision).await.unwrap();
            assert_eq!(effect.calls.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn terminal_reconciliation_cannot_be_loaded_as_an_executable_call() {
    let store = Arc::new(Store::default());
    let effect = Arc::new(Effect::default());
    let scope = scope();
    let e = create(store.clone(), effect.clone(), scope.clone()).await;
    approve(&e).await;
    *store.fault.lock().unwrap() = Some((5, false));
    assert!(e.resume("one", 3).await.is_err());
    *store.fault.lock().unwrap() = None;
    let completed = e
        .reconcile(
            "one",
            4,
            ToolApprovalReconciliation::Completed {
                result: EngineLocalToolResult::json(json!({"value":1})),
            },
            "operator",
            "Retained result from the fenced execution",
        )
        .await
        .unwrap();
    for mutation in 0..3 {
        let mut broken = completed.clone();
        let entry = broken.calls.get_mut("one").unwrap();
        match mutation {
            0 => entry.state = StoredToolCallState::Approved,
            1 => {
                entry.state = StoredToolCallState::Completed {
                    result: EngineLocalToolResult::json(json!({"value":2})),
                }
            }
            _ => {
                let mut extra = entry.resolutions[0].clone();
                extra.revision += 1;
                extra.resolution = ToolApprovalReconciliation::NotApplied;
                entry.resolutions.push(extra);
                broken.revision += 1;
            }
        }
        let bytes = serde_json::to_vec(&broken).unwrap();
        *store.bytes.lock().unwrap() = Some(bytes.clone());
        assert!(DurableToolExecutor::open(
            store.clone(),
            registry(effect.clone()),
            scope.clone(),
            8192,
            Duration::from_secs(1)
        )
        .await
        .is_err());
        assert_eq!(*store.bytes.lock().unwrap(), Some(bytes));
    }
    assert_eq!(effect.calls.load(Ordering::SeqCst), 1);
}

/// contracts/recovery.md (Retention): a journal settles once every call is terminal. Until
/// then a decision, dispatch or reconciliation waits on it, and the run it serves still reads
/// the results of a settled one.
#[tokio::test]
async fn a_journal_settles_only_when_every_call_is_terminal() {
    for ending in ["deny", "cancel", "complete", "abandon"] {
        let store = Arc::new(Store::default());
        let effect = Arc::new(Effect::default());
        let e = create(store.clone(), effect.clone(), scope()).await;
        assert!(e.inspect().await.unwrap().is_settled(), "{ending}");
        let r = e.submit(call("one"), 1).await.unwrap();
        assert!(
            !r.is_settled(),
            "{ending}: a pending call waits on a decision"
        );
        let action = match ending {
            "deny" => ToolApprovalAction::Deny,
            "cancel" => ToolApprovalAction::Cancel,
            _ => ToolApprovalAction::Allow,
        };
        let mut r = e
            .decide(
                "one",
                &r.calls["one"].request_revision,
                r.revision,
                action,
                "operator",
            )
            .await
            .unwrap();
        match ending {
            "complete" => {
                assert!(!r.is_settled(), "an approved call waits on dispatch");
                let result = e.resume("one", r.revision).await.unwrap();
                r = e.inspect().await.unwrap();
                assert_eq!(e.resume("one", r.revision).await.unwrap(), result);
            }
            "abandon" => {
                // The dispatched call's outcome is never published.
                *store.fault.lock().unwrap() = Some((r.revision + 2, false));
                assert!(e.resume("one", r.revision).await.is_err());
                *store.fault.lock().unwrap() = None;
                r = e.inspect().await.unwrap();
                assert!(matches!(r.calls["one"].state, StoredToolCallState::Intent));
                assert!(!r.is_settled(), "an intent waits on reconciliation");
                r = e
                    .reconcile(
                        "one",
                        r.revision,
                        ToolApprovalReconciliation::Abandon,
                        "operator",
                        "Executor fenced; effect cannot be recovered",
                    )
                    .await
                    .unwrap();
            }
            _ => {}
        }
        assert!(r.is_settled(), "{ending}");
        let r = e.submit(call("two"), r.revision).await.unwrap();
        assert!(
            !r.is_settled(),
            "{ending}: a later call unsettles the journal"
        );
    }
}

#[tokio::test]
async fn a_journal_written_by_battersea_0_17_reads_unchanged() {
    let written = include_str!("fixtures/journal-v0.17.0.json");
    let journal: ToolApprovalJournal = serde_json::from_str(written).unwrap();
    journal.validate().unwrap();
    assert_eq!(journal.version, TOOL_APPROVAL_JOURNAL_VERSION);
    assert_eq!(
        serde_json::to_string_pretty(&journal).unwrap() + "\n",
        written
    );
    let store = Arc::new(Store {
        bytes: Mutex::new(Some(written.as_bytes().to_vec())),
        ..Store::default()
    });
    let effect = Arc::new(Effect::default());
    let e = DurableToolExecutor::open(
        store,
        registry(effect.clone()),
        journal.scope.clone(),
        8192,
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(e.inspect().await.unwrap()).unwrap(),
        serde_json::to_value(&journal).unwrap()
    );
    assert!(e.resume("two", journal.revision).await.is_err());
    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
}
