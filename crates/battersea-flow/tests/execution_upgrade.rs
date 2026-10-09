use battersea_flow::{
    document::{inspect_document, load_document, serialize_canonical_document, DocumentInspection},
    execution::{ordered_sources, upgrade_v1_document, validate_execution_contract},
    FlowDocument, FlowExecutionLimits, FlowNodeDefinition,
};
use serde_json::{json, Value};
use std::collections::HashMap;

fn definitions() -> HashMap<String, FlowNodeDefinition> {
    [
        json!({"class_name":"Source","short_description":"Source","long_description":"Source","kind":"source","handler_id":"test.source",
            "output_ports":[{"name":"out","kind":"output","token_type":"text","mode":"final_value","phase":"snapshot"}]}),
        json!({"class_name":"Hybrid","short_description":"Hybrid","long_description":"Hybrid","kind":"hybrid","handler_id":"test.hybrid",
            "input_ports":[{"name":"budget","kind":"input","token_type":"text","display_class":"source","mode":"final_value","phase":"snapshot"}],
            "output_ports":[{"name":"out","kind":"output","token_type":"text","mode":"final_value","phase":"snapshot"}]}),
        json!({"class_name":"Stream","short_description":"Stream","long_description":"Stream","kind":"inline","handler_id":"test.stream",
            "output_ports":[{"name":"out","kind":"output","token_type":"text","mode":"stream","phase":"execution"}]}),
        json!({"class_name":"Sink","short_description":"Sink","long_description":"Sink","kind":"sink","handler_id":"test.sink",
            "input_ports":[{"name":"in","kind":"input","token_type":"text","mode":"stream","phase":"execution"}]}),
    ].into_iter().map(|v| {
        let definition: FlowNodeDefinition = serde_json::from_value(v).unwrap();
        (definition.class_name.clone(), definition)
    }).collect()
}

fn node(id: &str, definition: &str) -> Value {
    json!({"id":id,"definition_name":definition,"instance_name":format!("Label for {id}"),"parameter_values":{"authored":{"nested":[false,23]}}})
}

fn old_flow() -> Value {
    json!({"version":1,"flow_key":"upgrade-fixture","title":"Upgrade fixture",
        "nodes":[node("hybrid","Hybrid"),node("z-source","Source"),node("a-source","Source"),node("stream","Stream"),node("sink","Sink")],
        "edges":[
            {"id":"dependency","source_node_id":"a-source","source_port":"out","target_node_id":"hybrid","target_port":"budget","order":7},
            {"id":"stream-edge","source_node_id":"stream","source_port":"out","target_node_id":"sink","target_port":"in","order":3}
        ],"layout":{"editor":{"positions":[13,27],"future":["opaque"]}},"metadata":{"extension":{"keep":true}}
    })
}

#[test]
fn old_version_is_retained_without_interpreting_executable_fields() {
    let mut raw = old_flow();
    raw["nodes"] = json!({"not":"a supported node array"});
    let source = raw.to_string();
    let DocumentInspection::Incompatible { raw: retained, .. } = inspect_document(&source).unwrap()
    else {
        panic!("old execution semantics require explicit upgrade");
    };
    assert_eq!(retained, raw);
    assert!(load_document(&source).is_err());
    assert_eq!(source, raw.to_string());
}

#[test]
fn explicit_upgrade_preserves_authored_content_and_captures_source_order() {
    let before = old_flow();
    let source = before.to_string();
    let definitions = definitions();
    let upgraded =
        upgrade_v1_document(&source, &definitions, FlowExecutionLimits::default()).unwrap();
    validate_execution_contract(&upgraded, &definitions).unwrap();
    let after = serde_json::to_value(&upgraded).unwrap();
    for field in ["nodes", "layout", "metadata", "title", "flow_key"] {
        assert_eq!(after[field], before[field], "preserve {field}");
    }
    for (old, new) in before["edges"]
        .as_array()
        .unwrap()
        .iter()
        .zip(after["edges"].as_array().unwrap())
    {
        for (key, value) in old.as_object().unwrap() {
            assert_eq!(&new[key], value, "preserve edge {key}");
        }
    }
    assert!(after["edges"][1]["queue"].is_object());
    assert_eq!(source, before.to_string());
    let order = ordered_sources(&upgraded, &definitions).unwrap();
    let index = |id: &str| order.iter().position(|v| v == id).unwrap();
    assert!(
        index("a-source") < index("hybrid"),
        "source-phase dependency wins"
    );
    assert!(
        index("z-source") < index("a-source"),
        "independent source priority is preserved"
    );
    let mut shuffled = upgraded.clone();
    shuffled.nodes.reverse();
    shuffled.edges.reverse();
    for node in &mut shuffled.nodes {
        node.instance_name = "Different title".into();
    }
    assert_eq!(ordered_sources(&shuffled, &definitions).unwrap(), order);
    let round_trip = load_document(&serialize_canonical_document(&upgraded).unwrap()).unwrap();
    assert_eq!(ordered_sources(&round_trip, &definitions).unwrap(), order);
}

#[test]
fn supported_version_requires_an_explicit_execution_policy() {
    let mut raw = old_flow();
    raw["version"] = json!(battersea_flow::document::FLOW_DOCUMENT_VERSION);
    assert!(load_document(&raw.to_string()).is_err());
}

