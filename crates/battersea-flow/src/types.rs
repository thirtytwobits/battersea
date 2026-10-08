//! Copyright (c) Scott A Dixon
//!
//! Defines the serialisable flow, catalogue, port and template contracts.

/// Opaque client-owned editor state, keyed by namespace.
pub type ClientLayoutState = BTreeMap<String, Value>;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

pub const FLOW_SIGNAL_POST_ACTIVATE: &str = "post_activate";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FlowPortKind {
    Input,
    Output,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FlowNodeClass {
    Source,
    Control,
    Hybrid,
    Instrument,
    Inline,
    Logic,
    Sink,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowPortDisplayClass {
    Source,
    Inline,
    Sink,
}

/// A nominal datatype registered by the host, or a recursive list/map of datatypes.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowParameterDataType {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Box<FlowParameterDataType>")]
    pub item_type: Option<Box<FlowParameterDataType>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Box<FlowParameterDataType>")]
    pub value_type: Option<Box<FlowParameterDataType>>,
}
impl FlowParameterDataType {
    pub fn named(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            item_type: None,
            value_type: None,
        }
    }
    pub fn list(item_type: Self) -> Self {
        Self {
            kind: "list".into(),
            item_type: Some(Box::new(item_type)),
            value_type: None,
        }
    }
    pub fn map(value_type: Self) -> Self {
        Self {
            kind: "map".into(),
            item_type: None,
            value_type: Some(Box::new(value_type)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowParameterEditorKind {
    Boolean,
    Unsigned,
    InputPortCount,
    OutputPortCount,
    Enum,
    List,
    MediaPreview,
    String,
    Text,
    TextInput,
    Properties,
    Query,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowParameterEditor {
    pub kind: FlowParameterEditorKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u32")]
    pub min: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u32")]
    pub max: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Value")]
    pub default_value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowParameterDefinition {
    pub name: String,
    pub datatype: FlowParameterDataType,
    pub editor: FlowParameterEditor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowParameterEditor")]
    pub controller: Option<FlowParameterEditor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub short_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub long_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowPortFormatterKind {
    PromptTemplate,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowControllerOutputDefinition {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub short_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub long_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowControllerActionDefinition {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub short_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub long_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowEdgeKind {
    Token,
    Signal,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowActionPortDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowPortDisplayClass")]
    pub display_class: Option<FlowPortDisplayClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub short_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub long_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowSignalPortDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowPortDisplayClass")]
    pub display_class: Option<FlowPortDisplayClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub short_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub long_description: Option<String>,
}

/// A typed sink port that delivers a token's value into one of the host
/// node's `parameter_values` *after* the upstream node activates. Unlike an
/// input port, an automation write does not drive activation; it is a
/// post-activation side-channel write that downstream activations and UI
/// bindings observe via the next flow-document broadcast.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowAutomationPortDefinition {
    pub name: String,
    /// The host-node parameter that this port writes to on token delivery.
    pub parameter_name: String,
    pub token_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_token_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowPortDisplayClass")]
    pub display_class: Option<FlowPortDisplayClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub short_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub long_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowPromptTextTemplateNode {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowPromptSequenceTemplateNode {
    #[serde(default)]
    pub separator: String,
    #[serde(default)]
    pub items: Vec<FlowPromptTemplateNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowPromptOptionalTemplateNode {
    pub binding: String,
    pub template: Box<FlowPromptTemplateNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowPromptListTemplateNode {
    pub binding: String,
    #[serde(default)]
    pub separator: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_bindings: Vec<String>,
    pub template: Box<FlowPromptTemplateNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FlowPromptTemplateNode {
    Text(FlowPromptTextTemplateNode),
    Sequence(FlowPromptSequenceTemplateNode),
    Optional(FlowPromptOptionalTemplateNode),
    List(FlowPromptListTemplateNode),
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowPromptTemplateDefinition {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<String>,
    pub template: FlowPromptTemplateNode,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowPortFormatter {
    pub kind: FlowPortFormatterKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowPromptTemplateDefinition")]
    pub default: Option<FlowPromptTemplateDefinition>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variants: BTreeMap<String, FlowPromptTemplateDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowPort {
    pub name: String,
    pub kind: FlowPortKind,
    pub token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowPortDisplayClass")]
    pub display_class: Option<FlowPortDisplayClass>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_token_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub short_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub long_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowPortFormatter")]
    pub formatter: Option<FlowPortFormatter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<FlowParameterDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowDynamicPortGroup {
    pub count_parameter: String,
    pub name_template: String,
    pub token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowPortDisplayClass")]
    pub display_class: Option<FlowPortDisplayClass>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_token_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub short_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub long_description: Option<String>,
    /// Per-port parameter editors. Each port expanded from this group inherits
    /// the same set of parameter declarations; values are stored per-port name
    /// under `FlowNode::port_parameter_values.input[port_name]` (or
    /// `.output[port_name]` for dynamic output groups).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<FlowParameterDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowDynamicSignalPortGroup {
    pub count_parameter: String,
    pub name_template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowPortDisplayClass")]
    pub display_class: Option<FlowPortDisplayClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub short_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub long_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowNodeDefinition {
    pub class_name: String,
    pub short_description: String,
    pub long_description: String,
    pub kind: FlowNodeClass,
    pub handler_id: String,
    #[serde(default)]
    pub interfaces: Vec<String>,
    #[serde(default)]
    pub activation_parameters: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<FlowParameterDefinition>,
    #[serde(default)]
    pub input_ports: Vec<FlowPort>,
    #[serde(default)]
    pub output_ports: Vec<FlowPort>,
    #[serde(default)]
    pub dynamic_input_ports: Vec<FlowDynamicPortGroup>,
    #[serde(default)]
    pub dynamic_output_ports: Vec<FlowDynamicPortGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dynamic_action_ports: Vec<FlowDynamicSignalPortGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dynamic_signal_ports: Vec<FlowDynamicSignalPortGroup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<FlowControllerOutputDefinition>")]
    pub controller_outputs: Option<Vec<FlowControllerOutputDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<FlowControllerActionDefinition>")]
    pub controller_actions: Option<Vec<FlowControllerActionDefinition>>,
    #[serde(default)]
    pub action_ports: Vec<FlowActionPortDefinition>,
    #[serde(default)]
    pub signal_ports: Vec<FlowSignalPortDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation_ports: Vec<FlowAutomationPortDefinition>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowNodePortNames {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub action: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub automation: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub input: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub signal: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowNodePortOrder {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signal: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowNodePortParameterValues {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub input: BTreeMap<String, BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output: BTreeMap<String, BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowNode {
    pub id: String,
    pub definition_name: String,
    pub instance_name: String,
    #[serde(default)]
    pub parameter_values: HashMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowNodePortParameterValues")]
    pub port_parameter_values: Option<FlowNodePortParameterValues>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowNodePortOrder")]
    pub port_order: Option<FlowNodePortOrder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "FlowNodePortNames")]
    pub port_names: Option<FlowNodePortNames>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowEdge {
    pub id: String,
    #[serde(default = "default_flow_edge_kind")]
    pub kind: FlowEdgeKind,
    pub source_node_id: String,
    pub source_port: String,
    pub target_node_id: String,
    pub target_port: String,
    #[serde(default)]
    pub order: u32,
}

fn default_flow_edge_kind() -> FlowEdgeKind {
    FlowEdgeKind::Token
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowDocument {
    pub version: u32,
    pub flow_key: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub description: Option<String>,
    /// Flow-wide rendering encoding for nodes whose `output_encoding`
    /// parameter is set to `"inherit"` (the per-node default). One of
    /// `"markdown"`, `"xml"`, or `"plain"`. Missing values (legacy flows
    /// authored before this field existed) fall back to `"xml"` via
    /// [`default_flow_output_encoding`], matching the substrate's
    /// historical default.
    #[serde(default = "default_flow_output_encoding")]
    pub output_encoding: String,
    /// Flow-wide field delimiter used when a node emits the `plain`
    /// encoding and that encoding came from inheriting the flow's choice.
    /// Catalog enum token: `comma`, `blank_line`, `newline`, `space`, or
    /// `none`. Defaults to `blank_line` for legacy flows.
    #[serde(default = "default_flow_plain_fragment_delimiter")]
    pub plain_fragment_delimiter: String,
    /// Flow-wide whitespace handling for nodes whose `whitespace_mode`
    /// parameter is set to `"inherit"` (the per-node default). One of
    /// `"trim"`, `"preserve"`, or `"compact"`. Missing values (legacy flows
    /// authored before this field existed) fall back to `"trim"` via
    /// [`default_flow_whitespace_mode`], matching Concatenate's historical
    /// default.
    #[serde(default = "default_flow_whitespace_mode")]
    pub whitespace_mode: String,
    #[serde(default)]
    pub nodes: Vec<FlowNode>,
    #[serde(default)]
    pub edges: Vec<FlowEdge>,
    /// Opaque per-client canvas state, keyed by writer namespace.
    /// Each client controls its own namespace.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "ClientLayoutState")]
    pub layout: Option<ClientLayoutState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Value")]
    pub metadata: Option<Value>,
}

/// Default flow-level output encoding when a flow document omits the field.
/// Matches the substrate's historical fallback so flows authored before this
/// field existed keep their tagged XML output.
pub fn default_flow_output_encoding() -> String {
    "xml".to_string()
}

/// Default flow-level plain-fragment delimiter when omitted. Matches the
/// historical Concatenate `plain_delimiter` default so legacy flows keep
/// their blank-line-separated plain emissions.
pub fn default_flow_plain_fragment_delimiter() -> String {
    "blank_line".to_string()
}

/// Default flow-level whitespace mode when omitted. Matches the historical
/// Concatenate `whitespace_mode` default.
pub fn default_flow_whitespace_mode() -> String {
    "trim".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowValidationIssue {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub edge_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowValidationResult {
    pub valid: bool,
    #[serde(default)]
    pub issues: Vec<FlowValidationIssue>,
}
