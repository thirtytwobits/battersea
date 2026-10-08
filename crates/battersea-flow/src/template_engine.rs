//! Copyright (c) Scott A Dixon
//!
//! Provides generic placeholder rendering and declarative flow-output template
//! rendering for manifest-backed prompt fragments.

use crate::{FlowPort, FlowPortFormatter, FlowPromptTemplateDefinition, FlowPromptTemplateNode};
use anyhow::{anyhow, Result};
use regex_lite::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// Renders every `{binding}` placeholder in a string from left to right using
/// the shared placeholder matcher.
///
/// Placeholder interpolation accepts only scalar JSON values. Strings, numbers,
/// booleans, and `null` are rendered inline, while arrays and objects are
/// rejected because embedded string templates cannot safely preserve structure.
/// Text that does not match the placeholder pattern is left unchanged.
pub fn render_template_string(template: &str, bindings: &Map<String, Value>) -> Result<String> {
    let mut rendered = String::new();
    let mut last_end = 0usize;
    for capture in placeholder_regex().captures_iter(template) {
        let whole = capture
            .get(0)
            .ok_or_else(|| anyhow!("Prompt placeholder capture is invalid."))?;
        let name = capture
            .get(1)
            .ok_or_else(|| anyhow!("Prompt placeholder capture is invalid."))?
            .as_str();
        rendered.push_str(&template[last_end..whole.start()]);
        let value = bindings
            .get(name)
            .ok_or_else(|| anyhow!("Prompt template referenced missing placeholder \"{name}\"."))?;
        rendered.push_str(&render_inline_value(name, value)?);
        last_end = whole.end();
    }
    rendered.push_str(&template[last_end..]);
    Ok(rendered)
}

/// Renders a flow output port using either its default formatter or a named
/// formatter variant, returning a single string.
///
/// The port must declare a formatter. Variant selection is strict: a requested
/// variant must exist, and default rendering requires a default formatter
/// definition to be present.
///
/// Use this when the emitted token is `prompt.fragment` (one structurally
/// complete payload) — e.g. XML/CSV output where the formatter's internal
/// `separator` fields define the canonical inter-item spacing and downstream
/// receivers must not re-join. Use `render_port_formatter_fragments` instead
/// when the emitted token is `prompt.fragmentArray` (parallel items the
/// receiver may join with its own delimiter).
pub fn render_port_formatter(
    port: &FlowPort,
    variant_key: Option<&str>,
    bindings: &Map<String, Value>,
) -> Result<String> {
    let formatter = port.formatter.as_ref().ok_or_else(|| {
        anyhow!(
            "Flow output port \"{}\" does not declare a formatter.",
            port.name
        )
    })?;
    let definition = resolve_port_formatter_definition(formatter, &port.name, variant_key)?;
    render_flow_prompt_template(definition, bindings)
}

/// Renders a flow output port into top-level prompt fragments.
///
/// Fragment boundaries are derived only from the formatter's top-level
/// sequence children or top-level list items. Nested templates still render as
/// a single string fragment.
pub fn render_port_formatter_fragments(
    port: &FlowPort,
    variant_key: Option<&str>,
    bindings: &Map<String, Value>,
) -> Result<Vec<String>> {
    let formatter = port.formatter.as_ref().ok_or_else(|| {
        anyhow!(
            "Flow output port \"{}\" does not declare a formatter.",
            port.name
        )
    })?;
    let definition = resolve_port_formatter_definition(formatter, &port.name, variant_key)?;
    render_flow_prompt_template_fragments(definition, bindings)
}

/// Validates a flow port formatter declaration before runtime use.
///
/// A formatter must declare either a default template or at least one variant.
/// Every default and variant definition is validated recursively, and variant
/// keys must be non-empty after trimming.
pub fn validate_flow_port_formatter(formatter: &FlowPortFormatter) -> Result<()> {
    if formatter.default.is_none() && formatter.variants.is_empty() {
        return Err(anyhow!(
            "Flow output formatters must declare a default template or at least one variant."
        ));
    }

    if let Some(definition) = &formatter.default {
        validate_flow_prompt_template_definition(definition)?;
    }

    for (key, definition) in &formatter.variants {
        if key.trim().is_empty() {
            return Err(anyhow!(
                "Flow output formatter variants must have non-empty keys."
            ));
        }
        validate_flow_prompt_template_definition(definition)?;
    }

    Ok(())
}

