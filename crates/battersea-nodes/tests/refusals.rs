//! Generic nodes name each refusal with a runtime error code.
use battersea_flow::{FlowNode, FlowNodeDefinition};
use battersea_nodes::{logic::prepare_logic_outputs, text::activation_text};
use battersea_runtime::error_codes;
use serde_json::json;
use std::collections::{HashMap, HashSet};

#[test]
fn a_missing_or_non_text_activation_value_is_refused_by_code() {
    for values in [None, Some(HashMap::from([("text".to_string(), json!(1))]))] {
        let error = activation_text(values.as_ref(), "text").unwrap_err();
        assert_eq!(
            error.reason_code(),
            Some(error_codes::INVALID_ACTIVATION_VALUE)
        );
    }
    let supplied = "Supplied text";
    let values = HashMap::from([("text".to_string(), json!(supplied))]);
    assert_eq!(activation_text(Some(&values), "text").unwrap(), supplied);
}

#[test]
fn logic_evaluation_is_refused_for_a_handler_that_is_not_a_logic_gate() {
    let node: FlowNode = serde_json::from_value(
        json!({"id": "gate", "definition_name": "Gate", "instance_name": "Gate"}),
    )
    .unwrap();
    let definition = |handler_id: &str| -> FlowNodeDefinition {
        serde_json::from_value(json!({
            "class_name": "Gate", "short_description": "Gate", "long_description": "Gate",
            "kind": "logic", "handler_id": handler_id,
            "action_ports": [{"name": "input"}], "signal_ports": [{"name": "output"}]
        }))
        .unwrap()
    };
    let fired = HashSet::new();
    let error = prepare_logic_outputs(
        &node,
        &definition("example.not_a_gate"),
        &[],
        &[],
        &fired,
        true,
    )
    .unwrap_err();
    assert_eq!(
        error.reason_code(),
        Some(error_codes::UNSUPPORTED_HANDLER_OPERATION)
    );
    prepare_logic_outputs(
        &node,
        &definition("battersea.logic.invert"),
        &[],
        &[],
        &fired,
        true,
    )
    .unwrap();
}
