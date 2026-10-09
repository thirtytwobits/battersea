//! Copyright (c) Scott A Dixon
//!
//! Flow runtime handler that concatenates prompt-fragment inputs into one prompt
//! fragment output.

use async_trait::async_trait;
use battersea_flow::{output_encoding::WhitespaceMode, ports::effective_parameter_value};
use battersea_flow::{FlowNode, FlowNodeDefinition};
use serde_json::{Map, Value};
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

use crate::{NodeError as EngineError, NodeResult as EngineResult};
use battersea_runtime::Token as FlowToken;
use battersea_runtime::{ExecutionError, ExecutionHost, NodeHandler as FlowRuntimeHandler};

pub struct ConcatenateHandler;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConcatenateEmptyInputRule {
    Skip,
    Keep,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConcatenateConfig {
    empty_input_rule: ConcatenateEmptyInputRule,
    ordering: ConcatenateOrdering,
    encoding: battersea_flow::output_encoding::OutputEncoding,
    /// Resolved field separator string (e.g. "\n\n", ",", " "). Drives Plain
    /// inter-port spacing. Inherited from the flow when the node's
    /// `output_encoding` is `"inherit"`.
    plain_fragment_delimiter: String,
    whitespace_mode: WhitespaceMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConcatenateOrdering {
    Descending,
    Ascending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConcatenatePart {
    port_name: String,
    text: String,
    encoding: Option<battersea_flow::prompt_markdown::PromptFragmentEncoding>,
    /// Set when this part is one of several sibling fragments produced by an
    /// upstream `prompt.fragmentArray` on the same input port. Sibling parts
    /// share the same `port_name` and join into one rendered block using
    /// this delimiter (taken from the port's `array_delimiter` parameter,
    /// default `"\n\n"`). For scalar `prompt.fragment` inputs, this is None
    /// and the part stands alone.
    array_delimiter: Option<String>,
}

struct ConcatenateTextBlock {
    port_name: String,
    text: String,
    xml_text: String,
}

impl ConcatenateOrdering {
    fn from_parameter(value: Option<&str>) -> Self {
        match value {
            Some("ascending") => Self::Ascending,
            _ => Self::Descending,
        }
    }
}

impl ConcatenateEmptyInputRule {
    fn from_parameter(value: Option<&str>) -> Self {
        match value {
            Some("keep") => Self::Keep,
            _ => Self::Skip,
        }
    }
}

impl ConcatenateConfig {
    fn from_node(
        node: &FlowNode,
        definition: &FlowNodeDefinition,
        flow: &battersea_flow::FlowDocument,
    ) -> Self {
        let flow_encoding = battersea_flow::output_encoding::flow_output_encoding(flow);
        let encoding =
            battersea_flow::output_encoding::read_output_encoding(node, definition, flow_encoding);
        let plain_fragment_delimiter =
            battersea_flow::output_encoding::read_plain_fragment_delimiter(node, definition, flow);
        Self {
            empty_input_rule: ConcatenateEmptyInputRule::from_parameter(
                string_parameter_value(node, definition, "empty_input_rule").as_deref(),
            ),
            ordering: ConcatenateOrdering::from_parameter(
                string_parameter_value(node, definition, "ordering").as_deref(),
            ),
            encoding,
            plain_fragment_delimiter,
            whitespace_mode: battersea_flow::output_encoding::read_whitespace_mode(
                node, definition, flow,
            ),
        }
    }
}

fn string_parameter_value(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    parameter_name: &str,
) -> Option<String> {
    effective_parameter_value(node, definition, parameter_name)
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
}

/// Default inter-element delimiter when an input port receives a
/// `prompt.fragmentArray` and no per-port override is set. Two newlines
/// produces a markdown-friendly blank-line gap between adjacent items
/// regardless of whether each item happens to carry a trailing newline
/// of its own.
const DEFAULT_ARRAY_DELIMITER: &str = "\n\n";

/// Reads the per-port `array_delimiter` parameter, falling back to
/// `DEFAULT_ARRAY_DELIMITER` when nothing is authored. Stored under
/// `FlowNode::port_parameter_values.input[port_name].array_delimiter`.
fn port_array_delimiter(node: &FlowNode, port_name: &str) -> String {
    node.port_parameter_values
        .as_ref()
        .and_then(|values| values.input.get(port_name))
        .and_then(|port_values| port_values.get("array_delimiter"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| DEFAULT_ARRAY_DELIMITER.to_string())
}

fn normalise_concatenate_text(text: &str, whitespace_mode: WhitespaceMode) -> String {
    match whitespace_mode {
        WhitespaceMode::Trim => text.trim().to_string(),
        WhitespaceMode::Preserve => text.to_string(),
        WhitespaceMode::Compact => text.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

fn should_keep_concatenate_text(text: &str, empty_input_rule: ConcatenateEmptyInputRule) -> bool {
    match empty_input_rule {
        ConcatenateEmptyInputRule::Skip => !text.is_empty(),
        ConcatenateEmptyInputRule::Keep => true,
    }
}

fn sort_concatenate_ports(ordered_ports: &mut [String], ordering: ConcatenateOrdering) {
    ordered_ports.sort_by_key(|port| {
        port.rsplit_once('-')
            .and_then(|(_, suffix)| suffix.parse::<usize>().ok())
            .unwrap_or(usize::MAX)
    });
    if ordering == ConcatenateOrdering::Descending {
        ordered_ports.reverse();
    }
}

fn render_concatenate_output(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    parts: &[ConcatenatePart],
    config: &ConcatenateConfig,
) -> EngineResult<String> {
    let variant_key =
        battersea_flow::output_encoding::encoding_variant_key("concatenate", config.encoding);
    let bindings = concatenate_formatter_bindings(node, parts, config);
    let port = definition
        .output_ports
        .iter()
        .find(|port| port.name == "output")
        .ok_or_else(|| {
            EngineError::internal(format!(
                "Flow node \"{}\" definition \"{}\" does not declare output port \"output\".",
                node.id, definition.class_name
            ))
        })?;

    battersea_flow::template_engine::render_port_formatter(port, Some(&variant_key), &bindings)
        .map_err(|error| {
            EngineError::internal(format!(
                "Failed to render formatter for flow node \"{}\" output port \"output\": {}",
                node.id, error
            ))
        })
}

fn concatenate_formatter_bindings(
    node: &FlowNode,
    parts: &[ConcatenatePart],
    config: &ConcatenateConfig,
) -> Map<String, Value> {
    let items = concatenate_text_blocks(parts, config)
        .into_iter()
        .map(|block| {
            let mut item = Map::new();
            item.insert(
                "port_label".to_string(),
                Value::String(concatenate_port_label(node, &block.port_name)),
            );
            item.insert(
                "xml_element_name".to_string(),
                Value::String(concatenate_xml_element_name(node, &block.port_name)),
            );
            item.insert("xml_text".to_string(), Value::String(block.xml_text));
            item.insert("text".to_string(), Value::String(block.text));
            Value::Object(item)
        })
        .collect::<Vec<_>>();

    let mut bindings = Map::new();
    bindings.insert("items".to_string(), Value::Array(items));
    bindings.insert(
        "plain_fragment_delimiter".to_string(),
        Value::String(config.plain_fragment_delimiter.clone()),
    );
    bindings
}

fn concatenate_text_blocks(
    parts: &[ConcatenatePart],
    config: &ConcatenateConfig,
) -> Vec<ConcatenateTextBlock> {
    // Group sibling parts (same port_name, consecutive) so multiple fragments
    // from a single `prompt.fragmentArray` input use that port's
    // `array_delimiter` for within-port joining. The configured formatter owns
    // any between-port separators.
    let mut per_port_blocks: Vec<ConcatenateTextBlock> = Vec::new();
    let mut current_port: Option<&str> = None;
    let mut current_texts: Vec<String> = Vec::new();
    let mut current_xml_texts: Vec<String> = Vec::new();
    let mut current_array_delimiter: Option<&str> = None;
    for part in parts {
        match current_port {
            Some(port) if port == part.port_name => {
                current_texts.push(render_concatenate_part_text(part, config));
                current_xml_texts.push(render_concatenate_part_xml_text(part, config));
                if current_array_delimiter.is_none() {
                    current_array_delimiter = part.array_delimiter.as_deref();
                }
            }
            _ => {
                if let Some(port) = current_port.take() {
                    let delim = current_array_delimiter.unwrap_or(DEFAULT_ARRAY_DELIMITER);
                    per_port_blocks.push(ConcatenateTextBlock {
                        port_name: port.to_string(),
                        text: current_texts.join(delim),
                        xml_text: current_xml_texts.join(delim),
                    });
                }
                current_port = Some(&part.port_name);
                current_texts = vec![render_concatenate_part_text(part, config)];
                current_xml_texts = vec![render_concatenate_part_xml_text(part, config)];
                current_array_delimiter = part.array_delimiter.as_deref();
            }
        }
    }
    if let Some(port) = current_port {
        let delim = current_array_delimiter.unwrap_or(DEFAULT_ARRAY_DELIMITER);
        per_port_blocks.push(ConcatenateTextBlock {
            port_name: port.to_string(),
            text: current_texts.join(delim),
            xml_text: current_xml_texts.join(delim),
        });
    }
    per_port_blocks
}

fn render_concatenate_part_text(part: &ConcatenatePart, config: &ConcatenateConfig) -> String {
    match (config.encoding, part.encoding) {
        (
            battersea_flow::output_encoding::OutputEncoding::Markdown,
            Some(battersea_flow::prompt_markdown::PromptFragmentEncoding::PlainText),
        ) => battersea_flow::prompt_markdown::escape_plaintext_markdown_headings(&part.text),
        (
            battersea_flow::output_encoding::OutputEncoding::Markdown,
            Some(battersea_flow::prompt_markdown::PromptFragmentEncoding::Markdown),
        ) => battersea_flow::prompt_markdown::nest_markdown_headings(&part.text),
        _ => part.text.clone(),
    }
}

fn render_concatenate_part_xml_text(part: &ConcatenatePart, config: &ConcatenateConfig) -> String {
    match (config.encoding, part.encoding) {
        (
            battersea_flow::output_encoding::OutputEncoding::Xml,
            Some(battersea_flow::prompt_markdown::PromptFragmentEncoding::Xml),
        ) => part.text.clone(),
        (battersea_flow::output_encoding::OutputEncoding::Xml, _) => xml_escape_text(&part.text),
        _ => xml_escape_text(&render_concatenate_part_text(part, config)),
    }
}

fn concatenate_port_label(node: &FlowNode, port_name: &str) -> String {
    trimmed_input_port_alias(node, port_name)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| humanise_concatenate_port_name(port_name))
}

fn concatenate_xml_element_name(node: &FlowNode, port_name: &str) -> String {
    trimmed_input_port_alias(node, port_name)
        .and_then(sanitise_xml_element_name)
        .unwrap_or_else(|| {
            sanitise_xml_element_name(port_name).unwrap_or_else(|| "input".to_string())
        })
}

fn trimmed_input_port_alias<'a>(node: &'a FlowNode, port_name: &str) -> Option<&'a str> {
    let alias = node.port_names.as_ref()?.input.get(port_name)?;
    let trimmed = alias.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn humanise_concatenate_port_name(port_name: &str) -> String {
    port_name
        .split(['-', '_'])
        .filter(|segment| !segment.is_empty())
        .map(humanise_concatenate_segment)
        .collect::<Vec<_>>()
        .join(" ")
}

fn humanise_concatenate_segment(segment: &str) -> String {
    let mut characters = segment.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    let mut result = String::new();
    result.extend(first.to_uppercase());
    result.push_str(characters.as_str());
    result
}

fn sanitise_xml_element_name(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut sanitised = String::new();
    let mut last_was_separator = false;
    for character in trimmed.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            sanitised.push(character.to_ascii_lowercase());
            last_was_separator = false;
            continue;
        }

        if !last_was_separator && !sanitised.is_empty() {
            sanitised.push('_');
            last_was_separator = true;
        }
    }

    while sanitised.ends_with('_') {
        sanitised.pop();
    }
    if sanitised.is_empty() {
        return None;
    }
    if sanitised
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        sanitised.insert(0, '_');
    }
    Some(sanitised)
}

fn xml_escape_text(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            _ => character.to_string(),
        })
        .collect()
}

