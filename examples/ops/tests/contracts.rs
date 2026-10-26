//! Copyright (c) Scott A Dixon
//! The behaviour of Battersea's operations over the fixture host, through one composed set.
use battersea_ops::codes;
use clapham::{Cancellation, Category, ComposedSet, Evidence, OAuthContext, RecordSink, Refusal};
use ops_example::{agent, operator, seeded_decision, FINISHED_RUN, PENDING_CALL, PENDING_RUN};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

struct Daemon(ComposedSet);
impl Daemon {
    fn new() -> Self {
        Self(ops_example::composed().unwrap())
    }
    async fn call_with(
        &self,
        auth: OAuthContext,
        name: &str,
        input: Value,
        preview: bool,
    ) -> (Vec<Value>, Result<Value, Refusal>) {
        let records = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let records = records.clone();
            RecordSink::new(move |record| {
                records.lock().unwrap().push(record);
                async { Ok(()) }
            })
        };
        let result = self
            .0
            .call(
                &self.0.all(),
                auth,
                name,
                input,
                sink,
                Cancellation::new(),
                preview,
            )
            .await
            .map(|outcome| outcome.result.unwrap());
        let records = records.lock().unwrap().clone();
        (records, result)
    }
    async fn call(&self, name: &str, input: Value) -> Result<Value, Refusal> {
        self.call_with(operator(), name, input, false).await.1
    }
}

#[tokio::test]
async fn an_activation_streams_its_events_and_its_tool_call_waits_for_a_decision() {
    let daemon = Daemon::new();
    let (records, status) = daemon
        .call_with(
            operator(),
            "battersea.run.activate",
            json!({"flow_key": "example", "node_id": "text", "parameters": {"text": "Record it."}}),
            false,
        )
        .await;
    let status = status.unwrap();
    assert_eq!(status["status"], "awaiting_approval");
    assert_eq!(status["terminal"], false);
    let kinds = records
        .iter()
        .map(|r| r["kind"].clone())
        .collect::<Vec<_>>();
    assert!(kinds.contains(&json!("tool_call_started")), "{records:?}");
    let run_id = status["run_id"].clone();
    let journal = daemon
        .call("battersea.approval.list", json!({"run_id": run_id}))
        .await
        .unwrap();
    assert_eq!(journal["calls"][0]["state"], "pending");
    assert_eq!(journal["calls"][0]["tool_name"], "fixture.write");
}

#[tokio::test]
async fn an_operator_approves_a_pending_call_and_resumes_its_run() {
    let daemon = Daemon::new();
    let refused = daemon
        .call("battersea.run.resume", json!({"run_id": PENDING_RUN}))
        .await
        .unwrap_err();
    assert_eq!(refused.code, "tool_approval_pending");
    let decided = daemon
        .call("battersea.approval.decide", seeded_decision("allow").await)
        .await
        .unwrap();
    assert_eq!(decided["calls"][0]["call_id"], PENDING_CALL);
    assert_eq!(decided["calls"][0]["state"], "approved");
    let resumed = daemon
        .call("battersea.run.resume", json!({"run_id": PENDING_RUN}))
        .await
        .unwrap();
    assert_eq!(resumed["status"], "succeeded");
    let journal = daemon
        .call("battersea.approval.list", json!({"run_id": PENDING_RUN}))
        .await
        .unwrap();
    assert_eq!(journal["calls"][0]["state"], "completed");
}

#[tokio::test]
async fn the_runs_own_agent_grant_cannot_decide_its_calls() {
    let daemon = Daemon::new();
    let own = OAuthContext::from_verified(
        agent().issuer(),
        agent().audience(),
        agent().subject(),
        ops_example::SCOPES.iter().copied(),
    );
    let (_, refused) = daemon
        .call_with(
            own,
            "battersea.approval.decide",
            seeded_decision("allow").await,
            false,
        )
        .await;
    let refused = refused.unwrap_err();
    assert_eq!(refused.code, codes::OWN_GRANT_DECISION);
    assert_eq!(refused.category, Category::Refused);
    assert_eq!(refused.evidence, Evidence::NotDispatched);
    let (_, refused) = daemon
        .call_with(
            agent(),
            "battersea.approval.decide",
            seeded_decision("allow").await,
            false,
        )
        .await;
    assert_eq!(refused.unwrap_err().code, "insufficient_scope");
    let journal = daemon
        .call("battersea.approval.list", json!({"run_id": PENDING_RUN}))
        .await
        .unwrap();
    assert_eq!(journal["calls"][0]["state"], "pending");
}

#[tokio::test]
async fn an_upgrade_is_previewed_then_applied_at_the_previewed_revision() {
    let daemon = Daemon::new();
    let (_, preview) = daemon
        .call_with(
            operator(),
            "battersea.flow.upgrade",
            json!({"flow_key": "example"}),
            true,
        )
        .await;
    let preview = preview.unwrap();
    assert_eq!(preview["applied"], false);
    let revision = preview["source_revision"].clone();
    let missing = daemon
        .call("battersea.flow.upgrade", json!({"flow_key": "example"}))
        .await
        .unwrap_err();
    assert_eq!(missing.code, codes::UPGRADE_REVISION_REQUIRED);
    assert_eq!(missing.category, Category::Usage);
    let stale = daemon
        .call(
            "battersea.flow.upgrade",
            json!({"flow_key": "example", "expected_revision": "stale"}),
        )
        .await
        .unwrap_err();
    assert_eq!(stale.code, codes::STALE_UPGRADE_REVISION);
    let (_, stale_preview) = daemon
        .call_with(
            operator(),
            "battersea.flow.upgrade",
            json!({"flow_key": "example", "expected_revision": "stale"}),
            true,
        )
        .await;
    assert_eq!(
        stale_preview.unwrap_err().code,
        codes::STALE_UPGRADE_REVISION
    );
    let applied = daemon
        .call(
            "battersea.flow.upgrade",
            json!({"flow_key": "example", "expected_revision": revision}),
        )
        .await
        .unwrap();
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["document"], preview["document"]);
}

