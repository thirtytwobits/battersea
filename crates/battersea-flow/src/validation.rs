//! Copyright (c) Scott A Dixon

use crate::issue_codes as codes;
use crate::registry::Registry;
use crate::{
    FlowDocument, FlowEdgeKind, FlowNode, FlowParameterDefinition, FlowParameterEditorKind,
    FlowPortKind, FlowValidationIssue, FlowValidationResult,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

use super::catalog::node_definition;
use super::ports::{
    effective_accepted_token_types, effective_automation_accepted_token_types, expand_ports,
    expanded_action_ports_for_node, expanded_automation_ports_for_node,
    expanded_input_ports_for_node, expanded_output_ports_for_node, expanded_signal_ports_for_node,
    resolve_action_ports_for_node, resolve_automation_ports_for_node, resolve_input_ports_for_node,
    resolve_output_ports_for_node, resolve_signal_ports_for_node, token_connection_compatible,
    ResolvedFlowPort, ResolvedFlowPortSide,
};

/// Validate generic graph invariants and then the host's registered graph rules.
///
/// Generic issues carry codes from [`crate::issue_codes`]; host issues carry the host's codes.
pub fn validate_document(
    flow: &FlowDocument,
    definitions: &HashMap<String, crate::FlowNodeDefinition>,
    registry: &Registry,
) -> FlowValidationResult {
    if flow.version != crate::document::FLOW_DOCUMENT_VERSION {
        return FlowValidationResult {
            valid: false,
            issues: vec![FlowValidationIssue::new(
                codes::UNSUPPORTED_DOCUMENT_VERSION,
                format!("Unsupported flow document version {}", flow.version),
            )],
        };
    }
    let mut issues = Vec::new();
    if let Err(message) = crate::execution::validate_execution_contract(flow, definitions) {
        issues.push(FlowValidationIssue::new(
            codes::INVALID_EXECUTION_CONTRACT,
            message,
        ));
    }
    let mut node_ids = HashSet::new();
    let mut edge_ids = HashSet::new();
    let nodes_by_id = flow
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();

    for node in &flow.nodes {
        validate_node(node, definitions, registry, &mut node_ids, &mut issues);
    }

    let mut adjacency = HashMap::<String, Vec<String>>::new();
    let mut indegree = HashMap::<String, usize>::new();
    let mut token_targets = HashSet::<(String, String)>::new();
    let mut signal_sources = HashSet::<(String, String)>::new();
    let mut action_targets = HashSet::<(String, String)>::new();
    for node in &flow.nodes {
        adjacency.entry(node.id.clone()).or_default();
        indegree.entry(node.id.clone()).or_insert(0);
    }

    for edge in &flow.edges {
        validate_edge(
            flow,
            edge,
            definitions,
            &nodes_by_id,
            &mut edge_ids,
            &mut issues,
            &mut adjacency,
            &mut indegree,
            &mut token_targets,
            &mut signal_sources,
            &mut action_targets,
        );
    }

    let mut queue = indegree
        .iter()
        .filter_map(|(node_id, degree)| (*degree == 0).then_some(node_id.clone()))
        .collect::<VecDeque<_>>();
    let mut visited = 0usize;
    while let Some(node_id) = queue.pop_front() {
        visited += 1;
        if let Some(targets) = adjacency.get(&node_id) {
            for target in targets {
                if let Some(degree) = indegree.get_mut(target) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(target.clone());
                    }
                }
            }
        }
    }

    if visited != flow.nodes.len() {
        issues.push(FlowValidationIssue::new(
            codes::GRAPH_CYCLE,
            "Flow graph must be acyclic.",
        ));
    }

    issues.extend(registry.validate_graph(flow, definitions));

    FlowValidationResult {
        valid: issues.is_empty(),
        issues,
    }
}

