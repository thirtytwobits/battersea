//! Copyright (c) Scott A Dixon
//!
//! Shared `output_encoding` substrate used by source nodes (and Concatenate)
//! that let the author pick how a record set or input bundle is rendered
//! into a prompt fragment (markdown / xml / plain). The `plain` encoding
//! produces CSV-shaped output (header row + data rows) with a configurable
//! inter-cell delimiter set by the node's (or flow's) `plain_fragment_delimiter`
//! parameter.
//!
//! Every node that exposes an `output_encoding` enum parameter and declares
//! formatter variants suffixed `.markdown`, `.xml`, `.plain` should:
//!
//! 1. Read the chosen encoding with [`read_output_encoding`].
//! 2. Read the inter-cell delimiter with [`read_plain_fragment_delimiter`]
//!    (which also handles `inherit` cascading from the flow).
//! 3. Build a per-encoding variant key from a preset name with
//!    [`encoding_variant_key`] (or compose its own).
//! 4. Emit through [`render_encoding_aware_fragments`], passing the resolved
//!    delimiter as a binding so the formatter templates interpolate it into
//!    their separators.
//!
//! Field escaping ([`OutputEncoding::escape_field_with_delim`]) is the author's
//! responsibility — call it when interpolating untrusted text into formatter
//! bindings so XML/Plain outputs stay well-formed.

use crate::{FlowDocument, FlowNode, FlowNodeDefinition, FlowPort};
use anyhow::Result;
use serde_json::{Map, Value};

use crate::ports::effective_parameter_value;
use crate::template_engine::{render_port_formatter, render_port_formatter_fragments};

/// Encoding selected by a source node's `output_encoding` parameter.
///
/// Defaults to [`Self::Xml`] when the parameter value is missing or
/// unrecognised — matches the historical rule-query default and keeps flows
/// authored before this parameter existed emitting tagged output. Individual
/// nodes are free to override the default in their manifest's
/// `editor.default_value`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutputEncoding {
    Markdown,
    Xml,
    Plain,
}

impl OutputEncoding {
    /// Resolves the encoding from a node parameter value. Unknown / missing →
    /// `Xml` so legacy flows keep their tagged output.
    pub fn from_param_value(value: Option<&str>) -> Self {
        match value {
            Some("markdown") => Self::Markdown,
            Some("plain") => Self::Plain,
            _ => Self::Xml,
        }
    }

    /// Suffix used in formatter variant keys (`{preset}.{suffix}`).
    pub fn variant_suffix(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Xml => "xml",
            Self::Plain => "plain",
        }
    }

    /// Escapes one field for the active encoding: raw for Markdown, XML
    /// entities for Xml, RFC-4180-style quoting keyed off the supplied
    /// `plain_fragment_delimiter` for Plain (cells containing the actual
    /// delimiter, any quote, or newline get wrapped in double-quotes).
    pub fn escape_field_with_delim(self, value: &str, delimiter: &str) -> String {
        match self {
            Self::Markdown => value.to_string(),
            Self::Xml => escape_xml(value),
            Self::Plain => plain_escape(value, delimiter),
        }
    }

    /// Renders a plaintext field for this encoding. Markdown output protects
    /// heading-shaped lines so plaintext cannot accidentally claim prompt
    /// structure.
    pub fn render_plaintext_field_with_delim(self, value: &str, delimiter: &str) -> String {
        match self {
            Self::Markdown => crate::prompt_markdown::escape_plaintext_markdown_headings(value),
            Self::Xml | Self::Plain => self.escape_field_with_delim(value, delimiter),
        }
    }

    /// Renders an authored-markdown field for this encoding. Markdown output
    /// shifts source headings below the prompt section heading; non-markdown
    /// encodings keep the existing escaping rules.
    pub fn render_markdown_field_with_delim(self, value: &str, delimiter: &str) -> String {
        match self {
            Self::Markdown => crate::prompt_markdown::nest_markdown_headings(value),
            Self::Xml | Self::Plain => self.escape_field_with_delim(value, delimiter),
        }
    }

    pub fn prompt_fragment_encoding(self) -> crate::prompt_markdown::PromptFragmentEncoding {
        match self {
            Self::Markdown => crate::prompt_markdown::PromptFragmentEncoding::Markdown,
            Self::Xml => crate::prompt_markdown::PromptFragmentEncoding::Xml,
            Self::Plain => crate::prompt_markdown::PromptFragmentEncoding::Plain,
        }
    }
}