#[tokio::test]
async fn reads_report_stored_flows_runs_and_connections() {
    let daemon = Daemon::new();
    let flows = daemon.call("battersea.flow.list", json!({})).await.unwrap();
    assert_eq!(flows["flows"][0]["flow_key"], "example");
    assert!(flows["flows"][1]["unavailable_reason"].is_string());
    let future = daemon
        .call("battersea.flow.get", json!({"flow_key": "future"}))
        .await
        .unwrap_err();
    assert_eq!(future.code, codes::INVALID_FLOW);
    let missing = daemon
        .call("battersea.flow.get", json!({"flow_key": "missing"}))
        .await
        .unwrap_err();
    assert_eq!(missing.category, Category::NotFound);
    let (records, finished) = daemon
        .call_with(
            operator(),
            "battersea.run.watch",
            json!({"run_id": FINISHED_RUN}),
            false,
        )
        .await;
    assert_eq!(finished.unwrap()["terminal"], true);
    assert_eq!(records.len(), 2);
    let refreshed = daemon
        .call("battersea.mcp.refresh", json!({"server_id": "remote"}))
        .await
        .unwrap();
    assert_eq!(refreshed["binding_revision"], "binding-2");
    let servers = daemon
        .call("battersea.mcp.servers", json!({}))
        .await
        .unwrap();
    assert_eq!(servers["servers"][0]["binding_revision"], "binding-2");
}

#[tokio::test]
async fn every_refusal_is_one_the_operation_declares() {
    let daemon = Daemon::new();
    for (name, input) in [
        ("battersea.flow.get", json!({"flow_key": "missing"})),
        ("battersea.run.status", json!({"run_id": "run-9"})),
        ("battersea.run.cancel", json!({"run_id": "run-9"})),
        ("battersea.run.resume", json!({"run_id": PENDING_RUN})),
        ("battersea.approval.list", json!({"run_id": "run-9"})),
        (
            "battersea.approval.reconcile",
            json!({
                "run_id": PENDING_RUN, "call_id": PENDING_CALL, "expected_revision": 2,
                "resolution": {"outcome": "not_applied"}, "proof": "Not applied."
            }),
        ),
        (
            "battersea.approval.reconcile",
            json!({
                "run_id": PENDING_RUN, "call_id": PENDING_CALL, "expected_revision": 2,
                "resolution": {"outcome": "completed", "result": 1}, "proof": "Applied."
            }),
        ),
        (
            "battersea.run.activate",
            json!({"flow_key": "example", "node_id": "absent"}),
        ),
        ("battersea.mcp.refresh", json!({"server_id": "absent"})),
    ] {
        let refusal = daemon.call(name, input).await.unwrap_err();
        assert_ne!(refusal.code, "undeclared_refusal", "{name}: {refusal:?}");
        assert_eq!(
            codes::category(&refusal.code),
            Some(refusal.category),
            "{name}"
        );
    }
}

#[test]
fn every_code_is_declared_once_under_its_category() {
    let declared = codes::declared();
    for code in codes::ALL {
        assert_eq!(
            codes::category(code),
            declared.get(*code).copied(),
            "{code}"
        );
    }
    assert_eq!(declared["activation_cancelled"], Category::Interrupted);
    assert_eq!(declared["tool_approval_pending"], Category::Refused);
    assert_eq!(declared["graph_cycle"], Category::InvalidInput);
    let host = codes::refusal("a_code_battersea_does_not_declare", "Detail.");
    assert_eq!(host.code, codes::HOST_FAILURE);
    assert_eq!(
        host.details,
        Some(json!({"code": "a_code_battersea_does_not_declare"}))
    );
}

#[tokio::test]
async fn concurrent_activations_are_distinct_runs() {
    let daemon = Daemon::new();
    let activation =
        || json!({"flow_key": "example", "node_id": "text", "parameters": {"text": "Record it."}});
    let (first, second) = tokio::join!(
        daemon.call("battersea.run.activate", activation()),
        daemon.call("battersea.run.activate", activation()),
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_ne!(first["run_id"], second["run_id"]);
    for run in [first, second] {
        let journal = daemon
            .call("battersea.approval.list", json!({"run_id": run["run_id"]}))
            .await
            .unwrap();
        assert_eq!(journal["calls"].as_array().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn a_cancelled_run_does_not_resume_its_approved_call() {
    let daemon = Daemon::new();
    daemon
        .call("battersea.approval.decide", seeded_decision("allow").await)
        .await
        .unwrap();
    let cancelled = daemon
        .call("battersea.run.cancel", json!({"run_id": PENDING_RUN}))
        .await
        .unwrap();
    assert_eq!(cancelled["status"], "cancelled");
    let refused = daemon
        .call("battersea.run.resume", json!({"run_id": PENDING_RUN}))
        .await
        .unwrap_err();
    assert_eq!(refused.code, "execution_terminal");
    assert_eq!(refused.category, Category::Conflict);
    let journal = daemon
        .call("battersea.approval.list", json!({"run_id": PENDING_RUN}))
        .await
        .unwrap();
    assert_eq!(journal["calls"][0]["state"], "approved");
    let status = daemon
        .call("battersea.run.status", json!({"run_id": PENDING_RUN}))
        .await
        .unwrap();
    assert_eq!(status["status"], "cancelled");
}
