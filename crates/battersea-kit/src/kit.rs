//! Copyright (c) Scott A Dixon
//!
//! The kit descriptor and the name rules of `contracts/kits.md`.
use battersea_flow::{
    catalog::parse_definition_manifest, failure::NODE_FAILURE_TOKEN_TYPE, registry::Registry,
    FlowNodeDefinition, FlowPromptTemplateDefinition,
};
use serde::Deserialize;
use std::collections::BTreeMap;

/// The generic kit's name. Its class names and [`SHARED_TOKEN_TYPES`] are the shared
/// vocabulary every kit may reference.
pub const GENERIC_KIT: &str = "battersea";
/// Token types the generic kit registers and any kit may reference.
pub const SHARED_TOKEN_TYPES: &[&str] = &["prompt.fragment", "prompt.fragmentArray", "chat.raw"];
/// Token type markers and built-in types a manifest may name without registering them.
const STRUCTURAL_TOKEN_TYPES: &[&str] = &["auto", "oneof", NODE_FAILURE_TOKEN_TYPE];

/// A kit's descriptor: its name, its manifest and the token types it registers.
#[derive(Debug, Clone, Copy)]
pub struct Kit {
    /// The namespace of every handler id, token type and class name; matches
    /// `[a-z][a-z0-9_]*`.
    pub name: &'static str,
    /// The manifest of node definitions and formatter blocks.
    pub manifest: &'static str,
    /// The token types `register_token_types` registers.
    pub token_types: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KitError {
    #[error("kit name {0:?} must match [a-z][a-z0-9_]*")]
    Name(String),
    #[error("kit {kit} manifest does not parse: {message}")]
    Manifest { kit: String, message: String },
    #[error("kit {kit} registers token type {token_type:?} outside its namespace")]
    RegisteredTokenType { kit: String, token_type: String },
    #[error(
        "kit {kit} definition {class_name:?} has handler id {handler_id:?} outside its namespace"
    )]
    HandlerId {
        kit: String,
        class_name: String,
        handler_id: String,
    },
    #[error("kit {kit} class name {class_name:?} must begin with {prefix:?}")]
    ClassName {
        kit: String,
        class_name: String,
        prefix: String,
    },
    #[error("kit {kit} definition {class_name:?} references token type {token_type:?} outside its namespace and the shared vocabulary")]
    ReferencedTokenType {
        kit: String,
        class_name: String,
        token_type: String,
    },
    #[error("kit {kit} definition {class_name:?} references token type {token_type:?}, which the kit does not register")]
    UnregisteredTokenType {
        kit: String,
        class_name: String,
        token_type: String,
    },
}

