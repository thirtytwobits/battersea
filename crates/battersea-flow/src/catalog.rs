//! Copyright (c) Scott A Dixon

use crate::registry::Registry;
use crate::{
    FlowActionPortDefinition, FlowAutomationPortDefinition, FlowControllerActionDefinition,
    FlowControllerOutputDefinition, FlowDynamicPortGroup, FlowDynamicSignalPortGroup, FlowNode,
    FlowNodeClass, FlowNodeDefinition, FlowParameterDataType, FlowParameterDefinition,
    FlowParameterEditor, FlowParameterEditorKind, FlowPort, FlowPortDisplayClass,
    FlowPortFormatter, FlowPortFormatterKind, FlowPortKind, FlowPromptTemplateDefinition,
    FlowSignalPortDefinition, FLOW_SIGNAL_POST_ACTIVATE,
};
use anyhow::{ensure, Context};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};

use super::validation::validate_parameter_value;
use crate::template_engine::validate_flow_port_formatter;

/// Names the interface that marks a source node as directly activatable by the engine.
pub const FLOW_NODE_ACTIVATE_INTERFACE: &str = "IFlowNodeActivate";
pub const FLOW_INSTRUMENT_INTERFACE: &str = "IFlowInstrument";
pub const FLOW_ACTION_DISABLE: &str = "disable";

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FlowManifestParameterControl {
    #[serde(default)]
    kind: Option<FlowParameterEditorKind>,
    #[serde(default)]
    min: Option<u32>,
    #[serde(default)]
    max: Option<u32>,
    #[serde(default)]
    values: Vec<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    default_value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FlowManifestParameterDefinition {
    name: String,
    datatype: FlowParameterDataType,
    editor: FlowManifestParameterControl,
    #[serde(default)]
    controller: Option<FlowManifestParameterControl>,
    #[serde(default)]
    short_description: Option<String>,
    #[serde(default)]
    long_description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FlowManifestFormatterRef {
    #[serde(rename = "ref")]
    ref_key: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
enum FlowManifestFormatterDefinition {
    Ref(FlowManifestFormatterRef),
    Inline(FlowPromptTemplateDefinition),
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FlowManifestPortFormatter {
    kind: FlowPortFormatterKind,
    #[serde(default)]
    default: Option<FlowManifestFormatterDefinition>,
    #[serde(default)]
    variants: BTreeMap<String, FlowManifestFormatterDefinition>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FlowManifestPort {
    name: String,
    kind: FlowPortKind,
    token_type: String,
    #[serde(default)]
    display_class: Option<FlowPortDisplayClass>,
    #[serde(default)]
    accepted_token_types: Vec<String>,
    #[serde(default)]
    short_description: Option<String>,
    #[serde(default)]
    long_description: Option<String>,
    #[serde(default)]
    formatter: Option<FlowManifestPortFormatter>,
    #[serde(default)]
    parameters: Vec<FlowManifestParameterDefinition>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FlowManifestNodeDefinition {
    class_name: String,
    short_description: String,
    long_description: String,
    kind: FlowNodeClass,
    handler_id: String,
    #[serde(default)]
    interfaces: Vec<String>,
    #[serde(default)]
    activation_parameters: Vec<String>,
    #[serde(default)]
    parameters: Vec<FlowManifestParameterDefinition>,
    #[serde(default)]
    input_ports: Vec<FlowManifestPort>,
    #[serde(default)]
    output_ports: Vec<FlowManifestPort>,
    #[serde(default)]
    dynamic_input_ports: Vec<FlowDynamicPortGroup>,
    #[serde(default)]
    dynamic_output_ports: Vec<FlowDynamicPortGroup>,
    #[serde(default)]
    dynamic_action_ports: Vec<FlowDynamicSignalPortGroup>,
    #[serde(default)]
    dynamic_signal_ports: Vec<FlowDynamicSignalPortGroup>,
    #[serde(default)]
    controller_outputs: Vec<FlowControllerOutputDefinition>,
    #[serde(default)]
    controller_actions: Vec<FlowControllerActionDefinition>,
    #[serde(default)]
    action_ports: Vec<FlowActionPortDefinition>,
    #[serde(default)]
    signal_ports: Vec<FlowSignalPortDefinition>,
    #[serde(default)]
    automation_ports: Vec<FlowAutomationPortDefinition>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FlowManifestDocument {
    #[serde(default)]
    formatter_blocks: BTreeMap<String, FlowPromptTemplateDefinition>,
    node_definitions: Vec<FlowManifestNodeDefinition>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
enum FlowManifestRoot {
    Document(FlowManifestDocument),
    LegacyList(Vec<FlowManifestNodeDefinition>),
}

struct FlowManifestCatalog {
    formatter_blocks: BTreeMap<String, FlowPromptTemplateDefinition>,
    node_definitions: Vec<FlowManifestNodeDefinition>,
}

impl From<FlowManifestRoot> for FlowManifestCatalog {
    fn from(root: FlowManifestRoot) -> Self {
        match root {
            FlowManifestRoot::Document(document) => Self {
                formatter_blocks: document.formatter_blocks,
                node_definitions: document.node_definitions,
            },
            FlowManifestRoot::LegacyList(node_definitions) => Self {
                formatter_blocks: BTreeMap::new(),
                node_definitions,
            },
        }
    }
}

fn builtin_post_activate_signal_port() -> FlowSignalPortDefinition {
    FlowSignalPortDefinition {
        name: FLOW_SIGNAL_POST_ACTIVATE.to_string(),
        display_class: None,
        short_description: Some("Fires after the node completes activation.".to_string()),
        long_description: Some(
            "Emits after the node has delivered all ordinary outputs for a successful activation."
                .to_string(),
        ),
    }
}

fn builtin_disable_action_port() -> FlowActionPortDefinition {
    FlowActionPortDefinition {
        name: FLOW_ACTION_DISABLE.to_string(),
        display_class: None,
        short_description: Some("Disable this node for the current activation.".to_string()),
        long_description: Some(
            "When triggered, this node stops emitting token and signal outputs for the remainder of the current activation."
                .to_string(),
        ),
    }
}

fn with_builtin_action_ports(mut entry: FlowNodeDefinition) -> FlowNodeDefinition {
    let output_capable = !entry.output_ports.is_empty() || !entry.dynamic_output_ports.is_empty();
    if output_capable
        && !entry
            .action_ports
            .iter()
            .any(|port| port.name == FLOW_ACTION_DISABLE)
    {
        entry.action_ports.push(builtin_disable_action_port());
    }
    entry
}

fn with_builtin_signal_ports(mut entry: FlowNodeDefinition) -> FlowNodeDefinition {
    let is_instrument = entry.kind == FlowNodeClass::Instrument
        || entry
            .interfaces
            .iter()
            .any(|interface| interface == FLOW_INSTRUMENT_INTERFACE);
    if !is_instrument && entry.kind != FlowNodeClass::Control && entry.kind != FlowNodeClass::Logic
    {
        entry.signal_ports.push(builtin_post_activate_signal_port());
    }
    entry
}

fn normalize_manifest_parameter_control(
    registry: &Registry,
    control: FlowManifestParameterControl,
    datatype: &FlowParameterDataType,
    parameter_name: &str,
    control_name: &str,
) -> Result<FlowParameterEditor, String> {
    let kind = match control.kind.or_else(|| registry.default_editor(datatype)) {
        Some(kind) => kind,
        None => {
            return Err(format!(
                "definition parameter \"{}\" {}.kind must be provided when datatype.kind={}",
                parameter_name, control_name, datatype.kind
            ))
        }
    };

    Ok(FlowParameterEditor {
        kind,
        min: control.min,
        max: control.max,
        values: control.values,
        source: control.source,
        default_value: control.default_value,
    })
}

fn normalize_manifest_parameter_definition(
    registry: &Registry,
    parameter: FlowManifestParameterDefinition,
) -> Result<FlowParameterDefinition, String> {
    let editor = normalize_manifest_parameter_control(
        registry,
        parameter.editor,
        &parameter.datatype,
        &parameter.name,
        "editor",
    )?;
    let controller = parameter
        .controller
        .map(|controller| {
            normalize_manifest_parameter_control(
                registry,
                controller,
                &parameter.datatype,
                &parameter.name,
                "controller",
            )
        })
        .transpose()?;

    Ok(FlowParameterDefinition {
        name: parameter.name,
        datatype: parameter.datatype,
        editor,
        controller,
        short_description: parameter.short_description,
        long_description: parameter.long_description,
    })
}

fn normalize_manifest_formatter_definition(
    definition: FlowManifestFormatterDefinition,
    formatter_blocks: &BTreeMap<String, FlowPromptTemplateDefinition>,
) -> Result<FlowPromptTemplateDefinition, String> {
    match definition {
        FlowManifestFormatterDefinition::Inline(definition) => Ok(definition),
        FlowManifestFormatterDefinition::Ref(reference) => {
            let ref_key = reference.ref_key.trim();
            if ref_key.is_empty() {
                return Err("flow formatter references must not be blank".to_string());
            }
            formatter_blocks.get(ref_key).cloned().ok_or_else(|| {
                format!(
                    "flow formatter reference \"{}\" does not match a formatter block",
                    ref_key
                )
            })
        }
    }
}

fn normalize_manifest_port_formatter(
    formatter: FlowManifestPortFormatter,
    formatter_blocks: &BTreeMap<String, FlowPromptTemplateDefinition>,
) -> Result<FlowPortFormatter, String> {
    Ok(FlowPortFormatter {
        kind: formatter.kind,
        default: formatter
            .default
            .map(|definition| normalize_manifest_formatter_definition(definition, formatter_blocks))
            .transpose()?,
        variants: formatter
            .variants
            .into_iter()
            .map(|(key, definition)| {
                normalize_manifest_formatter_definition(definition, formatter_blocks)
                    .map(|definition| (key, definition))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?,
    })
}

fn normalize_manifest_port(
    registry: &Registry,
    port: FlowManifestPort,
    formatter_blocks: &BTreeMap<String, FlowPromptTemplateDefinition>,
) -> Result<FlowPort, String> {
    Ok(FlowPort {
        name: port.name,
        kind: port.kind,
        token_type: port.token_type,
        display_class: port.display_class,
        accepted_token_types: port.accepted_token_types,
        short_description: port.short_description,
        long_description: port.long_description,
        formatter: port
            .formatter
            .map(|formatter| normalize_manifest_port_formatter(formatter, formatter_blocks))
            .transpose()?,
        parameters: port
            .parameters
            .into_iter()
            .map(|parameter| normalize_manifest_parameter_definition(registry, parameter))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn normalize_manifest_definition_with_blocks(
    registry: &Registry,
    entry: FlowManifestNodeDefinition,
    formatter_blocks: &BTreeMap<String, FlowPromptTemplateDefinition>,
) -> Result<FlowNodeDefinition, String> {
    Ok(FlowNodeDefinition {
        class_name: entry.class_name,
        short_description: entry.short_description,
        long_description: entry.long_description,
        kind: entry.kind,
        handler_id: entry.handler_id,
        interfaces: entry.interfaces,
        activation_parameters: entry.activation_parameters,
        parameters: entry
            .parameters
            .into_iter()
            .map(|parameter| normalize_manifest_parameter_definition(registry, parameter))
            .collect::<Result<Vec<_>, _>>()?,
        input_ports: entry
            .input_ports
            .into_iter()
            .map(|port| normalize_manifest_port(registry, port, formatter_blocks))
            .collect::<Result<Vec<_>, _>>()?,
        output_ports: entry
            .output_ports
            .into_iter()
            .map(|port| normalize_manifest_port(registry, port, formatter_blocks))
            .collect::<Result<Vec<_>, _>>()?,
        dynamic_input_ports: entry.dynamic_input_ports,
        dynamic_output_ports: entry.dynamic_output_ports,
        dynamic_action_ports: entry.dynamic_action_ports,
        dynamic_signal_ports: entry.dynamic_signal_ports,
        controller_outputs: (!entry.controller_outputs.is_empty())
            .then_some(entry.controller_outputs),
        controller_actions: (!entry.controller_actions.is_empty())
            .then_some(entry.controller_actions),
        action_ports: entry.action_ports,
        signal_ports: entry.signal_ports,
        automation_ports: entry.automation_ports,
    })
}

/// Parse an explicit manifest for host composition before `Catalog` validates the combined definitions.
pub fn parse_definition_manifest(
    registry: &Registry,
    source: &str,
) -> Result<Vec<FlowNodeDefinition>, String> {
    let root: FlowManifestRoot = serde_yaml::from_str(source).map_err(|error| error.to_string())?;
    let catalog = FlowManifestCatalog::from(root);

    catalog
        .node_definitions
        .into_iter()
        .map(|definition| {
            normalize_manifest_definition_with_blocks(
                registry,
                definition,
                &catalog.formatter_blocks,
            )
        })
        .collect()
}

/// Immutable definitions validated against an explicit host registry.
pub struct Catalog {
    order: Vec<String>,
    definitions: HashMap<String, FlowNodeDefinition>,
    registry: Registry,
}
impl Catalog {
    pub fn from_manifest(source: &str, registry: Registry) -> anyhow::Result<Self> {
        let entries = parse_definition_manifest(&registry, source).map_err(anyhow::Error::msg)?;
        Self::from_definitions(entries, registry)
    }
    pub fn from_manifests(sources: &[(&str, &str)], registry: Registry) -> anyhow::Result<Self> {
        let mut entries = Vec::new();
        for (name, source) in sources {
            entries.extend(
                parse_definition_manifest(&registry, source)
                    .map_err(|error| anyhow::anyhow!("{name}: {error}"))?,
            );
        }
        Self::from_definitions(entries, registry)
    }
    pub fn from_definitions(
        entries: Vec<FlowNodeDefinition>,
        registry: Registry,
    ) -> anyhow::Result<Self> {
        validate_definition_manifest(&registry, &entries)?;
        let order = entries
            .iter()
            .map(|entry| entry.class_name.clone())
            .collect();
        let definitions = entries
            .into_iter()
            .map(with_builtin_action_ports)
            .map(with_builtin_signal_ports)
            .map(|entry| (entry.class_name.clone(), entry))
            .collect();
        Ok(Self {
            order,
            definitions,
            registry,
        })
    }
    /// Definitions in their manifest order.
    pub fn entries(&self) -> impl Iterator<Item = &FlowNodeDefinition> {
        self.order.iter().map(|name| &self.definitions[name])
    }
    pub fn definitions(&self) -> &HashMap<String, FlowNodeDefinition> {
        &self.definitions
    }
    pub fn validate(&self, flow: &crate::FlowDocument) -> crate::FlowValidationResult {
        crate::validation::validate_document(flow, &self.definitions, &self.registry)
    }
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
    pub fn into_definitions(self) -> HashMap<String, FlowNodeDefinition> {
        self.definitions
    }
}
/// Resolves the node definition referenced by a flow node.
pub fn node_definition<'a>(
    definitions: &'a HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Option<&'a FlowNodeDefinition> {
    definitions.get(node.definition_name.as_str())
}

/// Returns whether a node definition declares the requested interface.
pub fn node_implements_interface(
    definitions: &HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
    interface_name: &str,
) -> bool {
    node_definition(definitions, node).is_some_and(|definition| {
        definition
            .interfaces
            .iter()
            .any(|interface| interface == interface_name)
    })
}

/// Performs startup-time validation of the bundled flow-definition manifest.
fn validate_definition_manifest(
    registry: &Registry,
    entries: &[FlowNodeDefinition],
) -> anyhow::Result<()> {
    let mut class_names = HashSet::new();

    for entry in entries {
        ensure!(
            !entry.class_name.trim().is_empty(),
            "flow manifest definitions must have a non-empty class_name"
        );
        ensure!(
            !entry.handler_id.trim().is_empty(),
            "flow manifest definitions must have a non-empty handler_id"
        );
        ensure!(
            registry.has_handler(&entry.handler_id),
            "flow manifest definitions must reference a registered handler_id"
        );
        ensure!(
            class_names.insert(entry.class_name.clone()),
            "flow manifest must not contain duplicate class_name values"
        );

        let mut interface_names = HashSet::new();
        for interface in &entry.interfaces {
            ensure!(
                !interface.trim().is_empty(),
                "flow manifest interfaces must have non-empty names"
            );
            ensure!(
                interface_names.insert(interface.clone()),
                "flow manifest must not contain duplicate interfaces on a definition"
            );
        }

        let mut parameter_names = HashSet::new();
        for parameter in &entry.parameters {
            ensure!(
                !parameter.name.trim().is_empty(),
                "definition parameters must have non-empty names"
            );
            ensure!(
                parameter_names.insert(parameter.name.clone()),
                "definition parameter names must be unique"
            );
            validate_parameter_definition(registry, parameter)?;
        }
        let mut activation_parameter_names = HashSet::new();
        for parameter_name in &entry.activation_parameters {
            ensure!(
                !parameter_name.trim().is_empty(),
                "definition activation_parameters must have non-empty names"
            );
            ensure!(
                activation_parameter_names.insert(parameter_name.clone()),
                "definition activation_parameters must be unique"
            );
        }

        let fixed_input_names = validate_fixed_ports(
            registry,
            &entry.class_name,
            &entry.input_ports,
            FlowPortKind::Input,
        )?;
        let fixed_output_names = validate_fixed_ports(
            registry,
            &entry.class_name,
            &entry.output_ports,
            FlowPortKind::Output,
        )?;
        validate_controller_outputs(entry)?;
        validate_controller_actions(entry)?;
        validate_action_ports(entry)?;
        validate_signal_ports(entry)?;
        validate_automation_ports(entry)?;
        registry.validate_definition(entry)?;
        for (name, accepted) in entry
            .input_ports
            .iter()
            .chain(&entry.output_ports)
            .map(|port| (&port.token_type, &port.accepted_token_types))
            .chain(
                entry
                    .dynamic_input_ports
                    .iter()
                    .chain(&entry.dynamic_output_ports)
                    .map(|port| (&port.token_type, &port.accepted_token_types)),
            )
            .chain(
                entry
                    .automation_ports
                    .iter()
                    .map(|port| (&port.token_type, &port.accepted_token_types)),
            )
        {
            ensure!(registry.has_token(name), "Unknown token type {name}");
            for name in accepted {
                ensure!(
                    registry.has_token(name) && name != "auto" && name != "oneof",
                    "Unknown concrete token type {name}"
                );
            }
        }
        for group in entry
            .dynamic_input_ports
            .iter()
            .chain(&entry.dynamic_output_ports)
        {
            for parameter in &group.parameters {
                validate_parameter_definition(registry, parameter)?;
            }
        }

        for group in &entry.dynamic_input_ports {
            validate_dynamic_port_group(
                group,
                &entry.parameters,
                FlowParameterEditorKind::InputPortCount,
                &fixed_input_names,
            )?;
        }
        for group in &entry.dynamic_output_ports {
            validate_dynamic_port_group(
                group,
                &entry.parameters,
                FlowParameterEditorKind::OutputPortCount,
                &fixed_output_names,
            )?;
        }
        let fixed_action_names = validate_named_ports(&entry.action_ports, "action")?;
        let fixed_signal_names = validate_named_ports(&entry.signal_ports, "signal")?;
        for group in &entry.dynamic_action_ports {
            validate_dynamic_signal_port_group(
                group,
                &entry.parameters,
                FlowParameterEditorKind::InputPortCount,
                &fixed_action_names,
            )?;
        }
        for group in &entry.dynamic_signal_ports {
            validate_dynamic_signal_port_group(
                group,
                &entry.parameters,
                FlowParameterEditorKind::OutputPortCount,
                &fixed_signal_names,
            )?;
        }

        let input_count = entry.input_ports.len() + entry.dynamic_input_ports.len();
        let output_count = entry.output_ports.len() + entry.dynamic_output_ports.len();
        let action_count = entry.action_ports.len() + entry.dynamic_action_ports.len();
        let signal_count = entry.signal_ports.len() + entry.dynamic_signal_ports.len();
        let is_instrument = entry
            .interfaces
            .iter()
            .any(|interface| interface == FLOW_INSTRUMENT_INTERFACE);
        match entry.kind {
            FlowNodeClass::Source => {
                ensure!(
                    input_count == 0,
                    "source definitions must not declare input ports"
                );
                ensure!(
                    output_count > 0,
                    "source definitions must declare at least one output port"
                );
            }
            FlowNodeClass::Control => {
                ensure!(
                    input_count == 0,
                    "control definitions must not declare token input ports"
                );
                ensure!(
                    output_count == 0,
                    "control definitions must not declare token output ports"
                );
                ensure!(
                    entry.action_ports.is_empty(),
                    "control definitions must not declare graph action ports"
                );
                ensure!(
                    entry.signal_ports.is_empty(),
                    "control definitions must not declare graph signal ports"
                );
                ensure!(
                    entry
                        .controller_actions
                        .as_ref()
                        .is_some_and(|actions| !actions.is_empty()),
                    "control definitions must declare at least one controller action"
                );
            }
            FlowNodeClass::Hybrid => {
                if is_instrument {
                    ensure!(
                        !entry.action_ports.is_empty(),
                        "instrument hybrid definitions must declare at least one action port"
                    );
                    ensure!(
                        !entry.signal_ports.is_empty(),
                        "instrument hybrid definitions must declare at least one signal port"
                    );
                } else {
                    ensure!(
                        input_count > 0,
                        "hybrid definitions must declare at least one input port"
                    );
                    ensure!(
                        output_count > 0,
                        "hybrid definitions must declare at least one output port"
                    );
                }
            }
            FlowNodeClass::Instrument => {
                ensure!(
                    input_count == 0,
                    "instrument definitions must not declare token input ports"
                );
                ensure!(
                    output_count == 0,
                    "instrument definitions must not declare token output ports"
                );
                ensure!(
                    !entry.action_ports.is_empty(),
                    "instrument definitions must declare at least one action port"
                );
                ensure!(
                    !entry.signal_ports.is_empty(),
                    "instrument definitions must declare at least one signal port"
                );
            }
            FlowNodeClass::Logic => {
                ensure!(
                    input_count == 0,
                    "logic definitions must not declare token input ports"
                );
                ensure!(
                    output_count == 0,
                    "logic definitions must not declare token output ports"
                );
                ensure!(
                    action_count > 0,
                    "logic definitions must declare at least one action port"
                );
                ensure!(
                    signal_count > 0,
                    "logic definitions must declare at least one signal port"
                );
            }
            FlowNodeClass::Inline => {
                ensure!(
                    input_count > 0,
                    "inline definitions must declare at least one input port"
                );
                ensure!(
                    output_count > 0,
                    "inline definitions must declare at least one output port"
                );
            }
            FlowNodeClass::Sink => {
                ensure!(
                    input_count > 0,
                    "sink definitions must declare at least one input port"
                );
                ensure!(
                    output_count == 0,
                    "sink definitions must not declare output ports"
                );
            }
        }
    }
    Ok(())
}

/// Validates a set of fixed ports and returns their declared names.
fn validate_fixed_ports(
    registry: &Registry,
    class_name: &str,
    ports: &[FlowPort],
    expected_kind: FlowPortKind,
) -> anyhow::Result<HashSet<String>> {
    let mut names = HashSet::new();
    for port in ports {
        ensure!(
            !port.name.trim().is_empty(),
            "flow port names must be non-empty in the manifest"
        );
        ensure!(
            !port.token_type.trim().is_empty(),
            "flow ports must have non-empty token_type values"
        );
        ensure!(
            port.accepted_token_types
                .iter()
                .all(|token_type| !token_type.trim().is_empty()),
            "flow ports must not declare blank accepted_token_types"
        );
        ensure!(
            port.kind == expected_kind,
            "flow ports must match the expected port kind"
        );
        // The "oneof" marker means "accept any one of the listed types" — the
        // port must enumerate at least one variant, otherwise nothing matches.
        // Valid on either side: inputs accept any of the listed types from an
        // upstream emitter; outputs are allowed to emit any of the listed types
        // (the runtime emit check honours `accepted_token_types` for sources).
        if port.token_type == "oneof" {
            ensure!(
                !port.accepted_token_types.is_empty(),
                "definition \"{}\" port \"{}\" declares token_type \"oneof\" but has no accepted_token_types",
                class_name,
                port.name
            );
        }
        // The "auto" marker means "type adopted at runtime from whichever
        // input drove the host node." Outputs only — an auto input has no
        // upstream signal to derive its type from.
        if port.token_type == "auto" {
            ensure!(
                expected_kind == FlowPortKind::Output,
                "definition \"{}\" port \"{}\" uses \"auto\", which is only valid on output ports",
                class_name,
                port.name
            );
            ensure!(
                port.accepted_token_types.is_empty(),
                "definition \"{}\" port \"{}\" uses \"auto\"; accepted_token_types must be empty",
                class_name,
                port.name
            );
        }
        ensure!(
            names.insert(port.name.clone()),
            "flow port names must be unique within a definition"
        );
        if let Some(formatter) = &port.formatter {
            ensure!(
                expected_kind == FlowPortKind::Output,
                "only output ports may declare formatter metadata"
            );
            validate_flow_port_formatter(formatter)
                .with_context(|| format!("Invalid formatter on {}.{}", class_name, port.name))?;
        }
        let mut parameter_names = HashSet::new();
        for parameter in &port.parameters {
            ensure!(
                !parameter.name.trim().is_empty(),
                "flow port parameters must have non-empty names"
            );
            ensure!(
                parameter_names.insert(parameter.name.clone()),
                "flow port parameter names must be unique within a port"
            );
            validate_parameter_definition(registry, parameter)?;
        }
    }
    Ok(names)
}

fn validate_action_ports(entry: &FlowNodeDefinition) -> anyhow::Result<()> {
    validate_named_ports(&entry.action_ports, "action")?;
    Ok(())
}

fn validate_controller_actions(entry: &FlowNodeDefinition) -> anyhow::Result<()> {
    let mut names = HashSet::new();
    let mut kinds = HashSet::new();
    for action in entry.controller_actions.as_deref().unwrap_or(&[]) {
        ensure!(
            !action.name.trim().is_empty(),
            "flow controller action names must be non-empty in the manifest"
        );
        ensure!(
            names.insert(action.name.clone()),
            "flow controller action names must be unique within a definition"
        );
        ensure!(
            !action.kind.trim().is_empty(),
            "flow controller action kinds must be non-empty in the manifest"
        );
        ensure!(
            kinds.insert(action.kind.clone()),
            "flow controller action kinds must be unique within a definition"
        );
    }
    Ok(())
}

fn validate_controller_outputs(entry: &FlowNodeDefinition) -> anyhow::Result<()> {
    let mut names = HashSet::new();
    for output in entry.controller_outputs.as_deref().unwrap_or(&[]) {
        ensure!(
            !output.name.trim().is_empty(),
            "flow controller output names must be non-empty in the manifest"
        );
        ensure!(
            names.insert(output.name.clone()),
            "flow controller output names must be unique within a definition"
        );
        ensure!(
            !output.kind.trim().is_empty(),
            "Controller output kinds must have names"
        );
    }
    Ok(())
}

fn validate_signal_ports(entry: &FlowNodeDefinition) -> anyhow::Result<()> {
    validate_named_ports(&entry.signal_ports, "signal")?;
    Ok(())
}

fn validate_automation_ports(entry: &FlowNodeDefinition) -> anyhow::Result<()> {
    validate_named_ports(&entry.automation_ports, "automation")?;
    let parameter_names: HashSet<&str> = entry
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect();
    for port in &entry.automation_ports {
        ensure!(
            !port.parameter_name.trim().is_empty(),
            "flow automation port \"{}\" on \"{}\" must declare a non-empty parameter_name",
            port.name,
            entry.class_name
        );
        ensure!(
            parameter_names.contains(port.parameter_name.as_str()),
            "flow automation port \"{}\" on \"{}\" targets unknown parameter \"{}\"",
            port.name,
            entry.class_name,
            port.parameter_name
        );
        ensure!(
            !port.token_type.trim().is_empty(),
            "flow automation port \"{}\" on \"{}\" must declare a non-empty token_type",
            port.name,
            entry.class_name
        );
    }
    Ok(())
}

fn validate_named_ports<T>(ports: &[T], side: &str) -> anyhow::Result<HashSet<String>>
where
    T: ManifestNamedPort,
{
    let mut names = HashSet::new();
    for port in ports {
        let name = port.name();
        ensure!(
            !name.trim().is_empty(),
            "flow {} port names must be non-empty in the manifest",
            side
        );
        if side == "signal" {
            ensure!(
                name != FLOW_SIGNAL_POST_ACTIVATE,
                "flow definitions must not declare the reserved post_activate signal explicitly"
            );
        }
        ensure!(
            names.insert(name.to_string()),
            "flow {} port names must be unique within a definition",
            side
        );
    }
    Ok(names)
}

trait ManifestNamedPort {
    fn name(&self) -> &str;
}

impl ManifestNamedPort for FlowActionPortDefinition {
    fn name(&self) -> &str {
        &self.name
    }
}

impl ManifestNamedPort for FlowSignalPortDefinition {
    fn name(&self) -> &str {
        &self.name
    }
}

impl ManifestNamedPort for FlowAutomationPortDefinition {
    fn name(&self) -> &str {
        &self.name
    }
}

/// Validates one dynamic port group against the parameter list and fixed ports.
fn validate_dynamic_port_group(
    group: &FlowDynamicPortGroup,
    parameters: &[FlowParameterDefinition],
    expected_editor_kind: FlowParameterEditorKind,
    fixed_names: &HashSet<String>,
) -> anyhow::Result<()> {
    ensure!(
        !group.count_parameter.trim().is_empty(),
        "dynamic port groups must reference a count_parameter"
    );
    ensure!(
        !group.name_template.trim().is_empty(),
        "dynamic port groups must declare a name_template"
    );
    ensure!(
        group.name_template.contains("{index}"),
        "dynamic port group name_template values must contain {{index}}"
    );
    ensure!(
        !group.token_type.trim().is_empty(),
        "dynamic port groups must declare a token_type"
    );
    ensure!(
        group
            .accepted_token_types
            .iter()
            .all(|token_type| !token_type.trim().is_empty()),
        "dynamic port groups must not declare blank accepted_token_types"
    );
    if group.token_type == "oneof" {
        ensure!(
            !group.accepted_token_types.is_empty(),
            "dynamic port group \"{}\" declares token_type \"oneof\" but has no accepted_token_types",
            group.name_template
        );
        ensure!(
            expected_editor_kind == FlowParameterEditorKind::InputPortCount,
            "dynamic port group \"{}\" uses \"oneof\", which is only valid on input ports",
            group.name_template
        );
    }
    if group.token_type == "auto" {
        ensure!(
            expected_editor_kind == FlowParameterEditorKind::OutputPortCount,
            "dynamic port group \"{}\" uses \"auto\", which is only valid on output ports",
            group.name_template
        );
        ensure!(
            group.accepted_token_types.is_empty(),
            "dynamic port group \"{}\" uses \"auto\"; accepted_token_types must be empty",
            group.name_template
        );
    }
    ensure!(
        !fixed_names.contains(&group.name_template),
        "dynamic port templates must not duplicate fixed port names"
    );

    let parameter = parameters
        .iter()
        .find(|parameter| parameter.name == group.count_parameter)
        .ok_or_else(|| anyhow::anyhow!("Unknown count parameter {}", group.count_parameter))?;
    ensure!(
        parameter.editor.kind == expected_editor_kind,
        "dynamic port groups must bind to the matching port-count parameter kind"
    );
    Ok(())
}

fn validate_dynamic_signal_port_group(
    group: &FlowDynamicSignalPortGroup,
    parameters: &[FlowParameterDefinition],
    expected_editor_kind: FlowParameterEditorKind,
    fixed_names: &HashSet<String>,
) -> anyhow::Result<()> {
    ensure!(
        !group.count_parameter.trim().is_empty(),
        "dynamic signal/action port groups must reference a count_parameter"
    );
    ensure!(
        !group.name_template.trim().is_empty(),
        "dynamic signal/action port groups must declare a name_template"
    );
    ensure!(
        group.name_template.contains("{index}"),
        "dynamic signal/action port group name_template values must contain {{index}}"
    );
    ensure!(
        !fixed_names.contains(&group.name_template),
        "dynamic signal/action port templates must not duplicate fixed port names"
    );
    ensure!(
        group.name_template != FLOW_SIGNAL_POST_ACTIVATE,
        "dynamic signal/action ports must not use the reserved post_activate signal"
    );

    let parameter = parameters
        .iter()
        .find(|parameter| parameter.name == group.count_parameter)
        .ok_or_else(|| anyhow::anyhow!("Unknown count parameter {}", group.count_parameter))?;
    ensure!(
        parameter.editor.kind == expected_editor_kind,
        "dynamic signal/action port groups must bind to the matching port-count parameter kind"
    );
    Ok(())
}

fn validate_parameter_definition(
    registry: &Registry,
    parameter: &FlowParameterDefinition,
) -> anyhow::Result<()> {
    registry.validate_datatype(&parameter.datatype)?;
    for control in std::iter::once(&parameter.editor).chain(parameter.controller.iter()) {
        let kind = parameter.datatype.kind.as_str();
        let compatible = match control.kind {
            FlowParameterEditorKind::Boolean => kind == "boolean",
            FlowParameterEditorKind::Unsigned
            | FlowParameterEditorKind::InputPortCount
            | FlowParameterEditorKind::OutputPortCount => kind == "int",
            FlowParameterEditorKind::Enum => {
                ensure!(!control.values.is_empty(), "Enum editors require values");
                kind == "string"
            }
            FlowParameterEditorKind::List | FlowParameterEditorKind::MediaPreview => kind == "list",
            FlowParameterEditorKind::String
            | FlowParameterEditorKind::Text
            | FlowParameterEditorKind::TextInput => {
                kind == "string"
                    || registry.default_editor(&parameter.datatype)
                        == Some(FlowParameterEditorKind::String)
            }
            _ => registry.default_editor(&parameter.datatype) == Some(control.kind.clone()),
        };
        ensure!(
            compatible,
            "Editor {:?} is incompatible with datatype {}",
            control.kind,
            kind
        );
        if let Some(value) = &control.default_value {
            let mut declaration = parameter.clone();
            declaration.editor = control.clone();
            if let Some(error) = validate_parameter_value(&declaration, value, registry) {
                anyhow::bail!("Invalid default for {}: {}", parameter.name, error);
            }
        }
    }
    Ok(())
}