#[test]
fn source_order_must_list_every_source_exactly_once_and_reject_cycles() {
    let definitions = definitions();
    let upgraded = upgrade_v1_document(
        &old_flow().to_string(),
        &definitions,
        FlowExecutionLimits::default(),
    )
    .unwrap();
    for order in [
        vec![],
        vec!["a-source", "a-source", "hybrid"],
        vec!["a-source", "z-source", "missing"],
    ] {
        let mut flow = upgraded.clone();
        flow.execution.source_order = order.into_iter().map(String::from).collect();
        assert!(ordered_sources(&flow, &definitions).is_err());
    }
    let mut cyclic = old_flow();
    cyclic["nodes"][1] = node("z-source", "Hybrid");
    cyclic["edges"].as_array_mut().unwrap().extend([
        json!({"id":"cycle-a","source_node_id":"hybrid","source_port":"out","target_node_id":"z-source","target_port":"budget"}),
        json!({"id":"cycle-b","source_node_id":"z-source","source_port":"out","target_node_id":"hybrid","target_port":"budget"}),
    ]);
    assert!(upgrade_v1_document(
        &cyclic.to_string(),
        &definitions,
        FlowExecutionLimits::default()
    )
    .is_err());
}

#[test]
fn upgrade_refuses_ambiguous_nodes_versions_and_lossy_final_value_delivery() {
    let definitions = definitions();
    for mutation in [
        "unknown_node",
        "future_version",
        "already_upgraded",
        "injected_policy",
    ] {
        let mut raw = old_flow();
        match mutation {
            "unknown_node" => raw["nodes"][0]["definition_name"] = json!("Unknown"),
            "future_version" => raw["version"] = json!(99),
            "already_upgraded" => {
                raw["version"] = json!(battersea_flow::document::FLOW_DOCUMENT_VERSION)
            }
            "injected_policy" => raw["execution"] = json!({"source_order":[]}),
            _ => unreachable!(),
        }
        assert!(
            upgrade_v1_document(
                &raw.to_string(),
                &definitions,
                FlowExecutionLimits::default()
            )
            .is_err(),
            "{mutation}"
        );
    }
    let mut flow = upgrade_v1_document(
        &old_flow().to_string(),
        &definitions,
        FlowExecutionLimits::default(),
    )
    .unwrap();
    flow.edges[0].queue = flow.edges[1].queue.clone();
    assert!(validate_execution_contract(&flow, &definitions).is_err());
}

#[test]
fn invalid_capacities_and_stream_final_mismatch_fail_validation() {
    let definitions = definitions();
    let flow = upgrade_v1_document(
        &old_flow().to_string(),
        &definitions,
        FlowExecutionLimits::default(),
    )
    .unwrap();
    for change in [
        "zero_items",
        "zero_bytes",
        "event_too_large",
        "no_queue",
        "wrong_mode",
    ] {
        let mut raw = serde_json::to_value(&flow).unwrap();
        match change {
            "zero_items" => raw["edges"][1]["queue"]["items"] = json!(0),
            "zero_bytes" => raw["execution"]["limits"]["retained_bytes"] = json!(0),
            "event_too_large" => raw["edges"][1]["queue"]["max_event_bytes"] = json!(u32::MAX),
            "no_queue" => {
                raw["edges"][1].as_object_mut().unwrap().remove("queue");
            }
            "wrong_mode" => raw["edges"][1]["source_node_id"] = json!("z-source"),
            _ => unreachable!(),
        }
        let invalid: FlowDocument = serde_json::from_value(raw).unwrap();
        assert!(
            validate_execution_contract(&invalid, &definitions).is_err(),
            "{change}"
        );
    }
}

#[test]
fn typed_documents_cannot_bypass_version_inspection() {
    let mut flow = upgrade_v1_document(
        &old_flow().to_string(),
        &definitions(),
        FlowExecutionLimits::default(),
    )
    .unwrap();
    flow.version = 1;
    let inspected = DocumentInspection::from(flow.clone());
    assert!(matches!(inspected, DocumentInspection::Incompatible { .. }));
    assert!(inspected.supported().is_err());
    assert!(DocumentInspection::Supported(Box::new(flow))
        .supported()
        .is_err());
}

#[test]
fn catalogue_accepts_token_fanout_but_requires_one_producer_per_input() {
    let definitions = definitions();
    let mut entries = vec![definitions["Stream"].clone(), definitions["Sink"].clone()];
    entries[0].kind = battersea_flow::FlowNodeClass::Source;
    let mut registry = battersea_flow::registry::Registry::default();
    registry.register_token_type("text", json!(true)).unwrap();
    for definition in &entries {
        registry.register_handler(&definition.handler_id).unwrap();
    }
    let catalogue = battersea_flow::catalog::Catalog::from_definitions(entries, registry).unwrap();
    let mut flow: FlowDocument = serde_json::from_value(json!({
        "version":battersea_flow::document::FLOW_DOCUMENT_VERSION,"flow_key":"fanout","title":"Fan-out",
        "execution":{"source_order":["source"],"limits":FlowExecutionLimits::default()},
        "nodes":[{"id":"source","definition_name":"Stream","instance_name":"Source"},{"id":"a","definition_name":"Sink","instance_name":"A"},{"id":"b","definition_name":"Sink","instance_name":"B"}],
        "edges":[{"id":"a","source_node_id":"source","source_port":"out","target_node_id":"a","target_port":"in","queue":battersea_flow::FlowQueueLimits::default()},
            {"id":"b","source_node_id":"source","source_port":"out","target_node_id":"b","target_port":"in","queue":battersea_flow::FlowQueueLimits::default()}]
    })).unwrap();
    assert!(catalogue.validate(&flow).valid);
    flow.edges[1].target_node_id = flow.edges[0].target_node_id.clone();
    assert!(!catalogue.validate(&flow).valid);
}