/// `foo_bar2` becomes `FooBar2`.
pub fn pascal_case(name: &str) -> String {
    name.split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut characters = segment.chars();
            match characters.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + characters.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn valid_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl Kit {
    /// Whether this is the generic kit, whose vocabulary every kit shares.
    pub fn is_generic(&self) -> bool {
        self.name == GENERIC_KIT
    }

    /// The prefix of every class name: the kit's name in PascalCase.
    pub fn class_prefix(&self) -> String {
        pascal_case(self.name)
    }

    /// Whether `name` is a handler id or token type inside the kit's namespace.
    pub fn owns(&self, name: &str) -> bool {
        name.strip_prefix(self.name)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|rest| !rest.is_empty())
    }

    /// Whether a definition of this kit may name `token_type`.
    pub fn may_reference(&self, token_type: &str) -> bool {
        self.owns(token_type)
            || SHARED_TOKEN_TYPES.contains(&token_type)
            || STRUCTURAL_TOKEN_TYPES.contains(&token_type)
    }

    /// The manifest's definitions, normalised without an application registry.
    pub fn definitions(&self) -> Result<Vec<FlowNodeDefinition>, KitError> {
        parse_definition_manifest(&Registry::default(), self.manifest).map_err(|message| {
            KitError::Manifest {
                kit: self.name.into(),
                message,
            }
        })
    }

    /// Refuses a name, a registered token type, a handler id, a class name or a token
    /// reference outside the rules of `contracts/kits.md`.
    pub fn check(&self) -> Result<(), KitError> {
        if !valid_name(self.name) {
            return Err(KitError::Name(self.name.into()));
        }
        for token_type in self.token_types {
            let shared = self.is_generic() && SHARED_TOKEN_TYPES.contains(token_type);
            if !self.owns(token_type) && !shared {
                return Err(KitError::RegisteredTokenType {
                    kit: self.name.into(),
                    token_type: (*token_type).into(),
                });
            }
        }
        let prefix = self.class_prefix();
        for definition in self.definitions()? {
            let kit = self.name.to_string();
            let class_name = definition.class_name.clone();
            if !self.owns(&definition.handler_id) {
                return Err(KitError::HandlerId {
                    kit,
                    class_name,
                    handler_id: definition.handler_id,
                });
            }
            if !self.is_generic() && !class_name.starts_with(&prefix) {
                return Err(KitError::ClassName {
                    kit,
                    class_name,
                    prefix,
                });
            }
            for token_type in referenced_token_types(&definition) {
                if !self.may_reference(&token_type) {
                    return Err(KitError::ReferencedTokenType {
                        kit,
                        class_name,
                        token_type,
                    });
                }
                if self.owns(&token_type) && !self.token_types.contains(&token_type.as_str()) {
                    return Err(KitError::UnregisteredTokenType {
                        kit,
                        class_name,
                        token_type,
                    });
                }
            }
        }
        Ok(())
    }

    /// The manifest text that reaches a model, keyed by formatter block
    /// (`formatter_blocks.NAME`) and by inline port formatter
    /// (`CLASS.PORT.default` or `CLASS.PORT.VARIANT`), each as canonical JSON. An
    /// application pins the map in a snapshot test.
    pub fn model_facing_text(&self) -> Result<BTreeMap<String, String>, KitError> {
        let manifest = |message: String| KitError::Manifest {
            kit: self.name.into(),
            message,
        };
        let root: RawRoot =
            serde_yaml::from_str(self.manifest).map_err(|e| manifest(e.to_string()))?;
        let (blocks, definitions) = match root {
            RawRoot::Document(document) => (document.formatter_blocks, document.node_definitions),
            RawRoot::List(definitions) => (BTreeMap::new(), definitions),
        };
        let mut text = BTreeMap::new();
        for (name, block) in blocks {
            text.insert(
                format!("formatter_blocks.{name}"),
                serde_json::to_string(&block).map_err(|e| manifest(e.to_string()))?,
            );
        }
        for definition in definitions {
            for port in definition
                .input_ports
                .iter()
                .chain(&definition.output_ports)
            {
                let Some(formatter) = &port.formatter else {
                    continue;
                };
                let inline = formatter
                    .default
                    .iter()
                    .map(|value| ("default".to_string(), value))
                    .chain(
                        formatter
                            .variants
                            .iter()
                            .map(|(key, value)| (key.clone(), value)),
                    );
                for (key, value) in inline {
                    if value.get("ref").is_some() {
                        continue;
                    }
                    let template: FlowPromptTemplateDefinition =
                        serde_yaml::from_value(value.clone())
                            .map_err(|e| manifest(e.to_string()))?;
                    text.insert(
                        format!("{}.{}.{key}", definition.class_name, port.name),
                        serde_json::to_string(&template).map_err(|e| manifest(e.to_string()))?,
                    );
                }
            }
        }
        Ok(text)
    }
}

fn referenced_token_types(definition: &FlowNodeDefinition) -> Vec<String> {
    let mut names = Vec::new();
    for port in definition
        .input_ports
        .iter()
        .chain(&definition.output_ports)
    {
        names.push(port.token_type.clone());
        names.extend(port.accepted_token_types.iter().cloned());
    }
    for group in definition
        .dynamic_input_ports
        .iter()
        .chain(&definition.dynamic_output_ports)
    {
        names.push(group.token_type.clone());
        names.extend(group.accepted_token_types.iter().cloned());
    }
    for port in &definition.automation_ports {
        names.push(port.token_type.clone());
        names.extend(port.accepted_token_types.iter().cloned());
    }
    names
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawRoot {
    Document(RawDocument),
    List(Vec<RawDefinition>),
}
#[derive(Deserialize)]
struct RawDocument {
    #[serde(default)]
    formatter_blocks: BTreeMap<String, FlowPromptTemplateDefinition>,
    #[serde(default)]
    node_definitions: Vec<RawDefinition>,
}
#[derive(Deserialize)]
struct RawDefinition {
    class_name: String,
    #[serde(default)]
    input_ports: Vec<RawPort>,
    #[serde(default)]
    output_ports: Vec<RawPort>,
}
#[derive(Deserialize)]
struct RawPort {
    name: String,
    #[serde(default)]
    formatter: Option<RawFormatter>,
}
#[derive(Deserialize)]
struct RawFormatter {
    #[serde(default)]
    default: Option<serde_yaml::Value>,
    #[serde(default)]
    variants: BTreeMap<String, serde_yaml::Value>,
}
