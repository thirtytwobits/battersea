//! Generic validation reports each failure under its issue code, addressed to the node and
//! edge it concerns.
use battersea_flow::{
    catalog::Catalog,
    document::FLOW_DOCUMENT_VERSION,
    issue_codes as codes,
    registry::{ParameterType, Registry},
    validation::{validate_document, validate_parameter_value},
    FlowDocument, FlowExecutionLimits, FlowNodeDefinition, FlowParameterDataType,
    FlowParameterDefinition, FlowParameterEditorKind, FlowQueueLimits, FlowValidationIssue,
    FlowValidationResult,
};
use serde_json::{json, Value};
use std::collections::HashMap;

fn port(name: &str, kind: &str, token_type: &str) -> Value {
    json!({"name": name, "kind": kind, "token_type": token_type, "mode": "final_value", "phase": "execution"})
}

fn string_parameter(name: &str) -> Value {
    json!({"name": name, "datatype": {"kind": "string"}, "editor": {"kind": "string"}})
}

fn count_parameter(name: &str, editor: &str) -> Value {
    json!({"name": name, "datatype": {"kind": "int"}, "editor": {"kind": editor, "min": 0, "max": 3}})
}

/// Prompt, Join, Count and Meta are inline; Fanout passes its input type through `auto`
/// outputs; Sink has no outputs; Relay is a logic node with dynamic signal ports.
fn definitions() -> Vec<FlowNodeDefinition> {
    let definition = |class_name: &str, kind: &str, body: Value| {
        let mut value = json!({
            "class_name": class_name, "short_description": class_name, "long_description": class_name,
            "kind": kind, "handler_id": format!("test.{class_name}")
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    };
    let mut out = port("out", "output", "text");
    out["parameters"] = json!([string_parameter("format")]);
    vec![
        definition(
            "Prompt",
            "inline",
            json!({
                "parameters": [{"name": "title", "datatype": {"kind": "label"}, "editor": {"kind": "string"}}],
                "input_ports": [port("seed", "input", "text")],
                "output_ports": [out],
                "action_ports": [{"name": "clear"}]
            }),
        ),
        definition(
            "Join",
            "inline",
            json!({
                "parameters": [
                    count_parameter("inputs", "input_port_count"),
                    {"name": "ordering", "datatype": {"kind": "string"}, "editor": {"kind": "enum", "values": ["ascending", "descending"]}}
                ],
                "dynamic_input_ports": [{
                    "count_parameter": "inputs", "name_template": "input-{index}", "token_type": "text",
                    "accepted_token_types": ["text", "number"], "mode": "final_value", "phase": "execution",
                    "parameters": [string_parameter("delimiter")]
                }],
                "output_ports": [port("out", "output", "text")]
            }),
        ),
        definition(
            "Count",
            "inline",
            json!({"input_ports": [port("in", "input", "text")], "output_ports": [port("out", "output", "number")]}),
        ),
        definition(
            "Meta",
            "inline",
            json!({"input_ports": [port("in", "input", "meta")], "output_ports": [port("out", "output", "meta")]}),
        ),
        definition(
            "Fanout",
            "inline",
            json!({
                "parameters": [count_parameter("outputs", "output_port_count")],
                "input_ports": [{"name": "input", "kind": "input", "token_type": "oneof", "accepted_token_types": ["text", "number"], "mode": "final_value", "phase": "execution"}],
                "dynamic_output_ports": [{"count_parameter": "outputs", "name_template": "output-{index}", "token_type": "auto", "mode": "final_value", "phase": "execution"}]
            }),
        ),
        definition(
            "Sink",
            "sink",
            json!({"input_ports": [port("in", "input", "text")]}),
        ),
        definition(
            "Relay",
            "logic",
            json!({
                "parameters": [count_parameter("outputs", "output_port_count")],
                "action_ports": [{"name": "input"}],
                "dynamic_signal_ports": [{"count_parameter": "outputs", "name_template": "output-{index}"}]
            }),
        ),
    ]
}

fn registry(definitions: &[FlowNodeDefinition]) -> Registry {
    let mut registry = Registry::default();
    for token in ["text", "number", "meta"] {
        registry.register_token_type(token, json!(true)).unwrap();
    }
    registry
        .register_parameter_type(
            "label",
            ParameterType::new(
                json!({"type": "string"}),
                Some(FlowParameterEditorKind::String),
                |value| match value.as_str() {
                    Some(text) if !text.trim().is_empty() => Ok(()),
                    _ => Err("A label needs visible text".into()),
                },
            ),
        )
        .unwrap();
    for definition in definitions {
        registry.register_handler(&definition.handler_id).unwrap();
    }
    registry
}

fn catalog() -> Catalog {
    let definitions = definitions();
    let registry = registry(&definitions);
    Catalog::from_definitions(definitions, registry).unwrap()
}

fn node(id: &str, definition_name: &str) -> Value {
    json!({"id": id, "definition_name": definition_name, "instance_name": id})
}

fn node_with(id: &str, definition_name: &str, fields: Value) -> Value {
    let mut node = node(id, definition_name);
    node.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    node
}

fn join(id: &str, inputs: Value) -> Value {
    node_with(id, "Join", json!({"parameter_values": {"inputs": inputs}}))
}

fn fanout(id: &str) -> Value {
    node_with(id, "Fanout", json!({"parameter_values": {"outputs": 2}}))
}

fn relay(id: &str) -> Value {
    node_with(id, "Relay", json!({"parameter_values": {"outputs": 2}}))
}

fn edge(kind: &str, id: &str, source: (&str, &str), target: (&str, &str)) -> Value {
    json!({
        "id": id, "kind": kind,
        "source_node_id": source.0, "source_port": source.1,
        "target_node_id": target.0, "target_port": target.1
    })
}

fn token(id: &str, source: (&str, &str), target: (&str, &str)) -> Value {
    edge("token", id, source, target)
}

fn signal(id: &str, source: (&str, &str), target: (&str, &str)) -> Value {
    edge("signal", id, source, target)
}

fn flow(nodes: Value, edges: Value) -> FlowDocument {
    serde_json::from_value(json!({
        "version": FLOW_DOCUMENT_VERSION, "flow_key": "validation", "title": "Validation",
        "execution": {"source_order": [], "limits": FlowExecutionLimits::default()},
        "nodes": nodes, "edges": edges
    }))
    .unwrap()
}

fn validate(nodes: Value, edges: Value) -> FlowValidationResult {
    catalog().validate(&flow(nodes, edges))
}

/// Whether `result` reports `code` for exactly this node and edge; `None` means the issue
/// names no node or no edge.
fn reports(
    result: &FlowValidationResult,
    code: &str,
    node_id: Option<&str>,
    edge_id: Option<&str>,
) -> bool {
    !result.valid
        && result.issues.iter().any(|issue| {
            issue.code == code
                && issue.node_id.as_deref() == node_id
                && issue.edge_id.as_deref() == edge_id
        })
}

fn assert_reports(
    result: &FlowValidationResult,
    code: &str,
    node_id: Option<&str>,
    edge_id: Option<&str>,
) {
    assert!(
        reports(result, code, node_id, edge_id),
        "expected {code} for node {node_id:?} and edge {edge_id:?}; got {:?}",
        result.issues
    );
}

fn assert_valid(result: &FlowValidationResult) {
    assert!(result.valid, "{:?}", result.issues);
    assert!(result.issues.is_empty());
}

#[test]
fn an_unsupported_version_is_reported_without_reading_the_graph() {
    let mut document = flow(json!([node("", "Missing")]), json!([]));
    document.version = FLOW_DOCUMENT_VERSION + 1;

    let result = catalog().validate(&document);

    assert_reports(&result, codes::UNSUPPORTED_DOCUMENT_VERSION, None, None);
    assert_eq!(result.issues.len(), 1, "{:?}", result.issues);
}

#[test]
fn an_execution_contract_failure_is_reported_for_the_document() {
    let mut document = flow(
        json!([node("prompt", "Prompt")]),
        json!([signal(
            "signal",
            ("prompt", "post_activate"),
            ("prompt", "clear")
        )]),
    );
    assert_valid(&catalog().validate(&document));

    document.edges[0].queue = Some(FlowQueueLimits::default());

    assert_reports(
        &catalog().validate(&document),
        codes::INVALID_EXECUTION_CONTRACT,
        None,
        None,
    );
}

#[test]
fn node_identity_definition_and_parameter_failures_accumulate() {
    let result = validate(
        json!([
            {"id": "", "definition_name": "Prompt", "instance_name": "", "parameter_values": {"unknown": "value"}},
            {"id": "", "definition_name": "Prompt", "instance_name": "Second prompt"},
            node("missing-def", "MissingDefinition"),
            node("blank-def", "")
        ]),
        json!([token("", ("", "out"), ("missing-target", "seed"))]),
    );

    assert_reports(&result, codes::BLANK_NODE_ID, Some(""), None);
    assert_reports(&result, codes::DUPLICATE_NODE_ID, Some(""), None);
    assert_reports(&result, codes::BLANK_INSTANCE_NAME, Some(""), None);
    assert_reports(&result, codes::UNKNOWN_PARAMETER, Some(""), None);
    assert_reports(
        &result,
        codes::UNKNOWN_DEFINITION,
        Some("missing-def"),
        None,
    );
    assert_reports(
        &result,
        codes::BLANK_DEFINITION_NAME,
        Some("blank-def"),
        None,
    );
    assert_reports(&result, codes::BLANK_EDGE_ID, None, Some(""));
    assert_reports(&result, codes::UNKNOWN_TARGET_NODE, None, Some(""));
}

#[test]
fn edges_need_unique_ids_known_endpoints_and_known_ports() {
    let nodes = json!([node("prompt", "Prompt"), join("join", json!(2))]);

    let blank_and_duplicate = validate(
        nodes.clone(),
        json!([
            token("", ("prompt", "out"), ("join", "input-0")),
            token("", ("prompt", "out"), ("join", "input-1"))
        ]),
    );
    assert_reports(&blank_and_duplicate, codes::BLANK_EDGE_ID, None, Some(""));
    assert_reports(
        &blank_and_duplicate,
        codes::DUPLICATE_EDGE_ID,
        None,
        Some(""),
    );

    let unknown_nodes = validate(
        nodes.clone(),
        json!([
            token("missing-source", ("missing", "out"), ("prompt", "seed")),
            token("missing-target", ("prompt", "out"), ("missing", "in"))
        ]),
    );
    assert_reports(
        &unknown_nodes,
        codes::UNKNOWN_SOURCE_NODE,
        None,
        Some("missing-source"),
    );
    assert_reports(
        &unknown_nodes,
        codes::UNKNOWN_TARGET_NODE,
        None,
        Some("missing-target"),
    );

    let unknown_ports = validate(
        nodes,
        json!([
            token(
                "bad-source-port",
                ("prompt", "missing_output"),
                ("join", "input-0")
            ),
            token(
                "bad-target-port",
                ("prompt", "out"),
                ("join", "missing_input")
            )
        ]),
    );
    assert_reports(
        &unknown_ports,
        codes::UNKNOWN_SOURCE_PORT,
        Some("prompt"),
        Some("bad-source-port"),
    );
    assert_reports(
        &unknown_ports,
        codes::UNKNOWN_TARGET_PORT,
        Some("join"),
        Some("bad-target-port"),
    );
}

#[test]
fn token_types_must_match_and_targets_admit_their_declared_alternates() {
    let nodes = json!([
        node("count", "Count"),
        node("prompt", "Prompt"),
        join("join", json!(1))
    ]);

    let incompatible = validate(
        nodes.clone(),
        json!([token(
            "number-to-text",
            ("count", "out"),
            ("prompt", "seed")
        )]),
    );
    assert_reports(
        &incompatible,
        codes::INCOMPATIBLE_TOKEN_TYPES,
        None,
        Some("number-to-text"),
    );

    assert_valid(&validate(
        nodes,
        json!([token("alternate", ("count", "out"), ("join", "input-0"))]),
    ));
}

#[test]
fn an_auto_output_carries_only_its_node_input_types_whether_or_not_they_are_connected() {
    let edges = json!([
        token("e-bad", ("fanout", "output-0"), ("meta", "in")),
        token("e-ok", ("fanout", "output-1"), ("prompt", "seed"))
    ]);
    let unconnected = validate(
        json!([
            fanout("fanout"),
            node("meta", "Meta"),
            node("prompt", "Prompt")
        ]),
        edges.clone(),
    );
    let mut connected_edges = edges.as_array().unwrap().clone();
    connected_edges.push(token("e-in", ("upstream", "out"), ("fanout", "input")));
    let connected = validate(
        json!([
            node("upstream", "Prompt"),
            fanout("fanout"),
            node("meta", "Meta"),
            node("prompt", "Prompt")
        ]),
        Value::Array(connected_edges),
    );

    for result in [unconnected, connected] {
        assert_reports(
            &result,
            codes::INCOMPATIBLE_TOKEN_TYPES,
            None,
            Some("e-bad"),
        );
        assert!(
            !result.issues.iter().any(|issue| {
                issue.code == codes::INCOMPATIBLE_TOKEN_TYPES
                    && issue.edge_id.as_deref() == Some("e-ok")
            }),
            "{:?}",
            result.issues
        );
    }
}

#[test]
fn a_token_edge_runs_from_an_output_port_to_an_input_port() {
    let definition: FlowNodeDefinition = serde_json::from_value(json!({
        "class_name": "Backwards", "short_description": "Backwards", "long_description": "Backwards",
        "kind": "inline", "handler_id": "test.backwards",
        "input_ports": [port("in", "output", "text")],
        "output_ports": [port("out", "input", "text")]
    }))
    .unwrap();
    let definitions = HashMap::from([(definition.class_name.clone(), definition)]);
    let document = flow(
        json!([node("from", "Backwards"), node("to", "Backwards")]),
        json!([token("backwards", ("from", "out"), ("to", "in"))]),
    );

    let result = validate_document(&document, &definitions, &Registry::default());

    assert_reports(
        &result,
        codes::MISDIRECTED_TOKEN_EDGE,
        None,
        Some("backwards"),
    );
}

#[test]
fn outputs_fan_out_but_each_input_has_one_producer() {
    assert_valid(&validate(
        json!([
            node("prompt", "Prompt"),
            join("join-1", json!(1)),
            join("join-2", json!(1))
        ]),
        json!([
            token("token-1", ("prompt", "out"), ("join-1", "input-0")),
            token("token-2", ("prompt", "out"), ("join-2", "input-0"))
        ]),
    ));

    let result = validate(
        json!([
            node("prompt-1", "Prompt"),
            node("prompt-2", "Prompt"),
            join("join", json!(1))
        ]),
        json!([
            token("token-1", ("prompt-1", "out"), ("join", "input-0")),
            token("token-2", ("prompt-2", "out"), ("join", "input-0"))
        ]),
    );
    assert_reports(
        &result,
        codes::DUPLICATE_INPUT_EDGE,
        Some("join"),
        Some("token-2"),
    );
}

#[test]
fn token_cycles_are_rejected_and_signal_cycles_are_not() {
    let token_cycle = validate(
        json!([node("prompt", "Prompt")]),
        json!([token("cycle", ("prompt", "out"), ("prompt", "seed"))]),
    );
    assert_reports(&token_cycle, codes::GRAPH_CYCLE, None, None);

    assert_valid(&validate(
        json!([node("prompt", "Prompt")]),
        json!([signal(
            "signal",
            ("prompt", "post_activate"),
            ("prompt", "clear")
        )]),
    ));
}

#[test]
fn signal_edges_reach_declared_built_in_and_dynamic_ports() {
    assert_valid(&validate(
        json!([node("prompt", "Prompt")]),
        json!([signal(
            "declared",
            ("prompt", "post_activate"),
            ("prompt", "clear")
        )]),
    ));
    assert_valid(&validate(
        json!([node("prompt", "Prompt"), join("join", json!(1))]),
        json!([signal(
            "disable",
            ("prompt", "post_activate"),
            ("join", "disable")
        )]),
    ));
    assert_valid(&validate(
        json!([
            node("source", "Prompt"),
            relay("relay"),
            node("first", "Prompt"),
            node("second", "Prompt")
        ]),
        json!([
            signal("in", ("source", "post_activate"), ("relay", "input")),
            signal("out-0", ("relay", "output-0"), ("first", "clear")),
            signal("out-1", ("relay", "output-1"), ("second", "clear"))
        ]),
    ));
}

#[test]
fn only_output_capable_nodes_offer_the_disable_action() {
    let result = validate(
        json!([node("prompt", "Prompt"), node("sink", "Sink")]),
        json!([signal(
            "signal",
            ("prompt", "post_activate"),
            ("sink", "disable")
        )]),
    );

    assert_reports(
        &result,
        codes::UNKNOWN_ACTION_PORT,
        Some("sink"),
        Some("signal"),
    );
}

#[test]
fn signal_edges_name_known_ports_and_each_signal_or_action_port_has_one_edge() {
    let unknown_ports = validate(
        json!([node("prompt-1", "Prompt"), node("prompt-2", "Prompt")]),
        json!([
            signal(
                "bad-signal-port",
                ("prompt-1", "missing_signal"),
                ("prompt-2", "clear")
            ),
            signal(
                "bad-action-port",
                ("prompt-1", "post_activate"),
                ("prompt-2", "missing_action")
            )
        ]),
    );
    assert_reports(
        &unknown_ports,
        codes::UNKNOWN_SIGNAL_PORT,
        Some("prompt-1"),
        Some("bad-signal-port"),
    );
    assert_reports(
        &unknown_ports,
        codes::UNKNOWN_ACTION_PORT,
        Some("prompt-2"),
        Some("bad-action-port"),
    );

    let shared_signal = validate(
        json!([node("prompt-1", "Prompt"), node("prompt-2", "Prompt")]),
        json!([
            signal(
                "signal-1",
                ("prompt-1", "post_activate"),
                ("prompt-1", "clear")
            ),
            signal(
                "signal-2",
                ("prompt-1", "post_activate"),
                ("prompt-2", "clear")
            )
        ]),
    );
    assert_reports(
        &shared_signal,
        codes::DUPLICATE_SIGNAL_EDGE,
        Some("prompt-1"),
        Some("signal-2"),
    );

    let shared_action = validate(
        json!([
            node("prompt-1", "Prompt"),
            node("prompt-2", "Prompt"),
            node("prompt-3", "Prompt")
        ]),
        json!([
            signal(
                "signal-1",
                ("prompt-1", "post_activate"),
                ("prompt-3", "clear")
            ),
            signal(
                "signal-2",
                ("prompt-2", "post_activate"),
                ("prompt-3", "clear")
            )
        ]),
    );
    assert_reports(
        &shared_action,
        codes::DUPLICATE_ACTION_EDGE,
        Some("prompt-3"),
        Some("signal-2"),
    );
}

#[test]
fn edges_touching_an_unknown_definition_report_that_node() {
    let result = validate(
        json!([
            node("missing", "MissingDefinition"),
            node("prompt", "Prompt")
        ]),
        json!([
            signal(
                "from-missing",
                ("missing", "post_activate"),
                ("prompt", "clear")
            ),
            signal(
                "to-missing",
                ("prompt", "post_activate"),
                ("missing", "clear")
            ),
            token("token-from-missing", ("missing", "out"), ("prompt", "seed"))
        ]),
    );

    for edge in ["from-missing", "to-missing", "token-from-missing"] {
        assert_reports(
            &result,
            codes::UNKNOWN_DEFINITION,
            Some("missing"),
            Some(edge),
        );
    }
}

#[test]
fn dynamic_port_counts_must_be_present_integers_within_bounds() {
    for count in [json!("two"), json!(-1), json!(4), Value::Null] {
        let join_node = if count.is_null() {
            node("join", "Join")
        } else {
            join("join", count.clone())
        };
        let result = validate(
            json!([node("prompt", "Prompt"), join_node]),
            json!([token("edge", ("prompt", "out"), ("join", "input-0"))]),
        );
        assert_reports(&result, codes::INVALID_DYNAMIC_PORTS, Some("join"), None);
        assert_reports(
            &result,
            codes::INVALID_DYNAMIC_PORTS,
            Some("join"),
            Some("edge"),
        );
    }

    let result = validate(
        json!([node("relay", "Relay"), node("prompt", "Prompt")]),
        json!([signal("edge", ("relay", "output-0"), ("prompt", "clear"))]),
    );
    assert_reports(
        &result,
        codes::INVALID_DYNAMIC_PORTS,
        Some("relay"),
        Some("edge"),
    );
}

#[test]
fn port_parameter_values_name_known_ports_and_parameters_with_valid_values() {
    let prompt = |values: Value| {
        node_with(
            "prompt",
            "Prompt",
            json!({"port_parameter_values": {"output": values}}),
        )
    };
    assert_valid(&validate(
        json!([prompt(json!({"out": {"format": "plain"}}))]),
        json!([]),
    ));
    assert_valid(&validate(
        json!([node_with(
            "join",
            "Join",
            json!({
                "parameter_values": {"inputs": 3},
                "port_parameter_values": {"input": {"input-2": {"delimiter": "---"}}}
            })
        )]),
        json!([]),
    ));

    for (values, code) in [
        (
            json!({"missing": {"format": "plain"}}),
            codes::PORT_PARAMETERS_FOR_UNKNOWN_PORT,
        ),
        (
            json!({"out": {"missing": "plain"}}),
            codes::UNKNOWN_PORT_PARAMETER,
        ),
        (
            json!({"out": {"format": false}}),
            codes::INVALID_PORT_PARAMETER_VALUE,
        ),
    ] {
        assert_reports(
            &validate(json!([prompt(values)]), json!([])),
            code,
            Some("prompt"),
            None,
        );
    }
}

#[test]
fn parameter_values_must_satisfy_their_editor_and_host_datatype() {
    assert_valid(&validate(
        json!([
            node_with(
                "prompt",
                "Prompt",
                json!({"parameter_values": {"title": "Opening"}})
            ),
            node_with(
                "join",
                "Join",
                json!({"parameter_values": {"inputs": 1, "ordering": "ascending"}})
            )
        ]),
        json!([]),
    ));

    let result = validate(
        json!([
            node_with(
                "prompt",
                "Prompt",
                json!({"parameter_values": {"title": "   "}})
            ),
            node_with(
                "join",
                "Join",
                json!({"parameter_values": {"inputs": 1, "ordering": "sideways"}})
            )
        ]),
        json!([]),
    );

    assert_reports(
        &result,
        codes::INVALID_PARAMETER_VALUE,
        Some("prompt"),
        None,
    );
    assert_reports(&result, codes::INVALID_PARAMETER_VALUE, Some("join"), None);
}

fn parameter(datatype: FlowParameterDataType, editor: Value) -> FlowParameterDefinition {
    serde_json::from_value(json!({"name": "value", "datatype": datatype, "editor": editor}))
        .unwrap()
}

#[test]
fn editor_constraints_bound_parameter_values() {
    let registry = Registry::default();
    let check = |parameter: &FlowParameterDefinition, accepted: &[Value], rejected: &[Value]| {
        for value in accepted {
            assert_eq!(
                validate_parameter_value(parameter, value, &registry),
                None,
                "{value} must be accepted by {parameter:?}"
            );
        }
        for value in rejected {
            assert!(
                validate_parameter_value(parameter, value, &registry).is_some(),
                "{value} must be rejected by {parameter:?}"
            );
        }
    };
    let int = FlowParameterDataType::named("int");
    let string = FlowParameterDataType::named("string");

    check(
        &parameter(int.clone(), json!({"kind": "unsigned", "min": 1, "max": 3})),
        &[json!(1), json!(3)],
        &[json!(1.5), json!(-1), json!(0), json!(4)],
    );
    check(
        &parameter(
            string.clone(),
            json!({"kind": "enum", "values": ["ascending", "descending"]}),
        ),
        &[json!("ascending")],
        &[json!(true), json!("sideways")],
    );
    check(
        &parameter(
            FlowParameterDataType::list(string.clone()),
            json!({"kind": "list", "max": 1}),
        ),
        &[json!(["a"])],
        &[json!("nope"), json!(["a", "b"])],
    );
    check(
        &parameter(
            FlowParameterDataType::list(string.clone()),
            json!({"kind": "list", "values": ["a"]}),
        ),
        &[json!(["a"])],
        &[json!(["b"])],
    );
    check(
        &parameter(
            FlowParameterDataType::list(int),
            json!({"kind": "list", "values": ["1"]}),
        ),
        &[],
        &[json!([1])],
    );
    check(
        &parameter(
            FlowParameterDataType::named("boolean"),
            json!({"kind": "boolean"}),
        ),
        &[json!(true)],
        &[json!("true")],
    );
    for editor in ["string", "text_input"] {
        check(
            &parameter(string.clone(), json!({"kind": editor})),
            &[json!("hello")],
            &[json!(true)],
        );
    }
}

#[test]
fn values_match_their_datatypes_strictly() {
    let definitions = definitions();
    let registry = registry(&definitions);
    let check = |datatype: FlowParameterDataType, accepted: Value, rejected: Value| {
        assert!(registry.validate_value(&datatype, &accepted).is_ok());
        assert!(registry.validate_value(&datatype, &rejected).is_err());
    };
    let string = FlowParameterDataType::named("string");

    check(
        FlowParameterDataType::named("boolean"),
        json!(true),
        json!("true"),
    );
    check(FlowParameterDataType::named("int"), json!(3), json!(3.5));
    check(string.clone(), json!("hello"), json!(true));
    check(
        FlowParameterDataType::named("label"),
        json!("Opening"),
        json!(" "),
    );
    check(
        FlowParameterDataType::list(string.clone()),
        json!(["a", "b"]),
        json!(["a", 2]),
    );
    check(
        FlowParameterDataType::map(string),
        json!({"first": "a", "second": "b"}),
        json!({"first": "a", "second": 2}),
    );
}

#[test]
fn port_aliases_name_known_ports_and_are_distinct_and_visible() {
    let aliased = |names: Value| {
        node_with(
            "join",
            "Join",
            json!({"parameter_values": {"inputs": 2}, "port_names": names}),
        )
    };
    assert_valid(&validate(
        json!([aliased(json!({
            "input": {"input-0": "Prompt", "input-1": "Context"},
            "output": {"out": "Combined"}
        }))]),
        json!([]),
    ));

    let unknown = validate(
        json!([aliased(
            json!({"input": {"input-0": "Prompt", "missing": "Ghost"}})
        )]),
        json!([]),
    );
    assert_reports(&unknown, codes::ALIAS_FOR_UNKNOWN_PORT, Some("join"), None);

    let duplicate_and_blank = validate(
        json!([aliased(json!({
            "input": {"input-0": "Prompt", "input-1": " Prompt "},
            "output": {"out": "   "}
        }))]),
        json!([]),
    );
    assert_reports(
        &duplicate_and_blank,
        codes::DUPLICATE_PORT_ALIAS,
        Some("join"),
        None,
    );
    assert_reports(
        &duplicate_and_blank,
        codes::BLANK_PORT_ALIAS,
        Some("join"),
        None,
    );

    let both_sides = validate(
        json!([aliased(json!({
            "input": {"missing": "Ghost"},
            "output": {"out": "   "}
        }))]),
        json!([]),
    );
    assert_reports(
        &both_sides,
        codes::ALIAS_FOR_UNKNOWN_PORT,
        Some("join"),
        None,
    );
    assert_reports(&both_sides, codes::BLANK_PORT_ALIAS, Some("join"), None);
}

#[test]
fn a_canonical_port_id_names_one_side_of_a_node() {
    let definition: FlowNodeDefinition = serde_json::from_value(json!({
        "class_name": "Colliding", "short_description": "Colliding", "long_description": "Colliding",
        "kind": "inline", "handler_id": "test.colliding",
        "input_ports": [port("shared", "input", "text")],
        "action_ports": [{"name": "shared"}],
        "signal_ports": [{"name": "pulse"}]
    }))
    .unwrap();
    let definitions = HashMap::from([(definition.class_name.clone(), definition)]);

    let result = validate_document(
        &flow(json!([node("colliding", "Colliding")]), json!([])),
        &definitions,
        &Registry::default(),
    );

    assert_reports(&result, codes::DUPLICATE_PORT_ID, Some("colliding"), None);
}

#[test]
fn host_graph_rules_report_their_own_codes_beside_the_generic_ones() {
    const HOST_CODE: &str = "host_single_prompt";
    let definitions = definitions();
    let mut registry = registry(&definitions);
    registry.register_graph_validator(|flow, _| {
        flow.nodes
            .iter()
            .filter(|node| node.definition_name == "Prompt")
            .skip(1)
            .map(|node| {
                FlowValidationIssue::new(HOST_CODE, "One prompt per flow.").with_node_id(&node.id)
            })
            .collect()
    });
    assert!(!codes::ALL.contains(&HOST_CODE));
    let catalog = Catalog::from_definitions(definitions, registry).unwrap();

    assert_valid(&catalog.validate(&flow(json!([node("first", "Prompt")]), json!([]))));

    let result = catalog.validate(&flow(
        json!([node("first", "Prompt"), node("second", "Prompt")]),
        json!([token("cycle", ("second", "out"), ("second", "seed"))]),
    ));
    assert_reports(&result, HOST_CODE, Some("second"), None);
    assert_reports(&result, codes::GRAPH_CYCLE, None, None);
}

#[test]
fn every_reported_code_is_declared() {
    let results = [
        validate(
            json!([node("", "Missing"), node("", "")]),
            json!([token("", ("x", "y"), ("", "z"))]),
        ),
        validate(
            json!([node("prompt", "Prompt"), node("count", "Count")]),
            json!([token("e", ("count", "out"), ("prompt", "seed"))]),
        ),
    ];
    for issue in results.iter().flat_map(|result| &result.issues) {
        assert!(codes::ALL.contains(&issue.code.as_str()), "{issue:?}");
    }
}