/// Whitespace handling selected by flow-wide defaults or node parameters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WhitespaceMode {
    Trim,
    Preserve,
    Compact,
}

impl WhitespaceMode {
    /// Resolves the whitespace mode from a parameter value. Unknown / missing
    /// values default to `Trim`, matching Concatenate's historical default.
    pub fn from_param_value(value: Option<&str>) -> Self {
        match value {
            Some("preserve") => Self::Preserve,
            Some("compact") => Self::Compact,
            _ => Self::Trim,
        }
    }
}

/// Escapes the XML-sensitive characters used by XML formatter variants.
pub fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Quotes a field for Plain (CSV-shaped) output: fields that contain the
/// configured `delimiter`, a double-quote, or any newline are wrapped in
/// double-quotes with embedded quotes doubled. An empty `delimiter` (the
/// "None" delimiter choice) only quotes on quote/newline content.
pub fn plain_escape(value: &str, delimiter: &str) -> String {
    let contains_delimiter = !delimiter.is_empty() && value.contains(delimiter);
    let contains_special = value
        .chars()
        .any(|character| matches!(character, '"' | '\n' | '\r'));
    if contains_delimiter || contains_special {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Resolves a flow document's `output_encoding` field — the flow-wide
/// fallback for source nodes set to `"inherit"`. Unknown / unset values
/// land on [`OutputEncoding::Xml`], matching the substrate's historical
/// default and the protocol-level [`crate::default_flow_output_encoding`]
/// constant.
pub fn flow_output_encoding(flow: &FlowDocument) -> OutputEncoding {
    OutputEncoding::from_param_value(Some(flow.output_encoding.as_str()))
}

/// Resolves a flow document's `whitespace_mode` field — the flow-wide
/// fallback for nodes set to `"inherit"`.
pub fn flow_whitespace_mode(flow: &FlowDocument) -> WhitespaceMode {
    WhitespaceMode::from_param_value(Some(flow.whitespace_mode.as_str()))
}

/// Reads the `output_encoding` parameter on a node, resolving the literal
/// `"inherit"` value (and any unrecognised string) to the flow-wide
/// fallback. Concrete values (`"markdown"`, `"xml"`, `"plain"`) override the
/// flow default for that node.
pub fn read_output_encoding(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    flow_default: OutputEncoding,
) -> OutputEncoding {
    let value = effective_parameter_value(node, definition, "output_encoding");
    match value.as_ref().and_then(|value| value.as_str()) {
        Some("markdown") => OutputEncoding::Markdown,
        Some("xml") => OutputEncoding::Xml,
        Some("plain") => OutputEncoding::Plain,
        // "inherit", any unknown literal, or a missing parameter all fall
        // back to the flow-wide encoding so the node automatically tracks
        // whatever encoding the surrounding flow declares.
        _ => flow_default,
    }
}

/// Reads the `plain_fragment_delimiter` parameter on a node, returning the
/// concrete delimiter string ready for use in CSV-style field separation
/// or Concatenate's plain joiner.
///
/// When the node's `output_encoding` is `"inherit"`, the delimiter also
/// inherits from the flow document. When the node's encoding is set
/// explicitly, the node's own delimiter is used (or its parameter default
/// if unset).
pub fn read_plain_fragment_delimiter(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    flow: &FlowDocument,
) -> String {
    let encoding_param = effective_parameter_value(node, definition, "output_encoding");
    let encoding_is_inherit = encoding_param
        .as_ref()
        .and_then(|value| value.as_str())
        .map(|value| value == "inherit")
        .unwrap_or(true);
    if encoding_is_inherit {
        return plain_fragment_delimiter_token_to_string(&flow.plain_fragment_delimiter);
    }
    let value = effective_parameter_value(node, definition, "plain_fragment_delimiter");
    let token = value
        .as_ref()
        .and_then(|value| value.as_str())
        .unwrap_or("blank_line");
    plain_fragment_delimiter_token_to_string(token)
}

/// Reads the `whitespace_mode` parameter on a node, resolving the literal
/// `"inherit"` value (and any unrecognised string) to the flow-wide fallback.
///
/// Markdown output requires line structure to remain intact, so `Compact`
/// resolves to `Trim` whenever the effective output encoding is Markdown.
pub fn read_whitespace_mode(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    flow: &FlowDocument,
) -> WhitespaceMode {
    let flow_encoding = flow_output_encoding(flow);
    let effective_encoding = read_output_encoding(node, definition, flow_encoding);
    let value = effective_parameter_value(node, definition, "whitespace_mode");
    let resolved = match value.as_ref().and_then(|value| value.as_str()) {
        Some("trim") => WhitespaceMode::Trim,
        Some("preserve") => WhitespaceMode::Preserve,
        Some("compact") => WhitespaceMode::Compact,
        _ => flow_whitespace_mode(flow),
    };
    if effective_encoding == OutputEncoding::Markdown && resolved == WhitespaceMode::Compact {
        WhitespaceMode::Trim
    } else {
        resolved
    }
}

/// Builds a per-encoding formatter variant key from a `{preset}.{suffix}`
/// pair. Use the same `preset` token across all three encoding variants in
/// `formatter_blocks` so this lookup stays stable.
pub fn encoding_variant_key(preset: &str, encoding: OutputEncoding) -> String {
    format!("{preset}.{}", encoding.variant_suffix())
}

/// Resolves a plain-fragment-delimiter token from the catalog enum
/// (`comma`, `blank_line`, `newline`, `space`, `none`) into the concrete
/// separator string used when joining or rendering CSV-shaped output.
pub fn plain_fragment_delimiter_token_to_string(token: &str) -> String {
    match token {
        "comma" => ",".to_string(),
        "newline" => "\n".to_string(),
        "space" => " ".to_string(),
        "none" => String::new(),
        _ => "\n\n".to_string(),
    }
}

/// Renders a port's formatter into an emission-ready fragment vector,
/// dispatching by encoding:
///
/// - **Markdown** uses [`render_port_formatter_fragments`] to emit one
///   element per top-level item.
/// - **XML / Plain** uses [`render_port_formatter`] to emit one structurally
///   complete string, wrapped as a single-element vector so the token type
///   stays `prompt.fragmentArray` and the receiver leaves the formatter-
///   internal layout untouched. The caller is responsible for including
///   `plain_fragment_delimiter` in `bindings` when Plain templates expect it.
///
/// Empty payloads collapse to an empty vector regardless of encoding.
pub fn render_encoding_aware_fragments(
    port: &FlowPort,
    variant_key: Option<&str>,
    bindings: &Map<String, Value>,
    encoding: OutputEncoding,
) -> Result<Vec<String>> {
    match encoding {
        OutputEncoding::Markdown => render_port_formatter_fragments(port, variant_key, bindings),
        OutputEncoding::Xml | OutputEncoding::Plain => {
            let single = render_port_formatter(port, variant_key, bindings)?;
            Ok(if single.is_empty() {
                Vec::new()
            } else {
                vec![single]
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{flow_whitespace_mode, read_whitespace_mode, WhitespaceMode};
    use crate::{
        FlowDocument, FlowNode, FlowNodeClass, FlowNodeDefinition, FlowParameterDataType,
        FlowParameterDefinition, FlowParameterEditor, FlowParameterEditorKind,
    };
    use serde_json::{json, Value};
    use std::collections::HashMap;

    fn flow(output_encoding: &str, whitespace_mode: &str) -> FlowDocument {
        FlowDocument {
            version: 1,
            flow_key: "test-flow".to_string(),
            title: "Test Flow".to_string(),
            description: None,
            output_encoding: output_encoding.to_string(),
            plain_fragment_delimiter: "blank_line".to_string(),
            whitespace_mode: whitespace_mode.to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
            layout: None,
            metadata: None,
        }
    }

    fn enum_parameter(name: &str, default_value: &str) -> FlowParameterDefinition {
        FlowParameterDefinition {
            name: name.to_string(),
            datatype: FlowParameterDataType::named("string"),
            editor: FlowParameterEditor {
                kind: FlowParameterEditorKind::Enum,
                min: None,
                max: None,
                values: Vec::new(),
                source: None,
                default_value: Some(Value::String(default_value.to_string())),
            },
            controller: None,
            short_description: None,
            long_description: None,
        }
    }

    fn definition() -> FlowNodeDefinition {
        FlowNodeDefinition {
            class_name: "Concatenate".to_string(),
            short_description: String::new(),
            long_description: String::new(),
            kind: FlowNodeClass::Inline,
            handler_id: "battersea.concatenate".to_string(),
            interfaces: Vec::new(),
            activation_parameters: Vec::new(),
            parameters: vec![
                enum_parameter("output_encoding", "inherit"),
                enum_parameter("whitespace_mode", "inherit"),
            ],
            input_ports: Vec::new(),
            output_ports: Vec::new(),
            dynamic_input_ports: Vec::new(),
            dynamic_output_ports: Vec::new(),
            dynamic_action_ports: Vec::new(),
            dynamic_signal_ports: Vec::new(),
            controller_outputs: None,
            controller_actions: None,
            action_ports: Vec::new(),
            signal_ports: Vec::new(),
            automation_ports: Vec::new(),
        }
    }

    fn node(parameter_values: HashMap<String, Value>) -> FlowNode {
        FlowNode {
            id: "concatenate-1".to_string(),
            definition_name: "Concatenate".to_string(),
            instance_name: "Concatenate".to_string(),
            parameter_values,
            port_parameter_values: None,
            port_order: None,
            port_names: None,
        }
    }

    #[test]
    fn flow_whitespace_mode_defaults_unknown_values_to_trim() {
        assert_eq!(
            flow_whitespace_mode(&flow("xml", "compact")),
            WhitespaceMode::Compact
        );
        assert_eq!(flow_whitespace_mode(&flow("xml", "")), WhitespaceMode::Trim);
    }

    #[test]
    fn read_whitespace_mode_inherits_from_flow() {
        assert_eq!(
            read_whitespace_mode(
                &node(HashMap::new()),
                &definition(),
                &flow("xml", "preserve")
            ),
            WhitespaceMode::Preserve
        );
    }

    #[test]
    fn read_whitespace_mode_prefers_explicit_node_value() {
        assert_eq!(
            read_whitespace_mode(
                &node(HashMap::from([(
                    "whitespace_mode".to_string(),
                    json!("compact")
                )])),
                &definition(),
                &flow("xml", "preserve"),
            ),
            WhitespaceMode::Compact
        );
    }

    #[test]
    fn read_whitespace_mode_resolves_compact_markdown_to_trim() {
        assert_eq!(
            read_whitespace_mode(
                &node(HashMap::from([
                    ("output_encoding".to_string(), json!("markdown")),
                    ("whitespace_mode".to_string(), json!("compact")),
                ])),
                &definition(),
                &flow("xml", "preserve"),
            ),
            WhitespaceMode::Trim
        );
        assert_eq!(
            read_whitespace_mode(
                &node(HashMap::new()),
                &definition(),
                &flow("markdown", "compact")
            ),
            WhitespaceMode::Trim
        );
    }
}