/// Validates one node's structural identity, manifest definition, parameters, and
/// dynamic port expansion contract.
///
/// Node validation rejects blank or duplicate ids, blank instance names, unknown
/// definitions or parameters, invalid parameter values, any input/output dynamic port expansion failures surfaced by the
/// manifest-backed port helpers.
fn validate_node(
    node: &FlowNode,
    definitions: &HashMap<String, crate::FlowNodeDefinition>,
    registry: &Registry,
    node_ids: &mut HashSet<String>,
    issues: &mut Vec<FlowValidationIssue>,
) {
    let issue = |code: &str, message: String| {
        FlowValidationIssue::new(code, message).with_node_id(&node.id)
    };
    if node.id.trim().is_empty() {
        issues.push(issue(
            codes::BLANK_NODE_ID,
            "Flow nodes must have a non-empty id.".to_string(),
        ));
    }
    if !node_ids.insert(node.id.clone()) {
        issues.push(issue(
            codes::DUPLICATE_NODE_ID,
            format!("Duplicate node id \"{}\".", node.id),
        ));
    }
    if node.definition_name.trim().is_empty() {
        issues.push(issue(
            codes::BLANK_DEFINITION_NAME,
            "Flow nodes must have a non-empty definition_name.".to_string(),
        ));
        return;
    }
    if node.instance_name.trim().is_empty() {
        issues.push(issue(
            codes::BLANK_INSTANCE_NAME,
            "Flow nodes must have a non-empty instance_name.".to_string(),
        ));
    }

    let Some(definition) = node_definition(definitions, node) else {
        issues.push(issue(
            codes::UNKNOWN_DEFINITION,
            format!(
                "Flow node \"{}\" references unknown definition \"{}\".",
                node.id, node.definition_name
            ),
        ));
        return;
    };

    let known_parameters = definition
        .parameters
        .iter()
        .map(|parameter| (parameter.name.as_str(), parameter))
        .collect::<HashMap<_, _>>();
    for (parameter_name, value) in &node.parameter_values {
        let Some(parameter) = known_parameters.get(parameter_name.as_str()) else {
            issues.push(issue(
                codes::UNKNOWN_PARAMETER,
                format!(
                    "Flow node \"{}\" references unknown parameter \"{}\".",
                    node.id, parameter_name
                ),
            ));
            continue;
        };
        if let Some(message) = validate_parameter_value(parameter, value, registry) {
            issues.push(issue(
                codes::INVALID_PARAMETER_VALUE,
                format!(
                    "Flow node \"{}\" parameter \"{}\" is invalid: {}",
                    node.id, parameter_name, message
                ),
            ));
        }
    }
    validate_port_parameter_values(node, definition, definitions, registry, issues);

    for input in [true, false] {
        if let Err(message) = expand_ports(node, definition, input) {
            issues.push(issue(codes::INVALID_DYNAMIC_PORTS, message));
        }
    }

    validate_addressable_port_ids(node, definitions, issues);
    validate_port_aliases(node, definitions, issues);
}

