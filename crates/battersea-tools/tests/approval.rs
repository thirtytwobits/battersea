//! Copyright (c) Scott A Dixon
mod support;

use async_trait::async_trait;
use battersea_model::*;
use battersea_tools::*;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

struct Policy {
    decision: ToolApprovalDecision,
    revisions: Mutex<Vec<String>>,
}
impl ToolApprovalPolicy for Policy {
    fn decide(&self, request: ToolApprovalRequest<'_>) -> ToolApprovalDecision {
        assert_eq!(request.call.name, request.tool.name);
        assert!(request.call.arguments["value"].is_u64());
        self.revisions.lock().unwrap().push(request.revision.into());
        self.decision
    }
}
#[derive(Default)]
struct Effect {
    calls: AtomicUsize,
    wait: bool,
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
        Ok(EngineLocalToolResult::json(call.arguments))
    }
}
fn tool() -> clapham::Operation {
    let mut operation = support::operation(
        "host.write",
        support::object(
            json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"]}),
        ),
    );
    operation.description = "Write a value".into();
    operation
}
fn registry(effect: Arc<Effect>, tool: clapham::Operation) -> ToolRegistry {
    support::registry(vec![(tool, effect)])
        .with_profile("operator", ["host.write"])
        .unwrap()
        .for_profile("operator")
}
/// A scope whose deadline is five seconds after `clock`'s current instant.
fn scope(clock: &ManualClock) -> ToolApprovalScope {
    ToolApprovalScope {
        run_id: "run-1".into(),
        binding_revision: "binding-1".into(),
        deadline: clock.instant() + Duration::from_secs(5),
    }
}
fn policy(decision: ToolApprovalDecision) -> Arc<Policy> {
    Arc::new(Policy {
        decision,
        revisions: Mutex::new(Vec::new()),
    })
}
fn call(id: &str, value: serde_json::Value) -> EngineLocalToolCall {
    EngineLocalToolCall {
        id: id.into(),
        name: "host_write".into(),
        arguments: json!({"value":value}),
    }
}
fn refused(error: EngineAdapterRequestError, code: ToolCode) {
    assert_eq!(error.tool_code, Some(code));
    assert_eq!(
        error.tool_call.unwrap().outcome,
        ToolCallOutcome::NotDispatched
    );
}