#[async_trait]
impl<H: ExecutionHost> FlowRuntimeHandler<H> for ConcatenateHandler {
    fn handler_id(&self) -> &'static str {
        "battersea.concatenate"
    }

    async fn execute_node(
        &self,
        core: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        definition: &FlowNodeDefinition,
        _activation_values: Option<&HashMap<String, Value>>,
        token: &CancellationToken,
    ) -> Result<(), H::Error> {
        let inputs = core
            .connected_input_ports(runtime, &node.id)
            .into_iter()
            .map(|port| {
                core.take_flow_input_token(runtime, &node.id, &port)
                    .map(|value| (port, value))
            })
            .collect::<Result<HashMap<_, _>, H::Error>>()?;
        let output = prepare_concatenate(&runtime.flow, node, definition, &inputs)
            .map_err(|e| H::Error::internal(e.to_string()))?;
        runtime.executed_nodes.insert(node.id.clone());
        core.emit_flow_token(runtime, &node.id, "output", output, token)
            .await
    }
}

pub fn prepare_concatenate(
    flow: &battersea_flow::FlowDocument,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    inputs: &HashMap<String, FlowToken>,
) -> EngineResult<FlowToken> {
    let config = ConcatenateConfig::from_node(node, definition, flow);
    let mut ordered_ports = inputs.keys().cloned().collect::<Vec<_>>();
    sort_concatenate_ports(&mut ordered_ports, config.ordering);

    let mut parts = Vec::new();
    // Track the first input's effective scalar token type so the auto
    // output can adopt it. `prompt.fragmentArray` collapses to its
    // element type (`prompt.fragment`) since concatenate flattens.
    let mut emitted_token_type: Option<String> = None;
    for port_name in ordered_ports {
        let token_value = &inputs[&port_name];
        if emitted_token_type.is_none() {
            emitted_token_type = Some(match token_value.token_type.as_str() {
                "prompt.fragmentArray" => "prompt.fragment".to_string(),
                other => other.to_string(),
            });
        }
        let raw_fragments =
            battersea_flow::prompt_markdown::prompt_fragment_items(&token_value.value);
        // Sibling fragments come from a `prompt.fragmentArray` (length > 1).
        // They share the same port_name and need a stable inter-element
        // delimiter regardless of any trailing whitespace on each fragment.
        let array_delimiter = (raw_fragments.len() > 1
            || token_value.token_type == "prompt.fragmentArray")
            .then(|| port_array_delimiter(node, &port_name));
        for fragment in raw_fragments {
            let normalised = normalise_concatenate_text(&fragment.text, config.whitespace_mode);
            if should_keep_concatenate_text(&normalised, config.empty_input_rule) {
                parts.push(ConcatenatePart {
                    port_name: port_name.clone(),
                    text: normalised,
                    encoding: fragment.encoding,
                    array_delimiter: array_delimiter.clone(),
                });
            }
        }
    }

    let emitted_token_type = emitted_token_type.unwrap_or_else(|| "prompt.fragment".to_string());
    let output_text = render_concatenate_output(node, definition, &parts, &config)?;
    let output_value = if emitted_token_type == "prompt.fragment" {
        battersea_flow::prompt_markdown::prompt_fragment_value(
            output_text,
            config.encoding.prompt_fragment_encoding(),
        )
    } else {
        Value::String(output_text)
    };
    Ok(FlowToken {
        token_type: emitted_token_type,
        value: output_value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use battersea_flow::FlowNodePortNames;
    use std::collections::BTreeMap;
    fn concatenate_definition() -> FlowNodeDefinition {
        crate::test_catalog().definitions()["Concatenate"].clone()
    }
    fn make_concatenate_node(
        id: &str,
        parameter_values: HashMap<String, Value>,
        port_names: Option<FlowNodePortNames>,
    ) -> FlowNode {
        FlowNode {
            id: id.to_string(),
            definition_name: "Concatenate".to_string(),
            instance_name: format!("Concatenate {id}"),
            parameter_values,
            port_parameter_values: None,
            port_order: None,
            port_names,
        }
    }

    fn make_port_names(aliases: &[(&str, &str)]) -> FlowNodePortNames {
        FlowNodePortNames {
            action: BTreeMap::new(),
            automation: BTreeMap::new(),
            input: aliases
                .iter()
                .map(|(port_name, alias)| (port_name.to_string(), alias.to_string()))
                .collect(),
            output: BTreeMap::new(),
            signal: BTreeMap::new(),
        }
    }

    fn config_for(
        whitespace_mode: WhitespaceMode,
        encoding: battersea_flow::output_encoding::OutputEncoding,
        plain_fragment_delimiter: &str,
        empty_input_rule: ConcatenateEmptyInputRule,
    ) -> ConcatenateConfig {
        ConcatenateConfig {
            ordering: ConcatenateOrdering::Ascending,
            whitespace_mode,
            encoding,
            plain_fragment_delimiter: plain_fragment_delimiter.to_string(),
            empty_input_rule,
        }
    }

    #[test]
    fn whitespace_modes_normalise_text_as_configured() {
        assert_eq!(
            normalise_concatenate_text("  one   two \n", WhitespaceMode::Trim),
            "one   two"
        );
        assert_eq!(
            normalise_concatenate_text("  one   two \n", WhitespaceMode::Preserve),
            "  one   two \n"
        );
        assert_eq!(
            normalise_concatenate_text("  one   two \n three\t", WhitespaceMode::Compact),
            "one two three"
        );
    }

    #[test]
    fn markdown_and_xml_labels_use_aliases_with_stable_fallbacks() {
        let node = make_concatenate_node(
            "concatenate-1",
            HashMap::new(),
            Some(make_port_names(&[
                ("input-0", "Scene Heading"),
                ("input-1", "  "),
                ("input-2", "123 lead"),
            ])),
        );

        assert_eq!(concatenate_port_label(&node, "input-0"), "Scene Heading");
        assert_eq!(concatenate_port_label(&node, "input-1"), "Input 1");
        assert_eq!(
            concatenate_xml_element_name(&node, "input-0"),
            "scene_heading"
        );
        assert_eq!(concatenate_xml_element_name(&node, "input-1"), "input_1");
        assert_eq!(concatenate_xml_element_name(&node, "input-2"), "_123_lead");
    }

    #[test]
    fn sanitise_xml_names_and_escape_text_for_output() {
        assert_eq!(
            sanitise_xml_element_name("Prompt Fragment"),
            Some("prompt_fragment".to_string())
        );
        assert_eq!(
            sanitise_xml_element_name("input-0"),
            Some("input_0".to_string())
        );
        assert_eq!(sanitise_xml_element_name("!!!"), None);
        assert_eq!(
            xml_escape_text("A & B < C > D \"quoted\" 'single'"),
            "A &amp; B &lt; C &gt; D &quot;quoted&quot; &apos;single&apos;"
        );
    }

    #[test]
    fn plain_markdown_and_xml_formats_render_expected_output() {
        let definition = concatenate_definition();
        let node = make_concatenate_node(
            "concatenate-1",
            HashMap::new(),
            Some(make_port_names(&[
                ("input-0", "Prompt"),
                ("input-1", "Context"),
            ])),
        );
        let parts = vec![
            ConcatenatePart {
                port_name: "input-0".to_string(),
                text: "First".to_string(),
                encoding: None,
                array_delimiter: None,
            },
            ConcatenatePart {
                port_name: "input-1".to_string(),
                text: "Second & third".to_string(),
                encoding: None,
                array_delimiter: None,
            },
        ];

        assert_eq!(
            render_concatenate_output(
                &node,
                &definition,
                &parts,
                &config_for(
                    WhitespaceMode::Trim,
                    battersea_flow::output_encoding::OutputEncoding::Plain,
                    " ",
                    ConcatenateEmptyInputRule::Skip,
                ),
            )
            .expect("plain concatenate render"),
            "First Second & third"
        );
        assert_eq!(
            render_concatenate_output(
                &node,
                &definition,
                &parts,
                &config_for(
                    WhitespaceMode::Trim,
                    battersea_flow::output_encoding::OutputEncoding::Markdown,
                    "\n\n",
                    ConcatenateEmptyInputRule::Skip,
                ),
            )
            .expect("markdown concatenate render"),
            // ai-prompt-trojan-allow markdown-formatter-structure: test expectation for config-backed Concatenate markdown output.
            "## Prompt\n\nFirst\n\n## Context\n\nSecond & third"
        );
        assert_eq!(
            render_concatenate_output(
                &node,
                &definition,
                &parts,
                &config_for(
                    WhitespaceMode::Trim,
                    battersea_flow::output_encoding::OutputEncoding::Xml,
                    "\n\n",
                    ConcatenateEmptyInputRule::Skip,
                ),
            )
            .expect("xml concatenate render"),
            // ai-prompt-trojan-allow xml-formatter-structure: test expectation for config-backed Concatenate XML output.
            "<concatenate>\n<prompt>First</prompt>\n<context>Second &amp; third</context>\n</concatenate>"
        );
    }

    #[test]
    fn markdown_concatenate_nests_markdown_inputs_and_escapes_plaintext_inputs() {
        let definition = concatenate_definition();
        let node = make_concatenate_node(
            "concatenate-1",
            HashMap::new(),
            Some(make_port_names(&[
                ("input-0", "Composed fragment"),
                ("input-1", "User text"),
            ])),
        );
        let parts = vec![
            ConcatenatePart {
                port_name: "input-0".to_string(),
                // ai-prompt-trojan-allow markdown-formatter-structure: test fixture for nested markdown input, not production prompt prose.
                text: "## Inner\n\n### Detail".to_string(),
                encoding: Some(battersea_flow::prompt_markdown::PromptFragmentEncoding::Markdown),
                array_delimiter: None,
            },
            ConcatenatePart {
                port_name: "input-1".to_string(),
                text: "# Literal\n\nSetext\n---".to_string(),
                encoding: Some(battersea_flow::prompt_markdown::PromptFragmentEncoding::PlainText),
                array_delimiter: None,
            },
        ];

        assert_eq!(
            render_concatenate_output(
                &node,
                &definition,
                &parts,
                &config_for(
                    WhitespaceMode::Trim,
                    battersea_flow::output_encoding::OutputEncoding::Markdown,
                    "\n\n",
                    ConcatenateEmptyInputRule::Skip,
                ),
            )
            .expect("markdown concatenate render"),
            // ai-prompt-trojan-allow markdown-formatter-structure: test expectation for nested markdown and escaped plaintext inputs.
            "## Composed fragment\n\n### Inner\n\n#### Detail\n\n## User text\n\n\\# Literal\n\nSetext\n\\---"
        );
    }

    #[test]
    fn xml_concatenate_wraps_xml_inputs_without_escaping_their_elements() {
        let definition = concatenate_definition();
        let node = make_concatenate_node(
            "concatenate-1",
            HashMap::new(),
            Some(make_port_names(&[
                ("input-0", "Story"),
                ("input-1", "Prompt"),
            ])),
        );
        let parts = vec![
            ConcatenatePart {
                port_name: "input-0".to_string(),
                // ai-prompt-trojan-allow xml-formatter-structure: test fixture for nested XML input, not production prompt prose.
                text: "<concatenate>\n<setup>A &amp; B</setup>\n</concatenate>".to_string(),
                encoding: Some(battersea_flow::prompt_markdown::PromptFragmentEncoding::Xml),
                array_delimiter: None,
            },
            ConcatenatePart {
                port_name: "input-1".to_string(),
                // ai-prompt-trojan-allow xml-formatter-structure: test fixture for XML escaping, not production prompt prose.
                text: "Use <angle> text & quotes".to_string(),
                encoding: Some(battersea_flow::prompt_markdown::PromptFragmentEncoding::PlainText),
                array_delimiter: None,
            },
        ];

        assert_eq!(
            render_concatenate_output(
                &node,
                &definition,
                &parts,
                &config_for(
                    WhitespaceMode::Trim,
                    battersea_flow::output_encoding::OutputEncoding::Xml,
                    "\n\n",
                    ConcatenateEmptyInputRule::Skip,
                ),
            )
            .expect("xml concatenate render"),
            // ai-prompt-trojan-allow xml-formatter-structure: test expectation for nested XML input wrapped by an outer port tag.
            "<concatenate>\n<story><concatenate>\n<setup>A &amp; B</setup>\n</concatenate></story>\n<prompt>Use &lt;angle&gt; text &amp; quotes</prompt>\n</concatenate>"
        );
    }

    #[test]
    fn sort_concatenate_ports_honours_requested_ordering() {
        let mut ascending = vec![
            "input-2".to_string(),
            "input-0".to_string(),
            "input-10".to_string(),
            "input-1".to_string(),
        ];
        sort_concatenate_ports(&mut ascending, ConcatenateOrdering::Ascending);
        assert_eq!(
            ascending,
            vec![
                "input-0".to_string(),
                "input-1".to_string(),
                "input-2".to_string(),
                "input-10".to_string(),
            ]
        );

        sort_concatenate_ports(&mut ascending, ConcatenateOrdering::Descending);
        assert_eq!(
            ascending,
            vec![
                "input-10".to_string(),
                "input-2".to_string(),
                "input-1".to_string(),
                "input-0".to_string(),
            ]
        );
    }
}