fn validate_port_parameter_values(
    node: &FlowNode,
    // `definition` was the source of port names; now the expanded helpers do
    // that lookup via `definitions`. Keeping the param so adjacent validators
    // sharing this call shape stay symmetric, and to leave room for
    // definition-level checks if more land here.
    _definition: &crate::FlowNodeDefinition,
    definitions: &HashMap<String, crate::FlowNodeDefinition>,
    registry: &Registry,
    issues: &mut Vec<FlowValidationIssue>,
) {
    let Some(port_values) = &node.port_parameter_values else {
        return;
    };

    // Use *expanded* ports (fixed + dynamic) so a key like `input-2` on a
    // Concatenate node — whose input ports come from a `dynamic_input_ports`
    // group — resolves correctly. Falling back to `definition.input_ports`
    // alone would treat every dynamic port name as unknown.
    let expanded_input_ports = expanded_input_ports_for_node(definitions, node).unwrap_or_default();
    let expanded_output_ports =
        expanded_output_ports_for_node(definitions, node).unwrap_or_default();

    for (side_label, values_by_port, ports) in [
        ("input", &port_values.input, &expanded_input_ports),
        ("output", &port_values.output, &expanded_output_ports),
    ] {
        let ports_by_name = ports
            .iter()
            .map(|port| (port.name.as_str(), port))
            .collect::<HashMap<_, _>>();

        for (port_name, parameter_values) in values_by_port {
            let Some(port) = ports_by_name.get(port_name.as_str()) else {
                issues.push(
                    FlowValidationIssue::new(
                        codes::PORT_PARAMETERS_FOR_UNKNOWN_PORT,
                        format!(
                            "Flow node \"{}\" references unknown {} port \"{}\" in port_parameter_values.",
                            node.id, side_label, port_name
                        ),
                    )
                    .with_node_id(&node.id),
                );
                continue;
            };

            let parameters_by_name = port
                .parameters
                .iter()
                .map(|parameter| (parameter.name.as_str(), parameter))
                .collect::<HashMap<_, _>>();
            for (parameter_name, value) in parameter_values {
                let Some(parameter) = parameters_by_name.get(parameter_name.as_str()) else {
                    issues.push(
                        FlowValidationIssue::new(
                            codes::UNKNOWN_PORT_PARAMETER,
                            format!(
                                "Flow node \"{}\" {} port \"{}\" references unknown parameter \"{}\".",
                                node.id, side_label, port_name, parameter_name
                            ),
                        )
                        .with_node_id(&node.id),
                    );
                    continue;
                };
                if let Some(message) = validate_parameter_value(parameter, value, registry) {
                    issues.push(
                        FlowValidationIssue::new(
                            codes::INVALID_PORT_PARAMETER_VALUE,
                            format!(
                                "Flow node \"{}\" {} port \"{}\" parameter \"{}\" is invalid: {}",
                                node.id, side_label, port_name, parameter_name, message
                            ),
                        )
                        .with_node_id(&node.id),
                    );
                }
            }
        }
    }
}

fn validate_addressable_port_ids(
    node: &FlowNode,
    definitions: &HashMap<String, crate::FlowNodeDefinition>,
    issues: &mut Vec<FlowValidationIssue>,
) {
    let mut seen_sides = HashMap::<String, &'static str>::new();
    let side_sets = [
        (
            "action",
            resolve_action_ports_for_node(definitions, node).ok(),
        ),
        (
            "input",
            resolve_input_ports_for_node(definitions, node).ok(),
        ),
        (
            "output",
            resolve_output_ports_for_node(definitions, node).ok(),
        ),
        (
            "signal",
            resolve_signal_ports_for_node(definitions, node).ok(),
        ),
    ];

    for (side_label, ports) in side_sets {
        for port in ports.into_iter().flatten() {
            if let Some(existing_side) = seen_sides.insert(port.id.clone(), side_label) {
                issues.push(
                    FlowValidationIssue::new(
                        codes::DUPLICATE_PORT_ID,
                        format!(
                            "Flow node \"{}\" reuses canonical port id \"{}\" across the {} and {} sides.",
                            node.id, port.id, existing_side, side_label
                        ),
                    )
                    .with_node_id(&node.id),
                );
            }
        }
    }
}

