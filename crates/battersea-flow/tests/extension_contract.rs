use battersea_flow::{
    catalog::Catalog,
    registry::{ParameterType, Registry},
    FlowDocument, FlowParameterEditorKind,
};
use serde_json::{json, Value};

fn registry() -> Registry {
    let mut registry = Registry::default();
    registry.register_handler("example.echo").unwrap();
    registry
        .register_token_type("example.message", json!({"type": "string"}))
        .unwrap();
    registry
        .register_parameter_type(
            "example.label",
            ParameterType::new(
                json!({"type": "string", "minLength": 1}),
                Some(FlowParameterEditorKind::String),
                |value| {
                    value
                        .as_str()
                        .filter(|text| !text.is_empty())
                        .map(|_| ())
                        .ok_or_else(|| "A label is required".into())
                },
            ),
        )
        .unwrap();
    registry
}

fn manifest() -> String {
    json!({"node_definitions": [{
        "class_name": "Echo", "short_description": "Echo", "long_description": "Echo",
        "kind": "source", "handler_id": "example.echo",
        "parameters": [{"name": "label", "datatype": {"kind": "example.label"}, "editor": {}}],
        "output_ports": [{"name": "out", "kind": "output", "token_type": "example.message","mode":"final_value","phase":"snapshot"}]
    }]})
    .to_string()
}

#[test]
fn a_host_registered_type_validates_values_without_product_dependencies() {
    let catalog = Catalog::from_manifest(&manifest(), registry()).unwrap();
    let mut flow: FlowDocument = serde_json::from_value(json!({
        "version": 2, "execution": battersea_flow::FlowExecutionPolicy { source_order: vec!["source".into()], limits: battersea_flow::FlowExecutionLimits::default() }, "flow_key": "example", "title": "Example", "nodes": [{
            "id": "source", "definition_name": "Echo", "instance_name": "Source", "parameter_values": {"label": "authored label"}
        }], "layout": {"independent_editor_v1": {"viewport": [4, 7, 1.5]}},
        "metadata": {"owner": {"colour": "violet"}}
    })).unwrap();
    assert!(catalog.validate(&flow).valid);
    let serialized = serde_json::to_value(&flow).unwrap();
    let round_trip: FlowDocument = serde_json::from_value(serialized).unwrap();
    assert_eq!(round_trip.layout, flow.layout);
    assert_eq!(round_trip.metadata, flow.metadata);
    flow.nodes[0]
        .parameter_values
        .insert("label".into(), Value::Bool(false));
    assert!(!catalog.validate(&flow).valid);
}

#[test]
fn invalid_external_catalogues_return_errors_without_panicking() {
    for change in [
        "unknown_handler",
        "unknown_type",
        "duplicate_class",
        "invalid_default",
    ] {
        let mut value: Value = serde_json::from_str(&manifest()).unwrap();
        match change {
            "unknown_handler" => value["node_definitions"][0]["handler_id"] = json!("missing"),
            "unknown_type" => {
                value["node_definitions"][0]["parameters"][0]["datatype"]["kind"] = json!("missing")
            }
            "duplicate_class" => {
                let node = value["node_definitions"][0].clone();
                value["node_definitions"].as_array_mut().unwrap().push(node);
            }
            "invalid_default" => {
                value["node_definitions"][0]["parameters"][0]["editor"]["default_value"] =
                    json!(false)
            }
            _ => unreachable!(),
        }
        assert!(
            Catalog::from_manifest(&value.to_string(), registry()).is_err(),
            "{change}"
        );
    }
}

#[test]
fn dynamic_expansion_refuses_out_of_range_counts_before_allocating_ports() {
    use battersea_flow::{
        ports::{expanded_input_ports_for_node, expanded_signal_ports_for_node},
        FlowNode, FlowNodeDefinition,
    };
    let definition: FlowNodeDefinition = serde_json::from_value(json!({
        "class_name": "Dynamic", "kind": "logic", "handler_id": "example.dynamic", "short_description": "Dynamic", "long_description": "Dynamic",
        "parameters": [{"name": "count", "datatype": {"kind": "int"}, "editor": {"kind": "input_port_count", "min": 0, "max": 3}}],
        "dynamic_input_ports": [{"count_parameter": "count", "name_template": "input-{index}", "token_type": "example.message", "mode":"final_value", "phase":"execution"}],
        "dynamic_signal_ports": [{"count_parameter": "count", "name_template": "signal-{index}"}]
    })).unwrap();
    let definitions =
        std::collections::HashMap::from([(definition.class_name.clone(), definition)]);
    let node: FlowNode = serde_json::from_value(json!({"id": "dynamic", "definition_name": "Dynamic", "instance_name": "Dynamic", "parameter_values": {"count": i64::MAX}})).unwrap();
    assert!(expanded_input_ports_for_node(&definitions, &node).is_err());
    assert!(expanded_signal_ports_for_node(&definitions, &node).is_err());
}

