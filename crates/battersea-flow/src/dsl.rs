use crate::{FlowDocument, FlowEdgeKind, FlowNode, FlowNodeClass, FlowNodeDefinition};

use crate::output_encoding::{
    flow_output_encoding, read_output_encoding, read_plain_fragment_delimiter,
    read_whitespace_mode, OutputEncoding, WhitespaceMode,
};
use crate::ports::effective_parameter_value;

use super::catalog::{node_implements_interface, FLOW_NODE_ACTIVATE_INTERFACE};
use std::collections::HashMap;

/// Lists the ids of source nodes that can be used as explicit flow activation entrypoints.
pub fn activatable_node_ids(
    flow: &FlowDocument,
    definitions: &HashMap<String, FlowNodeDefinition>,
) -> Vec<String> {
    flow.nodes
        .iter()
        .filter(|node| node_implements_interface(definitions, node, FLOW_NODE_ACTIVATE_INTERFACE))
        .map(|node| node.id.clone())
        .collect()
}

/// Exports a concise DSL view of a flow for diagnostics and tests.
pub fn export_flow_dsl(
    flow: &FlowDocument,
    definitions: &HashMap<String, FlowNodeDefinition>,
) -> String {
    let mut lines = vec![format!(
        "flow \"{}\" output_encoding={} plain_fragment_delimiter={} whitespace_mode={} {{",
        flow.flow_key, flow.output_encoding, flow.plain_fragment_delimiter, flow.whitespace_mode
    )];

    for node in &flow.nodes {
        let definition = definitions.get(node.definition_name.as_str());
        let role = definition
            .map(|definition| match definition.kind {
                FlowNodeClass::Source => "source",
                FlowNodeClass::Control => "control",
                FlowNodeClass::Hybrid => "hybrid",
                FlowNodeClass::Instrument => "instrument",
                FlowNodeClass::Inline => "inline",
                FlowNodeClass::Logic => "logic",
                FlowNodeClass::Sink => "sink",
            })
            .unwrap_or("node");

        let encoding_attrs = definition
            .and_then(|definition| node_encoding_attrs(flow, node, definition))
            .unwrap_or_default();
        lines.push(format!(
            "  {role} {} as {}{}",
            node.definition_name, node.id, encoding_attrs
        ));
    }

    for edge in &flow.edges {
        let connector = match edge.kind {
            FlowEdgeKind::Token => "->",
            FlowEdgeKind::Signal => "~>",
        };
        lines.push(format!(
            "  {}.{} {} {}.{}",
            edge.source_node_id, edge.source_port, connector, edge.target_node_id, edge.target_port
        ));
    }

    lines.push("}".to_string());
    lines.join("\n")
}

fn node_encoding_attrs(
    flow: &FlowDocument,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
) -> Option<String> {
    if !definition
        .parameters
        .iter()
        .any(|parameter| parameter.name == "output_encoding")
    {
        return None;
    }
    let declared_output_encoding = node
        .parameter_values
        .get("output_encoding")
        .and_then(|value| value.as_str())
        .unwrap_or("inherit");
    let flow_encoding = flow_output_encoding(flow);
    let effective_output_encoding = read_output_encoding(node, definition, flow_encoding);
    let declared_plain_fragment_delimiter = node
        .parameter_values
        .get("plain_fragment_delimiter")
        .and_then(|value| value.as_str())
        .unwrap_or("inherit");
    let effective_plain_fragment_delimiter = read_plain_fragment_delimiter(node, definition, flow);
    let declared_whitespace_mode = node
        .parameter_values
        .get("whitespace_mode")
        .and_then(|value| value.as_str())
        .unwrap_or("inherit");
    let effective_whitespace_mode = read_whitespace_mode(node, definition, flow);
    let defaulted_output_encoding = effective_parameter_value(node, definition, "output_encoding")
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| declared_output_encoding.to_string());
    Some(format!(
        " [output_encoding={} effective_output_encoding={} defaulted_output_encoding={} \
         plain_fragment_delimiter={} effective_plain_fragment_delimiter={} \
         whitespace_mode={} effective_whitespace_mode={}]",
        declared_output_encoding,
        output_encoding_token(effective_output_encoding),
        defaulted_output_encoding,
        declared_plain_fragment_delimiter,
        delimiter_token(&effective_plain_fragment_delimiter),
        declared_whitespace_mode,
        whitespace_mode_token(effective_whitespace_mode),
    ))
}

fn output_encoding_token(encoding: OutputEncoding) -> &'static str {
    encoding.variant_suffix()
}

fn whitespace_mode_token(mode: WhitespaceMode) -> &'static str {
    match mode {
        WhitespaceMode::Trim => "trim",
        WhitespaceMode::Preserve => "preserve",
        WhitespaceMode::Compact => "compact",
    }
}

fn delimiter_token(delimiter: &str) -> String {
    match delimiter {
        "," => "comma".to_string(),
        "\n\n" => "blank_line".to_string(),
        "\n" => "newline".to_string(),
        " " => "space".to_string(),
        "" => "none".to_string(),
        other => format!("{other:?}"),
    }
}