/// Renders a validated prompt-template definition against the provided binding
/// map.
///
/// This is a thin convenience wrapper around node rendering and performs no
/// additional binding validation beyond what the node renderer requires.
fn render_flow_prompt_template(
    definition: &FlowPromptTemplateDefinition,
    bindings: &Map<String, Value>,
) -> Result<String> {
    render_flow_prompt_template_node(&definition.template, bindings)
}

fn render_flow_prompt_template_fragments(
    definition: &FlowPromptTemplateDefinition,
    bindings: &Map<String, Value>,
) -> Result<Vec<String>> {
    match &definition.template {
        FlowPromptTemplateNode::Sequence(node) => {
            let mut fragments = Vec::new();
            for item in &node.items {
                fragments.extend(render_top_level_template_fragments(item, bindings)?);
            }
            Ok(fragments)
        }
        FlowPromptTemplateNode::List(node) => render_top_level_list_fragments(node, bindings),
        template => render_top_level_template_fragments(template, bindings),
    }
}

/// Renders a prompt-template node according to its structural semantics.
///
/// Text nodes interpolate placeholders, sequence nodes join only non-empty child
/// fragments, optional nodes omit themselves for absent or empty bindings, and
/// list nodes omit themselves for absent or non-array bindings while rendering
/// each array item with per-item bindings merged over the ambient scope.
fn render_flow_prompt_template_node(
    template: &FlowPromptTemplateNode,
    bindings: &Map<String, Value>,
) -> Result<String> {
    match template {
        FlowPromptTemplateNode::Text(node) => render_template_string(&node.text, bindings),
        FlowPromptTemplateNode::Sequence(node) => {
            let mut rendered = Vec::new();
            for item in &node.items {
                let value = render_flow_prompt_template_node(item, bindings)?;
                if !value.is_empty() {
                    rendered.push(value);
                }
            }
            let separator = resolve_template_separator(&node.separator, bindings)?;
            Ok(rendered.join(separator.as_str()))
        }
        FlowPromptTemplateNode::Optional(node) => {
            if bindings
                .get(node.binding.as_str())
                .is_some_and(value_is_non_empty)
            {
                render_flow_prompt_template_node(&node.template, bindings)
            } else {
                Ok(String::new())
            }
        }
        FlowPromptTemplateNode::List(node) => {
            let Some(items) = bindings
                .get(node.binding.as_str())
                .and_then(Value::as_array)
            else {
                return Ok(String::new());
            };
            let mut rendered_items = Vec::new();
            for item in items {
                let item_binding_map = build_item_binding_map(item, &node.item_bindings)?;
                let mut merged = bindings.clone();
                merged.extend(item_binding_map);
                let rendered = render_flow_prompt_template_node(&node.template, &merged)?;
                if !rendered.is_empty() {
                    rendered_items.push(rendered);
                }
            }
            let separator = resolve_template_separator(&node.separator, bindings)?;
            Ok(rendered_items.join(separator.as_str()))
        }
    }
}

/// Renders a template separator string through the placeholder substitution
/// so YAML authors can write `separator: "{plain_fragment_delimiter}"`
/// (etc.) and have the configured delimiter substituted at render time.
/// Literal separators (no `{` placeholder) are returned untouched and never
/// trigger placeholder errors, so existing formatter blocks keep working.
fn resolve_template_separator(separator: &str, bindings: &Map<String, Value>) -> Result<String> {
    if separator.contains('{') {
        render_template_string(separator, bindings)
    } else {
        Ok(separator.to_string())
    }
}

fn render_top_level_template_fragments(
    template: &FlowPromptTemplateNode,
    bindings: &Map<String, Value>,
) -> Result<Vec<String>> {
    match template {
        FlowPromptTemplateNode::List(node) => render_top_level_list_fragments(node, bindings),
        _ => {
            let rendered = render_flow_prompt_template_node(template, bindings)?;
            Ok((!rendered.is_empty())
                .then_some(rendered)
                .into_iter()
                .collect())
        }
    }
}

fn render_top_level_list_fragments(
    node: &crate::FlowPromptListTemplateNode,
    bindings: &Map<String, Value>,
) -> Result<Vec<String>> {
    let Some(items) = bindings
        .get(node.binding.as_str())
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };

    let mut fragments = Vec::new();
    for item in items {
        let item_binding_map = build_item_binding_map(item, &node.item_bindings)?;
        let mut merged = bindings.clone();
        merged.extend(item_binding_map);
        let rendered = render_flow_prompt_template_node(&node.template, &merged)?;
        if !rendered.is_empty() {
            fragments.push(rendered);
        }
    }
    Ok(fragments)
}