/// Validates all per-node port aliases against the node's effective port sets.
///
/// Alias validation is presentation-only: aliases may customise labels, but
/// they must not invent ports, target ports that do not exist after dynamic
/// expansion, or introduce invalid alias values. Failures are reported per side
/// so that one invalid alias set does not prevent best-effort validation of the
/// remaining sides.
fn validate_port_aliases(
    node: &FlowNode,
    definitions: &HashMap<String, crate::FlowNodeDefinition>,
    issues: &mut Vec<FlowValidationIssue>,
) {
    let Some(port_names) = node.port_names.as_ref() else {
        return;
    };

    let side_sets = [
        (
            ResolvedFlowPortSide::Action,
            "action",
            resolve_action_ports_for_node(definitions, node),
        ),
        (
            ResolvedFlowPortSide::Automation,
            "automation",
            resolve_automation_ports_for_node(definitions, node),
        ),
        (
            ResolvedFlowPortSide::Input,
            "input",
            resolve_input_ports_for_node(definitions, node),
        ),
        (
            ResolvedFlowPortSide::Output,
            "output",
            resolve_output_ports_for_node(definitions, node),
        ),
        (
            ResolvedFlowPortSide::Signal,
            "signal",
            resolve_signal_ports_for_node(definitions, node),
        ),
    ];

    for (side, side_label, ports_result) in side_sets {
        let aliases = match side {
            ResolvedFlowPortSide::Action => &port_names.action,
            ResolvedFlowPortSide::Automation => &port_names.automation,
            ResolvedFlowPortSide::Input => &port_names.input,
            ResolvedFlowPortSide::Output => &port_names.output,
            ResolvedFlowPortSide::Signal => &port_names.signal,
        };

        let ports = match ports_result {
            Ok(ports) => ports,
            Err(message) => {
                issues.push(
                    FlowValidationIssue::new(codes::INVALID_DYNAMIC_PORTS, message)
                        .with_node_id(&node.id),
                );
                continue;
            }
        };

        validate_port_alias_side(node, side_label, aliases, &ports, issues);
    }
}

