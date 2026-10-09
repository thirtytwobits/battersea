//! Copyright (c) Scott A Dixon

use crate::{
    FlowActionPortDefinition, FlowAutomationPortDefinition, FlowDynamicSignalPortGroup, FlowNode,
    FlowNodeDefinition, FlowPort, FlowPortKind, FlowSignalPortDefinition,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use super::catalog::node_definition;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedFlowPortSide {
    Action,
    Automation,
    Input,
    Output,
    Signal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFlowPort {
    pub id: String,
    pub name: Option<String>,
    pub label: String,
    pub side: ResolvedFlowPortSide,
    pub token_type: Option<String>,
    pub accepted_token_types: Vec<String>,
    pub short_description: Option<String>,
    pub long_description: Option<String>,
}

pub fn effective_accepted_token_types(port: &FlowPort) -> Vec<String> {
    if port.accepted_token_types.is_empty() {
        vec![port.token_type.clone()]
    } else {
        port.accepted_token_types.clone()
    }
}

pub fn port_accepts_token_type(port: &FlowPort, token_type: &str) -> bool {
    effective_accepted_token_types(port)
        .iter()
        .any(|accepted| accepted == token_type)
}

/// The single source of truth for token-edge type compatibility.
///
/// `target_accepted` is the target port's effective accepted-type list.
/// `source_node_input_accepted` is the union of the source node's input
/// ports' effective accepted types — consulted only for `auto` sources
/// (a Multiplexer-style output passes its node's input kind straight
/// through, so it can only ever carry a type that input accepts).
///
/// An `auto` source with no typed inputs carries no static information,
/// so the connection is deferred (accepted) and the engine re-checks at
/// emit time. Concrete sources match by exact type.
///
/// The editor's TypeScript `connectionUsesCompatibleTokenTypes`
/// implements this identical rule; both are pinned to
/// `fixtures/token-type-compatibility.json` so they cannot drift.
pub fn token_connection_compatible(
    source_token_type: &str,
    source_node_input_accepted: &[String],
    target_accepted: &[String],
) -> bool {
    if source_token_type == "auto" {
        if source_node_input_accepted.is_empty() {
            return true;
        }
        return source_node_input_accepted
            .iter()
            .any(|candidate| target_accepted.iter().any(|accepted| accepted == candidate));
    }
    target_accepted
        .iter()
        .any(|accepted| accepted == source_token_type)
}

pub fn effective_automation_accepted_token_types(
    port: &FlowAutomationPortDefinition,
) -> Vec<String> {
    if port.accepted_token_types.is_empty() {
        vec![port.token_type.clone()]
    } else {
        port.accepted_token_types.clone()
    }
}

pub fn automation_port_accepts_token_type(
    port: &FlowAutomationPortDefinition,
    token_type: &str,
) -> bool {
    effective_automation_accepted_token_types(port)
        .iter()
        .any(|accepted| accepted == token_type)
}

/// Returns the concrete input ports for a node definition, including any
/// manifest-declared dynamic input ports.
///
/// The node's definition must be resolvable from the supplied definitions map.
/// Only input-side fixed ports and dynamic input groups are considered.
pub fn expanded_input_ports_for_node(
    definitions: &HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Result<Vec<FlowPort>, String> {
    let definition = node_definition(definitions, node).ok_or_else(|| {
        format!(
            "Flow node \"{}\" references unknown definition \"{}\".",
            node.id, node.definition_name
        )
    })?;
    expand_ports(node, definition, true)
}

/// Returns the concrete output ports for a node definition, including any
/// manifest-declared dynamic output ports.
///
/// The node's definition must be resolvable from the supplied definitions map.
/// Only output-side fixed ports and dynamic output groups are considered.
pub fn expanded_output_ports_for_node(
    definitions: &HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Result<Vec<FlowPort>, String> {
    let definition = node_definition(definitions, node).ok_or_else(|| {
        format!(
            "Flow node \"{}\" references unknown definition \"{}\".",
            node.id, node.definition_name
        )
    })?;
    expand_ports(node, definition, false)
}

/// Resolves the effective input-port view for a node.
///
/// The returned ports must preserve the node definition's canonical input ids
/// for wiring and lookup, apply any valid node-instance aliases only as
/// presentation names, and keep the same concrete port set and ordering as the
/// node's expanded input ports.
pub fn resolve_input_ports_for_node(
    definitions: &HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Result<Vec<ResolvedFlowPort>, String> {
    let ports = expanded_input_ports_for_node(definitions, node)?;
    Ok(resolve_token_ports(
        node,
        ports,
        ResolvedFlowPortSide::Input,
    ))
}

/// Resolves the effective output-port view for a node.
///
/// The returned ports must preserve the node definition's canonical output ids
/// for wiring and lookup, apply any valid node-instance aliases only as
/// presentation names, and keep the same concrete port set and ordering as the
/// node's expanded output ports.
pub fn resolve_output_ports_for_node(
    definitions: &HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Result<Vec<ResolvedFlowPort>, String> {
    let ports = expanded_output_ports_for_node(definitions, node)?;
    Ok(resolve_token_ports(
        node,
        ports,
        ResolvedFlowPortSide::Output,
    ))
}

/// Resolves the effective action-port view for a node.
///
/// Action ports are always addressed by their canonical ids. Any node-instance
/// aliases are presentation-only and must not change which actions the node
/// exposes or the order in which they are reported.
pub fn resolve_action_ports_for_node(
    definitions: &HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Result<Vec<ResolvedFlowPort>, String> {
    let definition = node_definition(definitions, node).ok_or_else(|| {
        format!(
            "Flow node \"{}\" references unknown definition \"{}\".",
            node.id, node.definition_name
        )
    })?;
    let ports = expand_named_ports(
        node,
        definition,
        &definition.action_ports,
        &definition.dynamic_action_ports,
        ResolvedFlowPortSide::Action,
    )?;
    Ok(resolve_named_ports(
        node,
        &ports,
        ResolvedFlowPortSide::Action,
    ))
}

/// Resolves the effective automation-port view for a node.
///
/// Automation ports are typed sinks. The resolved record carries the port's
/// `token_type` and `accepted_token_types` so connection compatibility can
/// reuse the same predicate that input/output ports use, while addressing
/// continues to use the canonical port name.
pub fn resolve_automation_ports_for_node(
    definitions: &HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Result<Vec<ResolvedFlowPort>, String> {
    let ports = expanded_automation_ports_for_node(definitions, node)?;
    Ok(ports
        .into_iter()
        .map(|port| {
            let id = port.name.clone();
            let alias = port_alias_for_side(node, ResolvedFlowPortSide::Automation, &id);
            let accepted_token_types = if port.accepted_token_types.is_empty() {
                vec![port.token_type.clone()]
            } else {
                port.accepted_token_types.clone()
            };
            ResolvedFlowPort {
                label: alias.clone().unwrap_or_else(|| id.clone()),
                id,
                name: alias,
                side: ResolvedFlowPortSide::Automation,
                token_type: Some(port.token_type),
                accepted_token_types,
                short_description: port.short_description,
                long_description: port.long_description,
            }
        })
        .collect())
}

/// Resolves the effective signal-port view for a node.
///
/// Signal ports are always addressed by their canonical ids. Any node-instance
/// aliases are presentation-only and must not change which signals the node
/// exposes or the order in which they are reported.
pub fn resolve_signal_ports_for_node(
    definitions: &HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Result<Vec<ResolvedFlowPort>, String> {
    let definition = node_definition(definitions, node).ok_or_else(|| {
        format!(
            "Flow node \"{}\" references unknown definition \"{}\".",
            node.id, node.definition_name
        )
    })?;
    let ports = expand_named_ports(
        node,
        definition,
        &definition.signal_ports,
        &definition.dynamic_signal_ports,
        ResolvedFlowPortSide::Signal,
    )?;
    Ok(resolve_named_ports(
        node,
        &ports,
        ResolvedFlowPortSide::Signal,
    ))
}

/// Resolves the effective value for one node parameter.
///
/// Explicit node parameter values win over manifest defaults. If the node does
/// not provide a value, the definition parameter's editor `default_value` is
/// used when present. Missing parameters yield `None`.
pub fn effective_parameter_value(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    parameter_name: &str,
) -> Option<Value> {
    node.parameter_values
        .get(parameter_name)
        .cloned()
        .or_else(|| {
            definition
                .parameters
                .iter()
                .find(|parameter| parameter.name == parameter_name)
                .and_then(|parameter| parameter.editor.default_value.clone())
        })
}

pub fn effective_port_parameter_value(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    port_kind: FlowPortKind,
    port_name: &str,
    parameter_name: &str,
) -> Option<Value> {
    let port = match port_kind {
        FlowPortKind::Input => definition.input_ports.iter(),
        FlowPortKind::Output => definition.output_ports.iter(),
    }
    .find(|port| port.name == port_name)?;

    node.port_parameter_values
        .as_ref()
        .and_then(|port_values| match port_kind {
            FlowPortKind::Input => port_values.input.get(port_name),
            FlowPortKind::Output => port_values.output.get(port_name),
        })
        .and_then(|parameter_values| parameter_values.get(parameter_name))
        .cloned()
        .or_else(|| {
            port.parameters
                .iter()
                .find(|parameter| parameter.name == parameter_name)
                .and_then(|parameter| parameter.editor.default_value.clone())
        })
}

/// Expands the fixed and dynamic ports for one definition direction into a
/// concrete ordered port list.
///
/// Fixed ports keep their original order. Dynamic groups are appended in group
/// order, then zero-based index order within each group. Dynamic count
/// parameters must resolve to integers greater than or equal to zero, and the
/// final expanded port names must remain unique within the requested direction.
pub(crate) fn expand_ports(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    input: bool,
) -> Result<Vec<FlowPort>, String> {
    let mut ports = if input {
        definition.input_ports.clone()
    } else {
        definition.output_ports.clone()
    };
    let groups = if input {
        &definition.dynamic_input_ports
    } else {
        &definition.dynamic_output_ports
    };

    let mut names = ports
        .iter()
        .map(|port| port.name.clone())
        .collect::<HashSet<_>>();
    for group in groups {
        let count = resolve_dynamic_count(node, definition, &group.count_parameter)?;
        for index in 0..count {
            let name = group
                .name_template
                .replace("{index}", index.to_string().as_str());
            if !names.insert(name.clone()) {
                return Err(format!(
                    "Flow node \"{}\" expands duplicate port name \"{}\".",
                    node.id, name
                ));
            }
            ports.push(FlowPort {
                name,
                kind: if input {
                    FlowPortKind::Input
                } else {
                    FlowPortKind::Output
                },
                token_type: group.token_type.clone(),
                mode: group.mode,
                phase: group.phase,
                display_class: group.display_class,
                accepted_token_types: if input {
                    group.accepted_token_types.clone()
                } else {
                    Vec::new()
                },
                short_description: group.short_description.clone(),
                long_description: group.long_description.clone(),
                formatter: None,
                // Every expanded port inherits the group's per-port parameter
                // declarations so the detail-pane UI can surface editors for
                // each instance (e.g. Concatenate's `array_delimiter`).
                parameters: group.parameters.clone(),
            });
        }
    }

    Ok(ports)
}

/// Builds resolved token-port records from concrete input or output ports.
///
/// Each resolved port must retain the supplied canonical port id, derive its
/// user-facing label from the node-instance alias when one is defined for that
/// side and id, and otherwise fall back to the canonical id. Token metadata and
/// descriptions must be preserved for later validation and presentation.
fn resolve_token_ports(
    node: &FlowNode,
    ports: Vec<FlowPort>,
    side: ResolvedFlowPortSide,
) -> Vec<ResolvedFlowPort> {
    ports
        .into_iter()
        .map(|port| {
            let alias = port_alias_for_side(node, side, &port.name);
            let accepted_token_types = port_accepts_for_side(&port, side);
            ResolvedFlowPort {
                label: alias.clone().unwrap_or_else(|| port.name.clone()),
                id: port.name,
                name: alias,
                side,
                token_type: Some(port.token_type),
                accepted_token_types,
                short_description: port.short_description,
                long_description: port.long_description,
            }
        })
        .collect()
}

/// Builds resolved action or signal port records from named definition ports.
///
/// The returned records must preserve canonical port ids and declaration order,
/// apply aliases only to presentation fields, and carry through any available
/// descriptive metadata for the owning named port type.
fn resolve_named_ports<T>(
    node: &FlowNode,
    ports: &[T],
    side: ResolvedFlowPortSide,
) -> Vec<ResolvedFlowPort>
where
    T: NamedFlowPort,
{
    ports
        .iter()
        .map(|port| {
            let id = port.port_id().to_string();
            let alias = port_alias_for_side(node, side, &id);
            ResolvedFlowPort {
                label: alias.clone().unwrap_or_else(|| id.clone()),
                id,
                name: alias,
                side,
                token_type: None,
                accepted_token_types: Vec::new(),
                short_description: port.short_description().cloned(),
                long_description: port.long_description().cloned(),
            }
        })
        .collect()
}

pub fn expanded_action_ports_for_node(
    definitions: &HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Result<Vec<FlowActionPortDefinition>, String> {
    let definition = node_definition(definitions, node).ok_or_else(|| {
        format!(
            "Flow node \"{}\" references unknown definition \"{}\".",
            node.id, node.definition_name
        )
    })?;
    expand_named_ports(
        node,
        definition,
        &definition.action_ports,
        &definition.dynamic_action_ports,
        ResolvedFlowPortSide::Action,
    )
}

/// Returns the automation ports declared on a node's definition. Automation
/// ports are typed sinks that write into one of the host node's
/// `parameter_values` post-activation; they are addressed by canonical name and
/// do not currently support dynamic groups.
pub fn expanded_automation_ports_for_node(
    definitions: &HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Result<Vec<FlowAutomationPortDefinition>, String> {
    let definition = node_definition(definitions, node).ok_or_else(|| {
        format!(
            "Flow node \"{}\" references unknown definition \"{}\".",
            node.id, node.definition_name
        )
    })?;
    Ok(definition.automation_ports.clone())
}

pub fn expanded_signal_ports_for_node(
    definitions: &HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Result<Vec<FlowSignalPortDefinition>, String> {
    let definition = node_definition(definitions, node).ok_or_else(|| {
        format!(
            "Flow node \"{}\" references unknown definition \"{}\".",
            node.id, node.definition_name
        )
    })?;
    expand_named_ports(
        node,
        definition,
        &definition.signal_ports,
        &definition.dynamic_signal_ports,
        ResolvedFlowPortSide::Signal,
    )
}

fn expand_named_ports<T>(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    fixed_ports: &[T],
    groups: &[FlowDynamicSignalPortGroup],
    side: ResolvedFlowPortSide,
) -> Result<Vec<T>, String>
where
    T: NamedFlowPort + FromDynamicSignalPort,
{
    let mut ports = fixed_ports.to_vec();
    let mut names = ports
        .iter()
        .map(|port| port.port_id().to_string())
        .collect::<HashSet<_>>();
    for group in groups {
        let count = resolve_dynamic_count(node, definition, &group.count_parameter)?;
        for index in 0..count {
            let name = group
                .name_template
                .replace("{index}", index.to_string().as_str());
            if !names.insert(name.clone()) {
                return Err(format!(
                    "Flow node \"{}\" expands duplicate {:?} port name \"{}\".",
                    node.id, side, name
                ));
            }
            ports.push(T::from_dynamic_signal_port(name, group));
        }
    }
    Ok(ports)
}

fn resolve_dynamic_count(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    count_parameter: &str,
) -> Result<i64, String> {
    let count_value =
        effective_parameter_value(node, definition, count_parameter).ok_or_else(|| {
            format!(
                "Flow node \"{}\" is missing a value for dynamic port count parameter \"{}\".",
                node.id, count_parameter
            )
        })?;
    let Some(count) = count_value.as_i64() else {
        return Err(format!(
            "Flow node \"{}\" count parameter \"{}\" must be an integer.",
            node.id, count_parameter
        ));
    };
    if count < 0 {
        return Err(format!(
            "Flow node \"{}\" count parameter \"{}\" must be zero or greater.",
            node.id, count_parameter
        ));
    }
    if let Some(parameter) = definition
        .parameters
        .iter()
        .find(|parameter| parameter.name == count_parameter)
    {
        if parameter
            .editor
            .min
            .is_some_and(|min| count < i64::from(min))
            || parameter
                .editor
                .max
                .is_some_and(|max| count > i64::from(max))
        {
            return Err(format!(
                "Flow node \"{}\" count parameter \"{}\" is outside its declared bounds.",
                node.id, count_parameter
            ));
        }
    }
    Ok(count)
}

fn port_accepts_for_side(port: &FlowPort, side: ResolvedFlowPortSide) -> Vec<String> {
    if side == ResolvedFlowPortSide::Input {
        effective_accepted_token_types(port)
    } else {
        Vec::new()
    }
}

/// Returns the effective alias for one node-instance port, if any.
///
/// An alias is defined only when the node stores a non-blank entry for the
/// given port side and canonical port id. Missing, unknown, or whitespace-only
/// entries must behave as though no alias exists.
fn port_alias_for_side(
    node: &FlowNode,
    side: ResolvedFlowPortSide,
    port_id: &str,
) -> Option<String> {
    let aliases = node.port_names.as_ref()?;
    let alias = match side {
        ResolvedFlowPortSide::Action => aliases.action.get(port_id),
        ResolvedFlowPortSide::Automation => aliases.automation.get(port_id),
        ResolvedFlowPortSide::Input => aliases.input.get(port_id),
        ResolvedFlowPortSide::Output => aliases.output.get(port_id),
        ResolvedFlowPortSide::Signal => aliases.signal.get(port_id),
    }?;
    let trimmed = alias.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

trait NamedFlowPort {
    fn long_description(&self) -> Option<&String>;
    fn port_id(&self) -> &str;
    fn short_description(&self) -> Option<&String>;
}

trait FromDynamicSignalPort: NamedFlowPort + Clone {
    fn from_dynamic_signal_port(name: String, group: &FlowDynamicSignalPortGroup) -> Self;
}

impl NamedFlowPort for FlowActionPortDefinition {
    fn long_description(&self) -> Option<&String> {
        self.long_description.as_ref()
    }

    fn port_id(&self) -> &str {
        &self.name
    }

    fn short_description(&self) -> Option<&String> {
        self.short_description.as_ref()
    }
}

impl FromDynamicSignalPort for FlowActionPortDefinition {
    fn from_dynamic_signal_port(name: String, group: &FlowDynamicSignalPortGroup) -> Self {
        Self {
            name,
            display_class: group.display_class,
            short_description: group.short_description.clone(),
            long_description: group.long_description.clone(),
        }
    }
}

impl NamedFlowPort for FlowSignalPortDefinition {
    fn long_description(&self) -> Option<&String> {
        self.long_description.as_ref()
    }

    fn port_id(&self) -> &str {
        &self.name
    }

    fn short_description(&self) -> Option<&String> {
        self.short_description.as_ref()
    }
}

impl FromDynamicSignalPort for FlowSignalPortDefinition {
    fn from_dynamic_signal_port(name: String, group: &FlowDynamicSignalPortGroup) -> Self {
        Self {
            name,
            display_class: group.display_class,
            short_description: group.short_description.clone(),
            long_description: group.long_description.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        effective_parameter_value, expand_ports, expanded_input_ports_for_node,
        expanded_output_ports_for_node, resolve_action_ports_for_node,
        resolve_input_ports_for_node, resolve_output_ports_for_node, resolve_signal_ports_for_node,
        token_connection_compatible, ResolvedFlowPortSide,
    };
    use crate::{
        FlowActionPortDefinition, FlowDynamicPortGroup, FlowNode, FlowNodeClass,
        FlowNodeDefinition, FlowNodePortNames, FlowParameterDataType, FlowParameterDefinition,
        FlowParameterEditor, FlowParameterEditorKind, FlowPort, FlowPortKind,
        FlowSignalPortDefinition,
    };
    use serde_json::{json, Value};
    use std::collections::{BTreeMap, HashMap};

    fn flow_port(name: &str, kind: FlowPortKind, token_type: &str) -> FlowPort {
        FlowPort {
            name: name.to_string(),
            kind,
            token_type: token_type.to_string(),
            mode: crate::FlowPortMode::FinalValue,
            phase: crate::FlowPortPhase::Execution,
            display_class: None,
            accepted_token_types: Vec::new(),
            short_description: Some(format!("short {name}")),
            long_description: Some(format!("long {name}")),
            formatter: None,
            parameters: Vec::new(),
        }
    }

    fn parameter_with_default(name: &str, default_value: Option<Value>) -> FlowParameterDefinition {
        FlowParameterDefinition {
            name: name.to_string(),
            datatype: FlowParameterDataType::named("int"),
            editor: FlowParameterEditor {
                kind: FlowParameterEditorKind::InputPortCount,
                min: None,
                max: None,
                values: Vec::new(),
                source: None,
                default_value,
            },
            controller: None,
            short_description: None,
            long_description: None,
        }
    }

    fn dynamic_group(
        count_parameter: &str,
        name_template: &str,
        token_type: &str,
    ) -> FlowDynamicPortGroup {
        FlowDynamicPortGroup {
            count_parameter: count_parameter.to_string(),
            name_template: name_template.to_string(),
            token_type: token_type.to_string(),
            mode: crate::FlowPortMode::FinalValue,
            phase: crate::FlowPortPhase::Execution,
            display_class: None,
            accepted_token_types: Vec::new(),
            short_description: Some(format!("short {name_template}")),
            long_description: Some(format!("long {name_template}")),
            parameters: Vec::new(),
        }
    }

    fn action_port(name: &str) -> FlowActionPortDefinition {
        FlowActionPortDefinition {
            name: name.to_string(),
            display_class: None,
            short_description: Some(format!("short {name}")),
            long_description: Some(format!("long {name}")),
        }
    }

    fn signal_port(name: &str) -> FlowSignalPortDefinition {
        FlowSignalPortDefinition {
            name: name.to_string(),
            display_class: None,
            short_description: Some(format!("short {name}")),
            long_description: Some(format!("long {name}")),
        }
    }

    fn node_definition() -> FlowNodeDefinition {
        FlowNodeDefinition {
            class_name: "test.node".to_string(),
            short_description: "Test node".to_string(),
            long_description: "Test node definition".to_string(),
            kind: FlowNodeClass::Inline,
            handler_id: "primrose.test".to_string(),
            interfaces: Vec::new(),
            activation_parameters: Vec::new(),
            parameters: vec![
                parameter_with_default("input_count", Some(json!(2))),
                parameter_with_default("output_count", Some(json!(1))),
                FlowParameterDefinition {
                    name: "plain".to_string(),
                    datatype: FlowParameterDataType::named("string"),
                    editor: FlowParameterEditor {
                        kind: FlowParameterEditorKind::String,
                        min: None,
                        max: None,
                        values: Vec::new(),
                        source: None,
                        default_value: Some(json!("default-value")),
                    },
                    controller: None,
                    short_description: None,
                    long_description: None,
                },
                FlowParameterDefinition {
                    name: "no_default".to_string(),
                    datatype: FlowParameterDataType::named("string"),
                    editor: FlowParameterEditor {
                        kind: FlowParameterEditorKind::String,
                        min: None,
                        max: None,
                        values: Vec::new(),
                        source: None,
                        default_value: None,
                    },
                    controller: None,
                    short_description: None,
                    long_description: None,
                },
            ],
            input_ports: vec![flow_port(
                "fixed_in",
                FlowPortKind::Input,
                "prompt.fragment",
            )],
            output_ports: vec![flow_port(
                "fixed_out",
                FlowPortKind::Output,
                "prompt.fragment",
            )],
            dynamic_input_ports: vec![
                dynamic_group("input_count", "input_{index}", "prompt.fragment"),
                dynamic_group("extra_input_count", "extra_{index}", "prompt.fragment"),
            ],
            dynamic_output_ports: vec![dynamic_group(
                "output_count",
                "output_{index}",
                "prompt.fragment",
            )],
            dynamic_action_ports: Vec::new(),
            dynamic_signal_ports: Vec::new(),
            controller_outputs: None,
            controller_actions: None,
            action_ports: vec![action_port("clear_text"), action_port("submit")],
            signal_ports: vec![signal_port("post_activate"), signal_port("before_activate")],
            automation_ports: Vec::new(),
        }
    }

    fn flow_node() -> FlowNode {
        FlowNode {
            id: "node-1".to_string(),
            definition_name: "test.node".to_string(),
            instance_name: "Node 1".to_string(),
            parameter_values: HashMap::new(),
            port_parameter_values: None,
            port_order: None,
            port_names: None,
        }
    }

    #[test]
    fn effective_parameter_value_prefers_explicit_values_then_defaults() {
        let definition = node_definition();
        let mut node = flow_node();
        node.parameter_values
            .insert("plain".to_string(), json!("explicit-value"));

        assert_eq!(
            effective_parameter_value(&node, &definition, "plain"),
            Some(json!("explicit-value"))
        );
        assert_eq!(
            effective_parameter_value(&flow_node(), &definition, "plain"),
            Some(json!("default-value"))
        );
        assert_eq!(
            effective_parameter_value(&flow_node(), &definition, "no_default"),
            None
        );
        assert_eq!(
            effective_parameter_value(&flow_node(), &definition, "missing"),
            None
        );
    }

    #[test]
    fn expanded_input_ports_for_node_returns_input_side_ports_and_dynamic_groups() {
        let definition = node_definition();
        let mut node = flow_node();
        node.parameter_values
            .insert("extra_input_count".to_string(), json!(1));
        let definitions = HashMap::from([("test.node".to_string(), definition)]);

        let ports = expanded_input_ports_for_node(&definitions, &node).expect("input ports");

        assert_eq!(
            ports
                .iter()
                .map(|port| port.name.as_str())
                .collect::<Vec<_>>(),
            vec!["fixed_in", "input_0", "input_1", "extra_0"]
        );
        assert!(ports.iter().all(|port| port.kind == FlowPortKind::Input));
        assert!(ports.iter().all(|port| port.formatter.is_none()));
    }

    #[test]
    fn expanded_output_ports_for_node_returns_output_side_ports_and_dynamic_groups() {
        let definition = node_definition();
        let definitions = HashMap::from([("test.node".to_string(), definition)]);

        let ports =
            expanded_output_ports_for_node(&definitions, &flow_node()).expect("output ports");

        assert_eq!(
            ports
                .iter()
                .map(|port| port.name.as_str())
                .collect::<Vec<_>>(),
            vec!["fixed_out", "output_0"]
        );
        assert!(ports.iter().all(|port| port.kind == FlowPortKind::Output));
        assert!(ports.iter().all(|port| port.formatter.is_none()));
    }

    #[test]
    fn expanded_port_wrappers_error_for_unknown_node_definitions() {
        let definitions = HashMap::new();
        let node = flow_node();

        let input_error = expanded_input_ports_for_node(&definitions, &node).expect_err("input");
        assert!(input_error.contains("references unknown definition"));

        let output_error = expanded_output_ports_for_node(&definitions, &node).expect_err("output");
        assert!(output_error.contains("references unknown definition"));
    }

    #[test]
    fn expand_ports_uses_defaults_and_preserves_fixed_then_dynamic_order() {
        let definition = node_definition();
        let mut node = flow_node();
        node.parameter_values
            .insert("extra_input_count".to_string(), json!(1));

        let ports = expand_ports(&node, &definition, true).expect("expanded");

        assert_eq!(ports[0].name, "fixed_in");
        assert_eq!(ports[1].name, "input_0");
        assert_eq!(ports[2].name, "input_1");
        assert_eq!(ports[3].name, "extra_0");
        assert_eq!(ports[1].token_type, "prompt.fragment");
        assert_eq!(
            ports[1].short_description.as_deref(),
            Some("short input_{index}")
        );
        assert_eq!(
            ports[1].long_description.as_deref(),
            Some("long input_{index}")
        );
        assert_eq!(ports[1].kind, FlowPortKind::Input);
        assert!(ports[1].formatter.is_none());
    }

    #[test]
    fn expand_ports_returns_only_fixed_ports_when_dynamic_count_is_zero() {
        let mut definition = node_definition();
        definition.dynamic_input_ports = vec![dynamic_group("input_count", "input_{index}", "t")];
        let mut node = flow_node();
        node.parameter_values
            .insert("input_count".to_string(), json!(0));

        let ports = expand_ports(&node, &definition, true).expect("expanded");

        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].name, "fixed_in");
    }

    #[test]
    fn expand_ports_rejects_missing_non_integer_and_negative_counts() {
        let definition = node_definition();

        let missing_error = expand_ports(&flow_node(), &definition, true).expect_err("missing");
        assert!(missing_error.contains("missing a value for dynamic port count parameter"));

        let mut non_integer_node = flow_node();
        non_integer_node
            .parameter_values
            .insert("input_count".to_string(), json!("two"));
        let non_integer_error =
            expand_ports(&non_integer_node, &definition, true).expect_err("non integer");
        assert!(non_integer_error.contains("must be an integer"));

        let mut negative_node = flow_node();
        negative_node
            .parameter_values
            .insert("input_count".to_string(), json!(-1));
        let negative_error = expand_ports(&negative_node, &definition, true).expect_err("negative");
        assert!(negative_error.contains("must be zero or greater"));
    }

    #[test]
    fn expand_ports_rejects_duplicate_names_against_fixed_and_dynamic_ports() {
        let mut duplicate_fixed_definition = node_definition();
        duplicate_fixed_definition.dynamic_input_ports =
            vec![dynamic_group("input_count", "fixed_in", "prompt.fragment")];
        let mut node = flow_node();
        node.parameter_values
            .insert("input_count".to_string(), json!(1));
        let fixed_duplicate_error =
            expand_ports(&node, &duplicate_fixed_definition, true).expect_err("fixed duplicate");
        assert!(fixed_duplicate_error.contains("expands duplicate port name \"fixed_in\""));

        let mut duplicate_dynamic_definition = node_definition();
        duplicate_dynamic_definition.input_ports = Vec::new();
        duplicate_dynamic_definition.dynamic_input_ports = vec![
            dynamic_group("input_count", "shared_{index}", "prompt.fragment"),
            dynamic_group("extra_input_count", "shared_0", "prompt.fragment"),
        ];
        let mut duplicate_dynamic_node = flow_node();
        duplicate_dynamic_node
            .parameter_values
            .insert("input_count".to_string(), json!(1));
        duplicate_dynamic_node
            .parameter_values
            .insert("extra_input_count".to_string(), json!(1));
        let dynamic_duplicate_error =
            expand_ports(&duplicate_dynamic_node, &duplicate_dynamic_definition, true)
                .expect_err("dynamic duplicate");
        assert!(dynamic_duplicate_error.contains("expands duplicate port name \"shared_0\""));
    }

    #[test]
    fn resolve_token_ports_preserve_canonical_ids_and_apply_aliases_only_to_labels() {
        let definition = node_definition();
        let definitions = HashMap::from([("test.node".to_string(), definition)]);
        let mut node = flow_node();
        node.parameter_values
            .insert("extra_input_count".to_string(), json!(1));
        node.port_names = Some(FlowNodePortNames {
            action: BTreeMap::new(),
            automation: BTreeMap::new(),
            input: BTreeMap::from([
                ("fixed_in".to_string(), "  ".to_string()),
                ("input_1".to_string(), "Prompt".to_string()),
                ("ghost".to_string(), "Ghost".to_string()),
            ]),
            output: BTreeMap::from([("fixed_out".to_string(), "Combined".to_string())]),
            signal: BTreeMap::new(),
        });

        let input_ports = resolve_input_ports_for_node(&definitions, &node).expect("input ports");
        let output_ports =
            resolve_output_ports_for_node(&definitions, &node).expect("output ports");

        assert_eq!(
            input_ports
                .iter()
                .map(|port| port.id.as_str())
                .collect::<Vec<_>>(),
            vec!["fixed_in", "input_0", "input_1", "extra_0"]
        );
        assert_eq!(
            input_ports
                .iter()
                .map(|port| port.label.as_str())
                .collect::<Vec<_>>(),
            vec!["fixed_in", "input_0", "Prompt", "extra_0"]
        );
        assert_eq!(
            input_ports
                .iter()
                .map(|port| port.name.clone())
                .collect::<Vec<_>>(),
            vec![None, None, Some("Prompt".to_string()), None]
        );
        assert!(input_ports
            .iter()
            .all(|port| port.side == ResolvedFlowPortSide::Input));
        assert!(input_ports
            .iter()
            .all(|port| port.token_type.as_deref() == Some("prompt.fragment")));

        assert_eq!(
            output_ports
                .iter()
                .map(|port| (port.id.as_str(), port.label.as_str(), port.name.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("fixed_out", "Combined", Some("Combined")),
                ("output_0", "output_0", None),
            ]
        );
        assert!(output_ports
            .iter()
            .all(|port| port.side == ResolvedFlowPortSide::Output));
    }

    #[test]
    fn resolve_action_and_signal_ports_preserve_order_identity_and_descriptions() {
        let definition = node_definition();
        let definitions = HashMap::from([("test.node".to_string(), definition)]);
        let mut node = flow_node();
        node.port_names = Some(FlowNodePortNames {
            action: BTreeMap::from([
                ("clear_text".to_string(), "Clear Text".to_string()),
                ("submit".to_string(), "   ".to_string()),
            ]),
            automation: BTreeMap::new(),
            input: BTreeMap::new(),
            output: BTreeMap::new(),
            signal: BTreeMap::from([("post_activate".to_string(), "After Activate".to_string())]),
        });

        let action_ports =
            resolve_action_ports_for_node(&definitions, &node).expect("action ports");
        let signal_ports =
            resolve_signal_ports_for_node(&definitions, &node).expect("signal ports");

        assert_eq!(
            action_ports
                .iter()
                .map(|port| (port.id.as_str(), port.label.as_str(), port.name.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("clear_text", "Clear Text", Some("Clear Text")),
                ("submit", "submit", None),
            ]
        );
        assert_eq!(
            action_ports[0].short_description.as_deref(),
            Some("short clear_text")
        );
        assert_eq!(
            action_ports[0].long_description.as_deref(),
            Some("long clear_text")
        );
        assert!(action_ports
            .iter()
            .all(|port| port.side == ResolvedFlowPortSide::Action && port.token_type.is_none()));

        assert_eq!(
            signal_ports
                .iter()
                .map(|port| (port.id.as_str(), port.label.as_str(), port.name.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("post_activate", "After Activate", Some("After Activate")),
                ("before_activate", "before_activate", None),
            ]
        );
        assert!(signal_ports
            .iter()
            .all(|port| port.side == ResolvedFlowPortSide::Signal && port.token_type.is_none()));
    }

    #[test]
    fn resolve_port_wrappers_error_for_unknown_node_definitions() {
        let definitions = HashMap::new();
        let node = flow_node();

        assert!(resolve_input_ports_for_node(&definitions, &node)
            .expect_err("input")
            .contains("references unknown definition"));
        assert!(resolve_output_ports_for_node(&definitions, &node)
            .expect_err("output")
            .contains("references unknown definition"));
        assert!(resolve_action_ports_for_node(&definitions, &node)
            .expect_err("action")
            .contains("references unknown definition"));
        assert!(resolve_signal_ports_for_node(&definitions, &node)
            .expect_err("signal")
            .contains("references unknown definition"));
    }

    // Cross-language contract: the engine's `token_connection_compatible`
    // and the editor's TS `tokenConnectionCompatible` must agree on every
    // row of the shared fixture. The TS suite has the mirror of this test
    // (gui/apps/engine-editor/tests/dataflow/flow-node-ports.test.ts). If the
    // two implementations ever diverge, one of these two tests fails.
    #[test]
    fn token_connection_compatible_matches_the_shared_fixture() {
        #[derive(serde::Deserialize)]
        struct Case {
            name: String,
            source_token_type: String,
            source_node_input_accepted: Vec<String>,
            target_accepted: Vec<String>,
            expected: bool,
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            cases: Vec<Case>,
        }

        let fixture: Fixture =
            serde_json::from_str(include_str!("../fixtures/token-type-compatibility.json"))
                .expect("shared token-type-compatibility fixture must parse");
        assert!(
            !fixture.cases.is_empty(),
            "fixture must contain at least one case"
        );
        for case in &fixture.cases {
            assert_eq!(
                token_connection_compatible(
                    &case.source_token_type,
                    &case.source_node_input_accepted,
                    &case.target_accepted,
                ),
                case.expected,
                "shared fixture case \"{}\" disagrees with the engine rule",
                case.name
            );
        }
    }
}