/// Projects one list item into the binding map expected by a list-template
/// node.
///
/// Object items must contain every declared item binding. Scalar items are only
/// allowed when exactly one item binding is declared. Declaring no item
/// bindings yields an empty projection map.
fn build_item_binding_map(item: &Value, item_bindings: &[String]) -> Result<Map<String, Value>> {
    if item_bindings.is_empty() {
        return Ok(Map::new());
    }

    match item {
        Value::Object(object) => item_bindings
            .iter()
            .map(|binding| {
                object
                    .get(binding)
                    .cloned()
                    .map(|value| (binding.clone(), value))
                    .ok_or_else(|| {
                        anyhow!(
                            "Flow list item is missing expected binding \"{}\".",
                            binding
                        )
                    })
            })
            .collect(),
        _ if item_bindings.len() == 1 => {
            Ok(Map::from_iter([(item_bindings[0].clone(), item.clone())]))
        }
        _ => Err(anyhow!(
            "Flow list items must be objects unless exactly one item binding is declared."
        )),
    }
}

/// Resolves the concrete formatter definition to use for a port render.
///
/// When a variant key is supplied, that exact variant must exist. Without a
/// variant key, the formatter must declare a default definition. Missing
/// definitions are reported against the caller-facing port name.
fn resolve_port_formatter_definition<'a>(
    formatter: &'a FlowPortFormatter,
    port_name: &str,
    variant_key: Option<&str>,
) -> Result<&'a FlowPromptTemplateDefinition> {
    match variant_key {
        Some(key) => formatter.variants.get(key).ok_or_else(|| {
            anyhow!(
                "Flow output port \"{}\" does not declare formatter variant \"{}\".",
                port_name,
                key
            )
        }),
        None => formatter.default.as_ref().ok_or_else(|| {
            anyhow!(
                "Flow output port \"{}\" does not declare a default formatter.",
                port_name
            )
        }),
    }
}

/// Validates one prompt-template definition against its declared binding set.
///
/// Binding names must be non-empty and unique. The nested template is then
/// validated recursively so every referenced binding is declared exactly where
/// it is allowed to be used.
fn validate_flow_prompt_template_definition(
    definition: &FlowPromptTemplateDefinition,
) -> Result<()> {
    let mut bindings = BTreeSet::new();
    for binding in &definition.bindings {
        if binding.trim().is_empty() {
            return Err(anyhow!(
                "Flow formatter bindings must have non-empty names."
            ));
        }
        if !bindings.insert(binding.clone()) {
            return Err(anyhow!(
                "Flow formatter bindings must not contain duplicates."
            ));
        }
    }

    validate_flow_prompt_template_node(&definition.template, &bindings)
}

/// Validates one prompt-template node against the currently available binding
/// names.
///
/// Text placeholders must be declared, optional and list nodes must reference
/// declared non-empty bindings, and list item bindings must be non-empty and
/// unique. Item bindings extend the binding scope only inside the nested list
/// item template.
fn validate_flow_prompt_template_node(
    template: &FlowPromptTemplateNode,
    bindings: &BTreeSet<String>,
) -> Result<()> {
    match template {
        FlowPromptTemplateNode::Text(node) => {
            for capture in placeholder_regex().captures_iter(node.text.as_str()) {
                if let Some(name) = capture.get(1).map(|inner| inner.as_str()) {
                    if !bindings.contains(name) {
                        return Err(anyhow!(
                            "Flow formatter template referenced undeclared binding \"{}\".",
                            name
                        ));
                    }
                }
            }
            Ok(())
        }
        FlowPromptTemplateNode::Sequence(node) => node
            .items
            .iter()
            .try_for_each(|item| validate_flow_prompt_template_node(item, bindings)),
        FlowPromptTemplateNode::Optional(node) => {
            if node.binding.trim().is_empty() {
                return Err(anyhow!(
                    "Flow formatter optional nodes must name a binding."
                ));
            }
            if !bindings.contains(node.binding.as_str()) {
                return Err(anyhow!(
                    "Flow formatter optional node referenced undeclared binding \"{}\".",
                    node.binding
                ));
            }
            validate_flow_prompt_template_node(&node.template, bindings)
        }
        FlowPromptTemplateNode::List(node) => {
            if node.binding.trim().is_empty() {
                return Err(anyhow!("Flow formatter list nodes must name a binding."));
            }
            if !bindings.contains(node.binding.as_str()) {
                return Err(anyhow!(
                    "Flow formatter list node referenced undeclared binding \"{}\".",
                    node.binding
                ));
            }
            let mut next_bindings = bindings.clone();
            let mut item_binding_names = BTreeSet::new();
            for item_binding in &node.item_bindings {
                if item_binding.trim().is_empty() {
                    return Err(anyhow!(
                        "Flow formatter item bindings must have non-empty names."
                    ));
                }
                if !item_binding_names.insert(item_binding.clone()) {
                    return Err(anyhow!(
                        "Flow formatter item bindings must not contain duplicates."
                    ));
                }
                next_bindings.insert(item_binding.clone());
            }
            validate_flow_prompt_template_node(&node.template, &next_bindings)
        }
    }
}

