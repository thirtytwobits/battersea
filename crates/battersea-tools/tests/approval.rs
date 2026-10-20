//! Copyright (c) Scott A Dixon
use async_trait::async_trait;
use battersea_model::*;
use battersea_tools::*;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

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
fn tool() -> Tool {
    Tool::new(
        "host.write",
        json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"]})
            .as_object()
            .unwrap()
            .clone(),
    )
    .with_description("Write a value")
    .with_profiles(["operator"])
}
fn registry(effect: Arc<Effect>, tool: Tool) -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    registry.register(tool, effect).unwrap();
    registry.for_profile("operator")
}
fn scope() -> ToolApprovalScope {
    ToolApprovalScope {
        run_id: "run-1".into(),
        binding_revision: "binding-1".into(),
        deadline: Instant::now() + Duration::from_secs(5),
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
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Allow);
    let registry = registry(effect.clone(), tool());
    let executor = ApprovedToolExecutor::new(registry.clone(), scope(), policy.clone(), 8).unwrap();
    refused(
        executor
            .call(call("bad", json!("wrong")))
            .await
            .unwrap_err(),
        ToolCode::InvalidToolArguments,
    );
    let forbidden =
        ApprovedToolExecutor::new(registry.for_profile("forged"), scope(), policy.clone(), 8)
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
    for (decision, code) in [
        (ToolApprovalDecision::Deny, ToolCode::ToolApprovalDenied),
        (ToolApprovalDecision::Pending, ToolCode::ToolApprovalPending),
    ] {
        let effect = Arc::new(Effect::default());
        let policy = policy(decision);
        let executor =
            ApprovedToolExecutor::new(registry(effect.clone(), tool()), scope(), policy.clone(), 8)
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
    let mut expired = scope();
    expired.deadline = Instant::now();
    let executor =
        ApprovedToolExecutor::new(registry(effect.clone(), tool()), expired, policy.clone(), 8)
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
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Allow);
    let executor =
        ApprovedToolExecutor::new(registry(effect.clone(), tool()), scope(), policy.clone(), 8)
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
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Allow);
    let executor =
        ApprovedToolExecutor::new(registry(effect.clone(), tool()), scope(), policy.clone(), 2)
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
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Deny);
    for change in 0..6 {
        let mut scope = scope();
        let mut tool = tool();
        let mut call = call("one", json!(1));
        match change {
            1 => scope.run_id = "run-2".into(),
            2 => scope.binding_revision = "binding-2".into(),
            3 => tool.description = Some("Changed contract".into()),
            4 => call.id = "two".into(),
            5 => call.arguments = json!({"value":2}),
            _ => (),
        }
        let executor =
            ApprovedToolExecutor::new(registry(effect.clone(), tool), scope, policy.clone(), 8)
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
    for timeout in [false, true] {
        let effect = Arc::new(Effect {
            wait: true,
            ..Effect::default()
        });
        let policy = policy(ToolApprovalDecision::Allow);
        let mut scope = scope();
        if timeout {
            scope.deadline = Instant::now() + Duration::from_millis(50);
        }
        let executor =
            ApprovedToolExecutor::new(registry(effect.clone(), tool()), scope, policy.clone(), 8)
                .unwrap();
        let cloned = executor.clone();
        let task = tokio::spawn(async move { cloned.call(call("one", json!(1))).await });
        if timeout {
            let error = task.await.unwrap().unwrap_err();
            assert_eq!(error.classification, ErrorKind::Timeout);
            assert_eq!(error.tool_call.unwrap().outcome, ToolCallOutcome::Uncertain);
        } else {
            tokio::time::timeout(Duration::from_secs(2), async {
                while effect.calls.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
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
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Deny);
    let claimed = tool().with_annotations(ToolAnnotations::read_only_closed_world());
    assert!(claimed.effect().read_only);
    let executor =
        ApprovedToolExecutor::new(registry(effect.clone(), claimed), scope(), policy, 8).unwrap();
    refused(
        executor.call(call("one", json!(1))).await.unwrap_err(),
        ToolCode::ToolApprovalDenied,
    );
    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn approval_keeps_registry_output_validation_and_snapshot() {
    let effect = Arc::new(Effect::default());
    let policy = policy(ToolApprovalDecision::Allow);
    let mut definition = tool();
    definition.output_schema = Some(
        json!({"type":"object","required":["receipt"]})
            .as_object()
            .unwrap()
            .clone(),
    );
    let mut registry = registry(effect.clone(), definition);
    let executor = ApprovedToolExecutor::new(registry.clone(), scope(), policy, 8).unwrap();
    let mut extra = tool();
    extra.name = "host.other".into();
    registry.register(extra, effect.clone()).unwrap();
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
    for case in 0..4 {
        let mut scope = scope();
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
            ToolRegistry::default(),
            scope,
            policy(ToolApprovalDecision::Allow),
            capacity,
        );
        assert_eq!(
            result.err().unwrap().tool_code,
            Some(ToolCode::InvalidToolApproval)
        );
    }
}