#[test]
fn catalogue_order_follows_the_authored_manifest_and_defaults_obey_their_controller() {
    let mut source: Value = serde_json::from_str(&manifest()).unwrap();
    let mut other = source["node_definitions"][0].clone();
    other["class_name"] = json!("AnotherClass");
    source["node_definitions"]
        .as_array_mut()
        .unwrap()
        .push(other);
    let order = source["node_definitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["class_name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let catalog = Catalog::from_manifest(&source.to_string(), registry()).unwrap();
    assert_eq!(
        catalog
            .entries()
            .map(|node| node.class_name.clone())
            .collect::<Vec<_>>(),
        order
    );
    source["node_definitions"][0]["parameters"][0]["controller"] =
        json!({"kind": "string", "default_value": false});
    assert!(Catalog::from_manifest(&source.to_string(), registry()).is_err());
}

#[test]
fn an_external_graph_enforces_ports_cardinality_nominal_types_and_acyclicity() {
    let mut source: Value = serde_json::from_str(&manifest()).unwrap();
    source["node_definitions"][0]["kind"] = json!("inline");
    source["node_definitions"][0]["output_ports"][0]["phase"] = json!("execution");
    source["node_definitions"][0]["input_ports"] = json!([{"name": "in", "kind": "input", "token_type": "example.message","mode":"final_value","phase":"execution"}]);
    let catalog = Catalog::from_manifest(&source.to_string(), registry()).unwrap();
    let base = json!({"version": 2, "execution": battersea_flow::FlowExecutionPolicy { source_order: vec![], limits: battersea_flow::FlowExecutionLimits::default() }, "flow_key": "graph", "title": "Graph", "nodes": [
        {"id": "left", "definition_name": "Echo", "instance_name": "Left"},
        {"id": "right", "definition_name": "Echo", "instance_name": "Right"}
    ], "edges": [{"id": "forward", "source_node_id": "left", "source_port": "out", "target_node_id": "right", "target_port": "in"}]});
    let flow: FlowDocument = serde_json::from_value(base.clone()).unwrap();
    assert!(catalog.validate(&flow).valid);
    for invalid in [
        "cycle",
        "duplicate_output",
        "missing_node",
        "missing_port",
        "unknown_definition",
    ] {
        let mut document = base.clone();
        match invalid {
            "cycle" => document["edges"].as_array_mut().unwrap().push(json!({"id": "back", "source_node_id": "right", "source_port": "out", "target_node_id": "left", "target_port": "in"})),
            "duplicate_output" => {
                let mut edge = document["edges"][0].clone();
                edge["id"] = json!("duplicate");
                document["edges"].as_array_mut().unwrap().push(edge);
            },
            "missing_node" => document["edges"][0]["target_node_id"] = json!("missing"),
            "missing_port" => document["edges"][0]["target_port"] = json!("missing"),
            "unknown_definition" => document["nodes"][0]["definition_name"] = json!("missing"),
            _ => unreachable!(),
        }
        assert!(
            !catalog
                .validate(&serde_json::from_value(document).unwrap())
                .valid,
            "{invalid}"
        );
    }
    let mut registry = registry();
    registry
        .register_token_type("example.other", json!({"type": "boolean"}))
        .unwrap();
    source["node_definitions"][0]["input_ports"][0]["token_type"] = json!("example.other");
    let catalog = Catalog::from_manifest(&source.to_string(), registry).unwrap();
    assert!(!catalog.validate(&flow).valid);
    assert!(catalog
        .registry()
        .validate_token("example.other", &json!("invalid payload"))
        .is_err());
}