/// Defines whether a JSON value counts as "present" for optional template
/// nodes.
///
/// `null`, `false`, blank strings, and empty arrays or objects are considered
/// empty. Numbers and non-empty structured values are considered present.
fn value_is_non_empty(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(_) => true,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
    }
}

/// Renders one inline placeholder value for string interpolation.
///
/// Only scalar values can be embedded directly into a string template. `null`
/// renders as an empty string, while arrays and objects are rejected so callers
/// must use structured rendering instead.
fn render_inline_value(name: &str, value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Null => Ok(String::new()),
        Value::Array(_) | Value::Object(_) => Err(anyhow!(
            "Prompt placeholder \"{name}\" must resolve to a scalar when embedded inside a string template."
        )),
    }
}

#[cfg(test)]
pub fn render_template_value(template: &Value, bindings: &Map<String, Value>) -> Result<Value> {
    match template {
        Value::String(value) => {
            if let Some(name) = exact_placeholder_name(value) {
                let replacement = bindings.get(name).ok_or_else(|| {
                    anyhow!("Prompt template referenced missing placeholder \"{name}\".")
                })?;
                Ok(replacement.clone())
            } else {
                Ok(Value::String(render_template_string(value, bindings)?))
            }
        }
        Value::Array(items) => items
            .iter()
            .map(|item| render_template_value(item, bindings))
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        Value::Object(object) => {
            let mut rendered = serde_json::Map::with_capacity(object.len());
            for (key, value) in object {
                rendered.insert(key.clone(), render_template_value(value, bindings)?);
            }
            Ok(Value::Object(rendered))
        }
        _ => Ok(template.clone()),
    }
}

/// Returns the shared placeholder matcher used by both rendering and binding
/// collection.
///
/// The supported placeholder syntax is `{identifier}` where the identifier is
/// limited to ASCII letters, digits, and underscores.
fn placeholder_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\{([A-Za-z0-9_]+)\}").expect("prompt placeholder regex"))
}

#[cfg(test)]
fn exact_placeholder_name(template: &str) -> Option<&str> {
    exact_placeholder_regex()
        .captures(template)
        .and_then(|capture| capture.get(1).map(|inner| inner.as_str()))
}

