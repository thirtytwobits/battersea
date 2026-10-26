//! Copyright (c) Scott A Dixon
//! Battersea's operations through every Clapham front end: in-process, the CLI against its
//! daemon over Streamable HTTP and stdio, raw MCP over stdio and Streamable HTTP, and the MCP
//! client bound to each. Every front end sees the in-process result, records in order.
use clapham::{Cancellation, OAuthContext, RecordSink, ScopeGate};
use clapham_conformance::{
    run, Case, Fixture, FrontEnd, InProcess, Process, ProcessFrontEnd, ProcessKind,
};
use ops_example::{agent, operator, seeded_decision, FINISHED_RUN, PENDING_CALL, PENDING_RUN};
use serde_json::{json, Value};
use std::sync::Arc;

fn case(operation: &str, input: Value) -> Case {
    let mut case = Case::new(operation, input);
    case.auth = operator();
    case
}

/// The source revision a preview of the example flow's upgrade reports.
async fn upgrade_revision() -> String {
    let set = ops_example::composed().unwrap();
    let outcome = set
        .call(
            &set.all(),
            operator(),
            "battersea.flow.upgrade",
            json!({"flow_key": "example"}),
            RecordSink::default(),
            Cancellation::new(),
            true,
        )
        .await
        .unwrap();
    outcome.result.unwrap()["source_revision"]
        .as_str()
        .unwrap()
        .into()
}

async fn cases() -> Vec<Case> {
    // The agent's identity, holding the decision scope its grants never carry.
    let mut own_grant = case("battersea.approval.decide", seeded_decision("allow").await);
    own_grant.auth = OAuthContext::from_verified(
        agent().issuer(),
        agent().audience(),
        agent().subject(),
        ops_example::SCOPES.iter().copied(),
    );
    let mut agent_decision = case("battersea.approval.decide", seeded_decision("deny").await);
    agent_decision.auth = agent();
    vec![
        case("battersea.flow.list", json!({})),
        case("battersea.flow.get", json!({"flow_key": "example"})),
        case("battersea.flow.get", json!({"flow_key": "missing"})),
        case("battersea.flow.get", json!({"flow_key": "future"})),
        case("battersea.flow.validate", json!({"flow_key": "example"})),
        case(
            "battersea.flow.upgrade",
            json!({"flow_key": "example", "expected_revision": upgrade_revision().await}),
        ),
        case(
            "battersea.flow.upgrade",
            json!({"flow_key": "example", "expected_revision": "stale"}),
        ),
        case("battersea.flow.upgrade", json!({"flow_key": "example"})),
        case(
            "battersea.run.activate",
            json!({"flow_key": "example", "node_id": "text", "parameters": {"text": "Record it."}}),
        ),
        case(
            "battersea.run.activate",
            json!({"flow_key": "example", "node_id": "absent"}),
        ),
        case("battersea.run.status", json!({"run_id": PENDING_RUN})),
        case("battersea.run.status", json!({"run_id": "run-9"})),
        case("battersea.run.watch", json!({"run_id": FINISHED_RUN})),
        case("battersea.run.cancel", json!({"run_id": PENDING_RUN})),
        case("battersea.run.resume", json!({"run_id": PENDING_RUN})),
        case("battersea.approval.list", json!({"run_id": PENDING_RUN})),
        case("battersea.approval.decide", seeded_decision("allow").await),
        own_grant,
        agent_decision,
        case(
            "battersea.approval.reconcile",
            json!({
                "run_id": PENDING_RUN,
                "call_id": PENDING_CALL,
                "expected_revision": 2,
                "resolution": {"outcome": "not_applied"},
                "proof": "The fixture's write did not run."
            }),
        ),
        case("battersea.mcp.servers", json!({})),
        case("battersea.mcp.refresh", json!({"server_id": "remote"})),
        case("battersea.mcp.refresh", json!({"server_id": "absent"})),
    ]
}

#[tokio::test]
async fn every_operation_has_one_result_through_every_front_end() {
    let mut fixture = Fixture::new(ops_example::contributions, cases().await);
    fixture.denied_auth = OAuthContext::from_verified(
        ops_example::ISSUER,
        ops_example::AUDIENCE,
        "visitor",
        Vec::<String>::new(),
    );
    let process = Process::new(env!("CARGO_BIN_EXE_ops-example"));
    let mut front_ends: Vec<Box<dyn FrontEnd>> = vec![Box::new(InProcess)];
    for kind in [
        ProcessKind::CommandLine,
        ProcessKind::CommandLineStdio,
        ProcessKind::McpStdio,
        ProcessKind::McpHttp,
        ProcessKind::BoundStdio,
        ProcessKind::BoundHttp,
    ] {
        front_ends.push(Box::new(ProcessFrontEnd::new(kind, process.clone())));
    }
    let report = run(&fixture, Arc::new(ScopeGate), front_ends)
        .await
        .unwrap();
    assert_eq!(report.front_ends, 7);
    assert_eq!(report.calls, fixture.cases.len() * 7);
}
