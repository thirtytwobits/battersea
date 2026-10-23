//! Authored failure policy and explicit upgrade boundaries.
use battersea_flow::{document::*, execution::*, failure::*, registry::Registry, *};
use serde_json::{json, Value};
use std::collections::HashMap;

fn definition() -> FlowNodeDefinition {
    serde_json::from_value(json!({"class_name":"Node","handler_id":"node","kind":"inline",
        "short_description":"node","long_description":"node",
        "failure_ports":[{"name":"error","phase":"execution"}],
        "input_ports":[{"name":"in","kind":"input","token_type":"node_failure","phase":"execution","mode":"final_value"}]})).unwrap()
}
fn document(policy: Value) -> FlowDocument {
    serde_json::from_value(json!({"version":3,"flow_key":"failure","title":"Failure",
        "execution":{"source_order":[],"limits":FlowExecutionLimits::default()},
        "nodes":[{"id":"node","instance_name":"Node","definition_name":"Node","failure_policy":policy}]})).unwrap()
}

#[test]
fn policies_validate_typed_failure_ports_before_execution() {
    let mut definitions = HashMap::from([("Node".into(), definition())]);
    for policy in ["fail", "emit", "skip"] {
        assert!(validate_execution_contract(&document(json!(policy)), &definitions).is_ok());
    }
    definitions.get_mut("Node").unwrap().failure_ports.clear();
    assert!(validate_execution_contract(&document(json!("emit")), &definitions).is_err());
    assert!(validate_execution_contract(&document(json!("skip")), &definitions).is_ok());
    for invalid in [json!("retry"), json!(null), json!({}), json!(1)] {
        let mut raw = serde_json::to_value(document(json!("fail"))).unwrap();
        raw["nodes"][0]["failure_policy"] = invalid;
        assert!(serde_json::from_value::<FlowDocument>(raw).is_err());
    }
}

#[test]
fn failure_port_names_phases_and_wiring_are_validated() {
    for invalid in [
        json!([{"name":"","phase":"execution"}]),
        json!([{"name":"a","phase":"execution"},{"name":"b","phase":"execution"}]),
        json!([{"name":"error","phase":"snapshot"}]),
    ] {
        let mut raw = serde_json::to_value(definition()).unwrap();
        raw["failure_ports"] = invalid;
        let def: FlowNodeDefinition = serde_json::from_value(raw).unwrap();
        assert!(validate_failure_ports(&def).is_err());
    }
    let mut flow = document(json!("emit"));
    let mut target = flow.nodes[0].clone();
    target.id = "target".into();
    flow.nodes.push(target);
    flow.edges = serde_json::from_value(
        json!([{"id":"error","source_node_id":"node","source_port":"error",
        "target_node_id":"target","target_port":"in"}]),
    )
    .unwrap();
    let mut definitions = HashMap::from([("Node".into(), definition())]);
    assert!(validate_execution_contract(&flow, &definitions).is_ok());
    definitions.get_mut("Node").unwrap().input_ports[0].token_type = "text".into();
    assert!(validate_execution_contract(&flow, &definitions).is_err());
    definitions
        .get_mut("Node")
        .unwrap()
        .output_ports
        .push(definition().failure_ports[0].as_port());
    assert!(validate_failure_ports(&definitions["Node"]).is_err());
}

#[test]
fn explicit_v2_upgrade_preserves_bytes_until_invoked_and_defaults_to_fail() {
    let definitions = HashMap::from([("Node".into(), definition())]);
    let mut raw = serde_json::to_value(document(json!("skip"))).unwrap();
    raw["version"] = json!(2);
    raw["nodes"][0]
        .as_object_mut()
        .unwrap()
        .remove("failure_policy");
    raw["metadata"] = json!({"opaque":[1,"雪"]});
    raw["layout"] = json!({"host":{"x":12}});
    let source = serde_json::to_string_pretty(&raw).unwrap();
    let before = source.clone();
    assert!(matches!(
        inspect_document(&source).unwrap(),
        DocumentInspection::Incompatible { version: 2, .. }
    ));
    assert!(load_document(&source).is_err());
    let upgraded = upgrade_v2_document(&source, &definitions).unwrap();
    assert_eq!(source, before);
    assert_eq!(upgraded.nodes[0].failure_policy, FlowFailurePolicy::Fail);
    raw["version"] = json!(3);
    assert_eq!(serde_json::to_value(upgraded).unwrap(), raw);
    let mut smuggled: Value = serde_json::from_str(&source).unwrap();
    smuggled["nodes"][0]["failure_policy"] = json!("emit");
    assert!(upgrade_v2_document(&smuggled.to_string(), &definitions).is_err());
}

#[test]
fn policy_and_failure_schema_survive_json_and_text_round_trips() {
    let flow = document(json!("emit"));
    assert_eq!(
        load_document(&serialize_canonical_document(&flow).unwrap()).unwrap(),
        flow
    );
    assert_eq!(
        dsl::parse_flow_dsl(&dsl::export_flow_dsl(&flow).unwrap()).unwrap(),
        flow
    );
    let registry = Registry::default();
    assert!(registry
        .validate_token(
            NODE_FAILURE_TOKEN_TYPE,
            &json!({"id":"failure","node_id":"node",
        "phase":"execution","operation":"execute_node","code":"declined","message":"Declined"})
        )
        .is_ok());
    assert!(registry
        .validate_token(NODE_FAILURE_TOKEN_TYPE, &json!({"message":"untyped"}))
        .is_err());
}