/// Validates one side's alias map against the resolved ports for that side.
///
/// Every alias entry must refer to a known canonical port id on the requested
/// side, and every stored alias must resolve to a non-blank display name.
/// Aliases must also be unique within that side so the node exposes one
/// unambiguous display label per effective port.
fn validate_port_alias_side(
    node: &FlowNode,
    side_label: &str,
    aliases: &std::collections::BTreeMap<String, String>,
    ports: &[ResolvedFlowPort],
    issues: &mut Vec<FlowValidationIssue>,
) {
    if aliases.is_empty() {
        return;
    }

    let known_port_ids = ports
        .iter()
        .map(|port| port.id.as_str())
        .collect::<HashSet<_>>();
    let mut seen_aliases = HashSet::<String>::new();

    let issue = |code: &str, message: String| {
        FlowValidationIssue::new(code, message).with_node_id(&node.id)
    };
    for (port_id, alias) in aliases {
        if !known_port_ids.contains(port_id.as_str()) {
            issues.push(issue(
                codes::ALIAS_FOR_UNKNOWN_PORT,
                format!(
                    "Flow node \"{}\" aliases unknown {} port \"{}\".",
                    node.id, side_label, port_id
                ),
            ));
            continue;
        }

        let trimmed = alias.trim();
        if trimmed.is_empty() {
            issues.push(issue(
                codes::BLANK_PORT_ALIAS,
                format!(
                    "Flow node \"{}\" alias for {} port \"{}\" must not be blank.",
                    node.id, side_label, port_id
                ),
            ));
            continue;
        }

        if !seen_aliases.insert(trimmed.to_string()) {
            issues.push(issue(
                codes::DUPLICATE_PORT_ALIAS,
                format!(
                    "Flow node \"{}\" declares duplicate {} port alias \"{}\".",
                    node.id, side_label, trimmed
                ),
            ));
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Validates one edge against the known node set, manifest definitions, and per-kind
/// connectivity rules.
///
/// Token edges must connect known output and input ports with compatible token types and
/// contribute to the token graph used for cycle detection. Signal edges must connect
/// known signal and action ports and enforce one outgoing edge per signal port plus one
/// incoming edge per action port.
fn validate_edge(
    _flow: &FlowDocument,
    edge: &crate::FlowEdge,
    definitions: &HashMap<String, crate::FlowNodeDefinition>,
    nodes_by_id: &HashMap<&str, &FlowNode>,
    edge_ids: &mut HashSet<String>,
    issues: &mut Vec<FlowValidationIssue>,
    adjacency: &mut HashMap<String, Vec<String>>,
    indegree: &mut HashMap<String, usize>,
    token_targets: &mut HashSet<(String, String)>,
    signal_sources: &mut HashSet<(String, String)>,
    action_targets: &mut HashSet<(String, String)>,
) {
    let issue = |code: &str, message: String| {
        FlowValidationIssue::new(code, message).with_edge_id(&edge.id)
    };
    if edge.id.trim().is_empty() {
        issues.push(issue(
            codes::BLANK_EDGE_ID,
            "Flow edges must have a non-empty id.".to_string(),
        ));
    }
    if !edge_ids.insert(edge.id.clone()) {
        issues.push(issue(
            codes::DUPLICATE_EDGE_ID,
            format!("Duplicate edge id \"{}\".", edge.id),
        ));
    }

    let source_node = nodes_by_id.get(edge.source_node_id.as_str()).copied();
    let target_node = nodes_by_id.get(edge.target_node_id.as_str()).copied();
    let Some(source_node) = source_node else {
        issues.push(issue(
            codes::UNKNOWN_SOURCE_NODE,
            format!(
                "Flow edge \"{}\" references unknown source node \"{}\".",
                edge.id, edge.source_node_id
            ),
        ));
        return;
    };
    let Some(target_node) = target_node else {
        issues.push(issue(
            codes::UNKNOWN_TARGET_NODE,
            format!(
                "Flow edge \"{}\" references unknown target node \"{}\".",
                edge.id, edge.target_node_id
            ),
        ));
        return;
    };
    let unknown_definition = |node: &FlowNode| {
        issue(
            codes::UNKNOWN_DEFINITION,
            format!(
                "Flow node \"{}\" references unknown definition \"{}\".",
                node.id, node.definition_name
            ),
        )
        .with_node_id(&node.id)
    };
    let invalid_dynamic_ports = |node: &FlowNode, message: String| {
        issue(codes::INVALID_DYNAMIC_PORTS, message).with_node_id(&node.id)
    };

    match edge.kind {
        FlowEdgeKind::Token => {
            let Some(source_definition) = node_definition(definitions, source_node) else {
                issues.push(unknown_definition(source_node));
                return;
            };
            let Some(target_definition) = node_definition(definitions, target_node) else {
                issues.push(unknown_definition(target_node));
                return;
            };
            let source_ports = match expanded_output_ports_for_node(definitions, source_node) {
                Ok(ports) => ports,
                Err(message) => {
                    issues.push(invalid_dynamic_ports(source_node, message));
                    return;
                }
            };
            let target_input_ports = match expanded_input_ports_for_node(definitions, target_node) {
                Ok(ports) => ports,
                Err(message) => {
                    issues.push(invalid_dynamic_ports(target_node, message));
                    return;
                }
            };
            let target_automation_ports =
                match expanded_automation_ports_for_node(definitions, target_node) {
                    Ok(ports) => ports,
                    Err(message) => {
                        issues.push(invalid_dynamic_ports(target_node, message));
                        return;
                    }
                };

            let Some(source_port) = source_ports
                .iter()
                .find(|port| port.name == edge.source_port)
            else {
                issues.push(
                    issue(
                        codes::UNKNOWN_SOURCE_PORT,
                        format!(
                            "Flow edge \"{}\" references unknown source port \"{}\" on node \"{}\".",
                            edge.id, edge.source_port, edge.source_node_id
                        ),
                    )
                    .with_node_id(&edge.source_node_id),
                );
                return;
            };
            // Token edges may target either a regular input port or an
            // automation port. Look up the input list first, then the
            // automation list. Compatibility is checked against whichever
            // list the target lives in.
            let target_input_port = target_input_ports
                .iter()
                .find(|port| port.name == edge.target_port);
            let target_automation_port = target_automation_ports
                .iter()
                .find(|port| port.name == edge.target_port);
            if target_input_port.is_none() && target_automation_port.is_none() {
                issues.push(
                    issue(
                        codes::UNKNOWN_TARGET_PORT,
                        format!(
                            "Flow edge \"{}\" references unknown target port \"{}\" on node \"{}\".",
                            edge.id, edge.target_port, edge.target_node_id
                        ),
                    )
                    .with_node_id(&edge.target_node_id),
                );
                return;
            }

            let misdirected = || {
                issue(
                    codes::MISDIRECTED_TOKEN_EDGE,
                    format!(
                        "Flow edge \"{}\" must connect an output port to an input or automation port.",
                        edge.id
                    ),
                )
            };
            if source_port.kind != FlowPortKind::Output {
                issues.push(misdirected());
            }
            if !token_targets.insert((edge.target_node_id.clone(), edge.target_port.clone())) {
                issues.push(
                    issue(
                        codes::DUPLICATE_INPUT_EDGE,
                        format!(
                            "Input port \"{}\" on node \"{}\" already has an incoming edge.",
                            edge.target_port, edge.target_node_id
                        ),
                    )
                    .with_node_id(&edge.target_node_id),
                );
            }
            // Token-type compatibility is decided by the single shared
            // rule `token_connection_compatible` (see
            // `super::ports`), which the editor's TS check mirrors and
            // both pin to `engine/bindings/fixtures/token-type-compatibility.json`.
            // An `auto` source can only ever carry one of its node's
            // input accepted types, so gather that union here.
            let source_is_auto = source_port.token_type == "auto";
            let source_node_input_accepted: Vec<String> = if source_is_auto {
                expanded_input_ports_for_node(definitions, source_node)
                    .ok()
                    .map(|ports| {
                        ports
                            .iter()
                            .flat_map(effective_accepted_token_types)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let source_descriptor = if source_is_auto {
                if source_node_input_accepted.is_empty() {
                    "auto".to_string()
                } else {
                    source_node_input_accepted.join("|")
                }
            } else {
                source_port.token_type.clone()
            };
            let incompatible = |target_token_type: &str| {
                issue(
                    codes::INCOMPATIBLE_TOKEN_TYPES,
                    format!(
                        "Flow edge \"{}\" connects incompatible token types \"{}\" and \"{}\".",
                        edge.id, source_descriptor, target_token_type
                    ),
                )
            };
            if let Some(target_port) = target_input_port {
                if target_port.kind != FlowPortKind::Input {
                    issues.push(misdirected());
                }
                if !token_connection_compatible(
                    &source_port.token_type,
                    &source_node_input_accepted,
                    &effective_accepted_token_types(target_port),
                ) {
                    issues.push(incompatible(&target_port.token_type));
                }
            } else if let Some(target_port) = target_automation_port {
                if !token_connection_compatible(
                    &source_port.token_type,
                    &source_node_input_accepted,
                    &effective_automation_accepted_token_types(target_port),
                ) {
                    issues.push(incompatible(&target_port.token_type));
                }
            }

            if !matches!(source_definition.kind, crate::FlowNodeClass::Hybrid)
                && !matches!(target_definition.kind, crate::FlowNodeClass::Hybrid)
            {
                adjacency
                    .entry(edge.source_node_id.clone())
                    .or_default()
                    .push(edge.target_node_id.clone());
                *indegree.entry(edge.target_node_id.clone()).or_insert(0) += 1;
            }
        }
        FlowEdgeKind::Signal => {
            if node_definition(definitions, source_node).is_none() {
                issues.push(unknown_definition(source_node));
                return;
            }
            if node_definition(definitions, target_node).is_none() {
                issues.push(unknown_definition(target_node));
                return;
            }

            let source_signal_ports = match expanded_signal_ports_for_node(definitions, source_node)
            {
                Ok(ports) => ports,
                Err(message) => {
                    issues.push(invalid_dynamic_ports(source_node, message));
                    return;
                }
            };
            let source_port_exists = source_signal_ports
                .iter()
                .any(|port| port.name == edge.source_port);
            if !source_port_exists {
                issues.push(
                    issue(
                        codes::UNKNOWN_SIGNAL_PORT,
                        format!(
                            "Flow edge \"{}\" references unknown signal port \"{}\" on node \"{}\".",
                            edge.id, edge.source_port, edge.source_node_id
                        ),
                    )
                    .with_node_id(&edge.source_node_id),
                );
                return;
            }

            let target_action_ports = match expanded_action_ports_for_node(definitions, target_node)
            {
                Ok(ports) => ports,
                Err(message) => {
                    issues.push(invalid_dynamic_ports(target_node, message));
                    return;
                }
            };
            let target_port_exists = target_action_ports
                .iter()
                .any(|port| port.name == edge.target_port);
            if !target_port_exists {
                issues.push(
                    issue(
                        codes::UNKNOWN_ACTION_PORT,
                        format!(
                            "Flow edge \"{}\" references unknown action port \"{}\" on node \"{}\".",
                            edge.id, edge.target_port, edge.target_node_id
                        ),
                    )
                    .with_node_id(&edge.target_node_id),
                );
                return;
            }

            if !signal_sources.insert((edge.source_node_id.clone(), edge.source_port.clone())) {
                issues.push(
                    issue(
                        codes::DUPLICATE_SIGNAL_EDGE,
                        format!(
                            "Signal port \"{}\" on node \"{}\" already has an outgoing edge.",
                            edge.source_port, edge.source_node_id
                        ),
                    )
                    .with_node_id(&edge.source_node_id),
                );
            }
            if !action_targets.insert((edge.target_node_id.clone(), edge.target_port.clone())) {
                issues.push(
                    issue(
                        codes::DUPLICATE_ACTION_EDGE,
                        format!(
                            "Action port \"{}\" on node \"{}\" already has an incoming edge.",
                            edge.target_port, edge.target_node_id
                        ),
                    )
                    .with_node_id(&edge.target_node_id),
                );
            }
        }
    }
}

/// Validates one concrete parameter value against its declared datatype and editor rules.
///
/// Datatype mismatches are rejected before any editor-specific validation runs. Once the
/// datatype is correct, editor kinds may impose additional integer bounds, option sets,
/// list constraints, or structured-query/location validation.
pub fn validate_parameter_value(
    parameter: &FlowParameterDefinition,
    value: &Value,
    registry: &Registry,
) -> Option<String> {
    if let Err(error) = registry.validate_value(&parameter.datatype, value) {
        return Some(error.to_string());
    }

    match parameter.editor.kind {
        FlowParameterEditorKind::Boolean => {
            if value.as_bool().is_none() {
                return Some("value must be a boolean".to_string());
            }
        }
        FlowParameterEditorKind::Unsigned
        | FlowParameterEditorKind::InputPortCount
        | FlowParameterEditorKind::OutputPortCount => {
            let Some(number) = value.as_i64() else {
                return Some("value must be an integer".into());
            };
            if number < 0 {
                return Some("value must be zero or greater".to_string());
            }
            if let Some(min) = parameter.editor.min {
                if number < min as i64 {
                    return Some(format!("value must be at least {min}"));
                }
            }
            if let Some(max) = parameter.editor.max {
                if number > max as i64 {
                    return Some(format!("value must be at most {max}"));
                }
            }
        }
        FlowParameterEditorKind::Enum => {
            let Some(value) = value.as_str() else {
                return Some("value must be a string".into());
            };
            if !parameter.editor.values.iter().any(|option| option == value) {
                return Some("value must match one of the declared enum values".to_string());
            }
        }
        FlowParameterEditorKind::List => {
            let Some(items) = value.as_array() else {
                return Some("value must be a list".into());
            };
            if let Some(max) = parameter.editor.max {
                if items.len() > max as usize {
                    return Some(format!("list may contain at most {max} items"));
                }
            }
            if !parameter.editor.values.is_empty() {
                for item in items {
                    let Some(item) = item.as_str() else {
                        return Some("list editor values must be strings".to_string());
                    };
                    if !parameter.editor.values.iter().any(|option| option == item) {
                        return Some("list values must match the declared options".to_string());
                    }
                }
            }
        }
        FlowParameterEditorKind::MediaPreview | FlowParameterEditorKind::Query => {}
        FlowParameterEditorKind::Properties => {
            if value.as_object().is_none() {
                return Some("value must be an object".to_string());
            }
        }
        FlowParameterEditorKind::String
        | FlowParameterEditorKind::Text
        | FlowParameterEditorKind::TextInput => {}
    }

    None
}