#[cfg(test)]
fn exact_placeholder_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^\{([A-Za-z0-9_]+)\}$").expect("exact prompt placeholder regex")
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_item_binding_map, placeholder_regex, render_flow_prompt_template,
        render_flow_prompt_template_node, render_inline_value, render_port_formatter,
        render_template_string, render_template_value, resolve_port_formatter_definition,
        validate_flow_port_formatter, validate_flow_prompt_template_definition,
        validate_flow_prompt_template_node, value_is_non_empty,
    };
    use crate::{
        FlowPort, FlowPortFormatter, FlowPortFormatterKind, FlowPortKind,
        FlowPromptListTemplateNode, FlowPromptOptionalTemplateNode, FlowPromptSequenceTemplateNode,
        FlowPromptTemplateDefinition, FlowPromptTemplateNode, FlowPromptTextTemplateNode,
    };
    use serde_json::{json, Map, Value};
    use std::collections::{BTreeMap, BTreeSet};

    fn bindings() -> Map<String, Value> {
        let mut bindings = Map::new();
        bindings.insert("title".to_string(), json!("Primrose Hill"));
        bindings.insert("world".to_string(), json!({"title": "Primrose Hill"}));
        bindings.insert("tags".to_string(), json!(["night", "suspense"]));
        bindings
    }

    fn text_template(text: &str) -> FlowPromptTemplateNode {
        FlowPromptTemplateNode::Text(FlowPromptTextTemplateNode {
            text: text.to_string(),
        })
    }

    fn template_definition(
        bindings: &[&str],
        template: FlowPromptTemplateNode,
    ) -> FlowPromptTemplateDefinition {
        FlowPromptTemplateDefinition {
            bindings: bindings
                .iter()
                .map(|binding| (*binding).to_string())
                .collect(),
            template,
        }
    }

    fn formatter_port(formatter: Option<FlowPortFormatter>) -> FlowPort {
        FlowPort {
            name: "output".to_string(),
            kind: FlowPortKind::Output,
            token_type: "prompt.fragment".to_string(),
            display_class: None,
            accepted_token_types: Vec::new(),
            short_description: None,
            long_description: None,
            formatter,
            parameters: Vec::new(),
        }
    }

    #[test]
    fn renders_string_placeholders() {
        let rendered =
            render_template_string("Series title: {title}.", &bindings()).expect("rendered");
        assert_eq!(rendered, "Series title: Primrose Hill.");
    }

    #[test]
    fn render_template_string_handles_repetition_null_missing_and_malformed_placeholders() {
        let mut bindings = bindings();
        bindings.insert("count".to_string(), json!(3));
        bindings.insert("optional".to_string(), Value::Null);

        let rendered = render_template_string(
            "{title} / {title} / {count} / {optional} / { title } / {title-name}",
            &bindings,
        )
        .expect("rendered");

        assert_eq!(
            rendered,
            "Primrose Hill / Primrose Hill / 3 /  / { title } / {title-name}"
        );

        let missing_error = render_template_string("{missing}", &bindings).expect_err("missing");
        assert!(missing_error
            .to_string()
            .contains("missing placeholder \"missing\""));

        let structured_error =
            render_template_string("{world}", &bindings).expect_err("structured");
        assert!(structured_error
            .to_string()
            .contains("must resolve to a scalar"));
    }

    #[test]
    fn renders_structured_placeholders() {
        let template = json!({
            "world": "{world}",
            "summary": "Series title: {title}.",
        });
        let rendered = render_template_value(&template, &bindings()).expect("rendered");
        assert_eq!(rendered["world"]["title"], "Primrose Hill");
        assert_eq!(rendered["summary"], "Series title: Primrose Hill.");
    }

    #[test]
    fn renders_flow_port_formatters_with_optional_and_list_nodes() {
        let port = FlowPort {
            name: "output".to_string(),
            kind: FlowPortKind::Output,
            token_type: "prompt.fragment".to_string(),
            display_class: None,
            accepted_token_types: Vec::new(),
            short_description: None,
            long_description: None,
            formatter: Some(FlowPortFormatter {
                kind: FlowPortFormatterKind::PromptTemplate,
                default: Some(FlowPromptTemplateDefinition {
                    bindings: vec!["items".to_string()],
                    template: FlowPromptTemplateNode::List(FlowPromptListTemplateNode {
                        binding: "items".to_string(),
                        separator: "\n".to_string(),
                        item_bindings: vec!["item".to_string(), "tag".to_string()],
                        template: Box::new(FlowPromptTemplateNode::Sequence(
                            FlowPromptSequenceTemplateNode {
                                separator: ", ".to_string(),
                                items: vec![
                                    FlowPromptTemplateNode::Text(FlowPromptTextTemplateNode {
                                        text: "- {item}".to_string(),
                                    }),
                                    FlowPromptTemplateNode::Optional(
                                        FlowPromptOptionalTemplateNode {
                                            binding: "tag".to_string(),
                                            template: Box::new(FlowPromptTemplateNode::Text(
                                                FlowPromptTextTemplateNode {
                                                    text: "tag: {tag}".to_string(),
                                                },
                                            )),
                                        },
                                    ),
                                ],
                            },
                        )),
                    }),
                }),
                variants: BTreeMap::new(),
            }),
            parameters: Vec::new(),
        };
        let bindings = Map::from_iter([(
            "items".to_string(),
            json!([
                {"item": "Ada", "tag": "archivist"},
                {"item": "Iris", "tag": ""},
            ]),
        )]);

        let rendered = render_port_formatter(&port, None, &bindings).expect("rendered");

        assert_eq!(rendered, "- Ada, tag: archivist\n- Iris");
    }

    #[test]
    fn render_port_formatter_and_definition_resolution_follow_their_contracts() {
        let default = template_definition(&["title"], text_template("default {title}"));
        let variant = template_definition(&["title"], text_template("variant {title}"));
        let formatter = FlowPortFormatter {
            kind: FlowPortFormatterKind::PromptTemplate,
            default: Some(default.clone()),
            variants: BTreeMap::from_iter([("alt".to_string(), variant.clone())]),
        };
        let port = formatter_port(Some(formatter.clone()));

        let rendered_default = render_port_formatter(&port, None, &bindings()).expect("default");
        assert_eq!(rendered_default, "default Primrose Hill");

        let rendered_variant =
            render_port_formatter(&port, Some("alt"), &bindings()).expect("variant");
        assert_eq!(rendered_variant, "variant Primrose Hill");

        let resolved_default =
            resolve_port_formatter_definition(&formatter, "output", None).expect("definition");
        assert_eq!(resolved_default.template, default.template);

        let resolved_variant = resolve_port_formatter_definition(&formatter, "output", Some("alt"))
            .expect("variant definition");
        assert_eq!(resolved_variant.template, variant.template);

        let unknown_variant_error =
            render_port_formatter(&port, Some("missing"), &bindings()).expect_err("variant");
        assert!(unknown_variant_error
            .to_string()
            .contains("does not declare formatter variant"));

        let missing_formatter_port = formatter_port(None);
        let missing_formatter_error =
            render_port_formatter(&missing_formatter_port, None, &bindings()).expect_err("port");
        assert!(missing_formatter_error
            .to_string()
            .contains("does not declare a formatter"));

        let variant_only_formatter = FlowPortFormatter {
            kind: FlowPortFormatterKind::PromptTemplate,
            default: None,
            variants: BTreeMap::from_iter([("alt".to_string(), variant)]),
        };
        let missing_default_error =
            resolve_port_formatter_definition(&variant_only_formatter, "output", None)
                .expect_err("default");
        assert!(missing_default_error
            .to_string()
            .contains("does not declare a default formatter"));
    }

    #[test]
    fn formatter_validation_rejects_empty_blank_and_invalid_declarations() {
        let empty_formatter = FlowPortFormatter {
            kind: FlowPortFormatterKind::PromptTemplate,
            default: None,
            variants: BTreeMap::new(),
        };
        assert!(validate_flow_port_formatter(&empty_formatter).is_err());

        let formatter_with_blank_variant = FlowPortFormatter {
            kind: FlowPortFormatterKind::PromptTemplate,
            default: Some(template_definition(&["title"], text_template("{title}"))),
            variants: BTreeMap::from_iter([(
                "   ".to_string(),
                template_definition(&["title"], text_template("{title}")),
            )]),
        };
        assert!(validate_flow_port_formatter(&formatter_with_blank_variant).is_err());

        let formatter_with_invalid_variant = FlowPortFormatter {
            kind: FlowPortFormatterKind::PromptTemplate,
            default: Some(template_definition(&["items"], text_template("{items}"))),
            variants: BTreeMap::from_iter([(
                "alt".to_string(),
                template_definition(&["items"], text_template("{missing}")),
            )]),
        };
        assert!(validate_flow_port_formatter(&formatter_with_invalid_variant).is_err());
    }

    #[test]
    fn render_flow_prompt_template_helpers_cover_sequence_optional_and_list_contracts() {
        let sequence = FlowPromptTemplateNode::Sequence(FlowPromptSequenceTemplateNode {
            separator: " | ".to_string(),
            items: vec![
                text_template("{title}"),
                FlowPromptTemplateNode::Optional(FlowPromptOptionalTemplateNode {
                    binding: "empty".to_string(),
                    template: Box::new(text_template("unused")),
                }),
                text_template("tail"),
            ],
        });
        let sequence_bindings = Map::from_iter([
            ("title".to_string(), json!("Primrose Hill")),
            ("empty".to_string(), json!("   ")),
        ]);
        let rendered_sequence =
            render_flow_prompt_template_node(&sequence, &sequence_bindings).expect("sequence");
        assert_eq!(rendered_sequence, "Primrose Hill | tail");

        let optional_template = FlowPromptTemplateNode::Optional(FlowPromptOptionalTemplateNode {
            binding: "binding".to_string(),
            template: Box::new(text_template("present")),
        });
        for (value, expected) in [
            (Value::Null, ""),
            (json!(false), ""),
            (json!(0), "present"),
            (json!("   "), ""),
            (json!("x"), "present"),
            (json!([]), ""),
            (json!(["x"]), "present"),
            (json!({}), ""),
            (json!({"x": 1}), "present"),
        ] {
            let bindings = Map::from_iter([("binding".to_string(), value)]);
            let rendered =
                render_flow_prompt_template_node(&optional_template, &bindings).expect("optional");
            assert_eq!(rendered, expected);
        }

        let list_template = FlowPromptTemplateNode::List(FlowPromptListTemplateNode {
            binding: "items".to_string(),
            separator: ", ".to_string(),
            item_bindings: vec!["item".to_string(), "tag".to_string()],
            template: Box::new(FlowPromptTemplateNode::Sequence(
                FlowPromptSequenceTemplateNode {
                    separator: " / ".to_string(),
                    items: vec![
                        text_template("{item}"),
                        FlowPromptTemplateNode::Optional(FlowPromptOptionalTemplateNode {
                            binding: "tag".to_string(),
                            template: Box::new(text_template("{tag}")),
                        }),
                    ],
                },
            )),
        });
        let rendered_list = render_flow_prompt_template_node(
            &list_template,
            &Map::from_iter([(
                "items".to_string(),
                json!([
                    {"item": "Ada", "tag": "archivist"},
                    {"item": "Iris", "tag": ""}
                ]),
            )]),
        )
        .expect("list");
        assert_eq!(rendered_list, "Ada / archivist, Iris");

        let missing_list = render_flow_prompt_template_node(&list_template, &Map::new())
            .expect("missing list binding");
        assert_eq!(missing_list, "");

        let non_array_list = render_flow_prompt_template_node(
            &list_template,
            &Map::from_iter([("items".to_string(), json!("not-an-array"))]),
        )
        .expect("non array");
        assert_eq!(non_array_list, "");

        let scalar_list_template = FlowPromptTemplateNode::List(FlowPromptListTemplateNode {
            binding: "items".to_string(),
            separator: ", ".to_string(),
            item_bindings: vec!["item".to_string()],
            template: Box::new(text_template("{item}")),
        });
        let scalar_list = render_flow_prompt_template_node(
            &scalar_list_template,
            &Map::from_iter([("items".to_string(), json!(["Ada", "Iris"]))]),
        )
        .expect("scalar list");
        assert_eq!(scalar_list, "Ada, Iris");

        let missing_item_field = render_flow_prompt_template_node(
            &list_template,
            &Map::from_iter([("items".to_string(), json!([{"item": "Ada"}]))]),
        )
        .expect_err("item field");
        assert!(missing_item_field
            .to_string()
            .contains("missing expected binding"));

        let bad_scalar_list_template = FlowPromptTemplateNode::List(FlowPromptListTemplateNode {
            binding: "items".to_string(),
            separator: ", ".to_string(),
            item_bindings: vec!["item".to_string(), "tag".to_string()],
            template: Box::new(text_template("{item}")),
        });
        let scalar_item_error = render_flow_prompt_template_node(
            &bad_scalar_list_template,
            &Map::from_iter([("items".to_string(), json!(["Ada"]))]),
        )
        .expect_err("scalar item");
        assert!(scalar_item_error
            .to_string()
            .contains("must be objects unless exactly one item binding"));

        let overriding_list_template = FlowPromptTemplateNode::List(FlowPromptListTemplateNode {
            binding: "items".to_string(),
            separator: ", ".to_string(),
            item_bindings: vec!["item".to_string()],
            template: Box::new(text_template("{item}")),
        });
        let overriding_bindings = Map::from_iter([
            ("item".to_string(), json!("ambient")),
            ("items".to_string(), json!([{"item": "local"}])),
        ]);
        let overridden =
            render_flow_prompt_template_node(&overriding_list_template, &overriding_bindings)
                .expect("override");
        assert_eq!(overridden, "local");
        assert_eq!(
            overriding_bindings.get("item").expect("ambient"),
            &json!("ambient")
        );

        let rendered_definition = render_flow_prompt_template(
            &template_definition(&["title"], text_template("Hello {title}")),
            &bindings(),
        )
        .expect("definition");
        assert_eq!(rendered_definition, "Hello Primrose Hill");
    }

    #[test]
    fn build_item_binding_map_follows_object_scalar_and_empty_binding_contracts() {
        let empty = build_item_binding_map(&json!("Ada"), &[]).expect("empty");
        assert!(empty.is_empty());

        let object = build_item_binding_map(
            &json!({"item": "Ada", "tag": "archivist"}),
            &["item".to_string(), "tag".to_string()],
        )
        .expect("object");
        assert_eq!(object["item"], "Ada");
        assert_eq!(object["tag"], "archivist");

        let scalar = build_item_binding_map(&json!("Ada"), &["item".to_string()]).expect("item");
        assert_eq!(scalar["item"], "Ada");

        let scalar_error =
            build_item_binding_map(&json!("Ada"), &["item".to_string(), "tag".to_string()])
                .expect_err("scalar");
        assert!(scalar_error
            .to_string()
            .contains("must be objects unless exactly one item binding"));

        let object_error = build_item_binding_map(
            &json!({"item": "Ada"}),
            &["item".to_string(), "tag".to_string()],
        )
        .expect_err("missing key");
        assert!(object_error
            .to_string()
            .contains("missing expected binding \"tag\""));
    }

    #[test]
    fn template_definition_and_node_validation_follow_their_contracts() {
        let valid_definition = template_definition(
            &["items"],
            FlowPromptTemplateNode::List(FlowPromptListTemplateNode {
                binding: "items".to_string(),
                separator: "\n".to_string(),
                item_bindings: vec!["item".to_string()],
                template: Box::new(text_template("{item}")),
            }),
        );
        validate_flow_prompt_template_definition(&valid_definition).expect("valid");

        let blank_binding = template_definition(&["   "], text_template("{title}"));
        assert!(validate_flow_prompt_template_definition(&blank_binding).is_err());

        let duplicate_binding = template_definition(&["title", "title"], text_template("{title}"));
        assert!(validate_flow_prompt_template_definition(&duplicate_binding).is_err());

        let undeclared_placeholder = template_definition(&["title"], text_template("{missing}"));
        assert!(validate_flow_prompt_template_definition(&undeclared_placeholder).is_err());

        let bindings = BTreeSet::from_iter(["title".to_string(), "items".to_string()]);
        let blank_optional = FlowPromptTemplateNode::Optional(FlowPromptOptionalTemplateNode {
            binding: "   ".to_string(),
            template: Box::new(text_template("{title}")),
        });
        assert!(validate_flow_prompt_template_node(&blank_optional, &bindings).is_err());

        let undeclared_optional =
            FlowPromptTemplateNode::Optional(FlowPromptOptionalTemplateNode {
                binding: "missing".to_string(),
                template: Box::new(text_template("{title}")),
            });
        assert!(validate_flow_prompt_template_node(&undeclared_optional, &bindings).is_err());

        let blank_list_binding = FlowPromptTemplateNode::List(FlowPromptListTemplateNode {
            binding: "   ".to_string(),
            separator: ", ".to_string(),
            item_bindings: vec!["item".to_string()],
            template: Box::new(text_template("{item}")),
        });
        assert!(validate_flow_prompt_template_node(&blank_list_binding, &bindings).is_err());

        let undeclared_list_binding = FlowPromptTemplateNode::List(FlowPromptListTemplateNode {
            binding: "missing".to_string(),
            separator: ", ".to_string(),
            item_bindings: vec!["item".to_string()],
            template: Box::new(text_template("{item}")),
        });
        assert!(validate_flow_prompt_template_node(&undeclared_list_binding, &bindings).is_err());

        let blank_item_binding = FlowPromptTemplateNode::List(FlowPromptListTemplateNode {
            binding: "items".to_string(),
            separator: ", ".to_string(),
            item_bindings: vec!["   ".to_string()],
            template: Box::new(text_template("{item}")),
        });
        assert!(validate_flow_prompt_template_node(&blank_item_binding, &bindings).is_err());

        let duplicate_item_binding = FlowPromptTemplateNode::List(FlowPromptListTemplateNode {
            binding: "items".to_string(),
            separator: ", ".to_string(),
            item_bindings: vec!["item".to_string(), "item".to_string()],
            template: Box::new(text_template("{item}")),
        });
        assert!(validate_flow_prompt_template_node(&duplicate_item_binding, &bindings).is_err());
    }

    #[test]
    fn value_truthiness_inline_rendering_and_placeholder_matching_follow_their_contracts() {
        for (value, expected) in [
            (Value::Null, false),
            (json!(false), false),
            (json!(true), true),
            (json!(0), true),
            (json!(""), false),
            (json!("   "), false),
            (json!("title"), true),
            (json!([]), false),
            (json!(["x"]), true),
            (json!({}), false),
            (json!({"x": 1}), true),
        ] {
            assert_eq!(value_is_non_empty(&value), expected);
        }

        assert_eq!(
            render_inline_value("title", &json!("Primrose")).expect("string"),
            "Primrose"
        );
        assert_eq!(
            render_inline_value("count", &json!(7)).expect("number"),
            "7"
        );
        assert_eq!(
            render_inline_value("enabled", &json!(true)).expect("bool"),
            "true"
        );
        assert_eq!(
            render_inline_value("empty", &Value::Null).expect("null"),
            ""
        );
        assert!(render_inline_value("world", &json!({"title": "Primrose"})).is_err());
        assert!(render_inline_value("tags", &json!(["night"])).is_err());

        let matches: Vec<_> = placeholder_regex()
            .captures_iter("A {title} B {title_2} C { title } D {title-name}")
            .filter_map(|capture| capture.get(1).map(|name| name.as_str().to_string()))
            .collect();
        assert_eq!(matches, vec!["title".to_string(), "title_2".to_string()]);
    }
}