#[tokio::test]
async fn schema_and_profile_checks_precede_host_policy() {
    let clock = ManualClock::new(0);
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Allow);
    let registry = registry(effect.clone(), tool());
    let executor = ApprovedToolExecutor::new(
        registry.clone(),
        scope(&clock),
        policy.clone(),
        8,
        Arc::new(clock.clone()),
    )
    .unwrap();
    refused(
        executor
            .call(call("bad", json!("wrong")))
            .await
            .unwrap_err(),
        ToolCode::InvalidToolArguments,
    );
    let forbidden = ApprovedToolExecutor::new(
        registry.for_profile("forged"),
        scope(&clock),
        policy.clone(),
        8,
        Arc::new(clock.clone()),
    )
    .unwrap();
    refused(
        forbidden.call(call("bad", json!(1))).await.unwrap_err(),
        ToolCode::UnregisteredTool,
    );
    assert!(policy.revisions.lock().unwrap().is_empty());
    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn denied_pending_and_expired_calls_never_dispatch() {
    let clock = ManualClock::new(0);
    for (decision, code) in [
        (ToolApprovalDecision::Deny, ToolCode::ToolApprovalDenied),
        (ToolApprovalDecision::Pending, ToolCode::ToolApprovalPending),
    ] {
        let effect = Arc::new(Effect::default());
        let policy = policy(decision);
        let executor = ApprovedToolExecutor::new(
            registry(effect.clone(), tool()),
            scope(&clock),
            policy.clone(),
            8,
            Arc::new(clock.clone()),
        )
        .unwrap();
        refused(
            executor.call(call("one", json!(1))).await.unwrap_err(),
            code,
        );
        refused(
            executor.call(call("one", json!(2))).await.unwrap_err(),
            ToolCode::InvalidToolCallId,
        );
        assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
        assert_eq!(policy.revisions.lock().unwrap().len(), 1);
    }
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Allow);
    let mut expired = scope(&clock);
    expired.deadline = clock.instant();
    let executor = ApprovedToolExecutor::new(
        registry(effect.clone(), tool()),
        expired,
        policy.clone(),
        8,
        Arc::new(clock.clone()),
    )
    .unwrap();
    refused(
        executor.call(call("one", json!(1))).await.unwrap_err(),
        ToolCode::ToolApprovalExpired,
    );
    assert!(policy.revisions.lock().unwrap().is_empty());
    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn concurrent_and_altered_duplicates_share_one_dispatch_reservation() {
    let clock = ManualClock::new(0);
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Allow);
    let executor = ApprovedToolExecutor::new(
        registry(effect.clone(), tool()),
        scope(&clock),
        policy.clone(),
        8,
        Arc::new(clock.clone()),
    )
    .unwrap();
    let clone = executor.clone();
    let (first, duplicate) = tokio::join!(
        executor.call(call("one", json!(1))),
        clone.call(call("one", json!(2)))
    );
    assert!(first.is_ok());
    refused(duplicate.unwrap_err(), ToolCode::InvalidToolCallId);
    assert_eq!(effect.calls.load(Ordering::SeqCst), 1);
    assert_eq!(policy.revisions.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn capacity_preserves_existing_reservations_and_distinct_calls_execute() {
    let clock = ManualClock::new(0);
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Allow);
    let executor = ApprovedToolExecutor::new(
        registry(effect.clone(), tool()),
        scope(&clock),
        policy.clone(),
        2,
        Arc::new(clock.clone()),
    )
    .unwrap();
    let (a, b) = tokio::join!(
        executor.call(call("one", json!(1))),
        executor.call(call("two", json!(2)))
    );
    assert!(a.is_ok() && b.is_ok());
    refused(
        executor.call(call("three", json!(3))).await.unwrap_err(),
        ToolCode::ToolApprovalCapacity,
    );
    refused(
        executor.call(call("one", json!(1))).await.unwrap_err(),
        ToolCode::InvalidToolCallId,
    );
    assert_eq!(effect.calls.load(Ordering::SeqCst), 2);
    assert_eq!(policy.revisions.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn approval_revision_binds_run_binding_record_identity_and_arguments() {
    let clock = ManualClock::new(0);
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Deny);
    for change in 0..6 {
        let mut scope = scope(&clock);
        let mut tool = tool();
        let mut call = call("one", json!(1));
        match change {
            1 => scope.run_id = "run-2".into(),
            2 => scope.binding_revision = "binding-2".into(),
            3 => tool.description = "Changed contract".into(),
            4 => call.id = "two".into(),
            5 => call.arguments = json!({"value":2}),
            _ => (),
        }
        let executor = ApprovedToolExecutor::new(
            registry(effect.clone(), tool),
            scope,
            policy.clone(),
            8,
            Arc::new(clock.clone()),
        )
        .unwrap();
        refused(
            executor.call(call).await.unwrap_err(),
            ToolCode::ToolApprovalDenied,
        );
    }
    let revisions = policy.revisions.lock().unwrap();
    assert_eq!(
        revisions
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        6
    );
}

#[tokio::test]
async fn cancelled_and_timed_out_executions_cannot_be_repeated() {
    let clock = ManualClock::new(0);
    for timeout in [false, true] {
        let effect = Arc::new(Effect {
            wait: true,
            ..Effect::default()
        });
        let policy = policy(ToolApprovalDecision::Allow);
        let mut scope = scope(&clock);
        if timeout {
            scope.deadline = clock.instant() + Duration::from_millis(50);
        }
        let executor = ApprovedToolExecutor::new(
            registry(effect.clone(), tool()),
            scope,
            policy.clone(),
            8,
            Arc::new(clock.clone()),
        )
        .unwrap();
        let cloned = executor.clone();
        let task = tokio::spawn(async move { cloned.call(call("one", json!(1))).await });
        while effect.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        if timeout {
            // The executing tool never returns; the deadline elapses only when advanced.
            assert_eq!(
                clock.timer_within(Duration::from_millis(50)).await,
                Duration::from_millis(50)
            );
            clock.advance(Duration::from_millis(49));
            tokio::task::yield_now().await;
            assert!(!task.is_finished());
            clock.advance(Duration::from_millis(1));
            let error = task.await.unwrap().unwrap_err();
            assert_eq!(error.classification, ErrorKind::Timeout);
            assert_eq!(error.tool_call.unwrap().outcome, ToolCallOutcome::Uncertain);
        } else {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        }
        assert!(executor.call(call("one", json!(1))).await.is_err());
        assert_eq!(effect.calls.load(Ordering::SeqCst), 1);
        assert_eq!(policy.revisions.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn read_only_author_claims_cannot_override_the_host_decision() {
    let clock = ManualClock::new(0);
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Deny);
    let mut claimed = tool();
    claimed.effect = clapham::Effect::Query;
    assert!(Tool::for_operation(&claimed).effect().read_only);
    let executor = ApprovedToolExecutor::new(
        registry(effect.clone(), claimed),
        scope(&clock),
        policy,
        8,
        Arc::new(clock.clone()),
    )
    .unwrap();
    refused(
        executor.call(call("one", json!(1))).await.unwrap_err(),
        ToolCode::ToolApprovalDenied,
    );
    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn approval_keeps_registry_output_validation_and_snapshot() {
    let clock = ManualClock::new(0);
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Allow);
    let definition = support::with_output(tool(), json!({"type":"object","required":["receipt"]}));
    let registry = registry(effect.clone(), definition);
    let executor =
        ApprovedToolExecutor::new(registry, scope(&clock), policy, 8, Arc::new(clock.clone()))
            .unwrap();
    assert_eq!(executor.definitions().len(), 1);
    let error = executor.call(call("one", json!(1))).await.unwrap_err();
    assert_eq!(error.tool_code, Some(ToolCode::InvalidToolResult));
    assert_eq!(error.tool_call.unwrap().outcome, ToolCallOutcome::Completed);
    refused(
        executor.call(call("one", json!(1))).await.unwrap_err(),
        ToolCode::InvalidToolCallId,
    );
    assert_eq!(effect.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn approval_scope_and_capacity_must_be_explicit_and_bounded() {
    let clock = ManualClock::new(0);
    for case in 0..4 {
        let mut scope = scope(&clock);
        let capacity = match case {
            0 => 0,
            1 => 65537,
            2 => {
                scope.run_id.clear();
                8
            }
            _ => {
                scope.binding_revision.clear();
                8
            }
        };
        let result = ApprovedToolExecutor::new(
            support::registry(Vec::new()),
            scope,
            policy(ToolApprovalDecision::Allow),
            capacity,
            Arc::new(clock.clone()),
        );
        assert_eq!(
            result.err().unwrap().tool_code,
            Some(ToolCode::InvalidToolApproval)
        );
    }
}
