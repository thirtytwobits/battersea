//! Versioned, lossless flow source. See `contracts/text-source.md`.
use crate::catalog::{node_implements_interface, FLOW_NODE_ACTIVATE_INTERFACE};
use crate::document::{inspect_value, DocumentInspection, FLOW_DOCUMENT_VERSION};
use crate::{FlowDocument, FlowEdgeKind, FlowNodeDefinition};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fmt;

pub const FLOW_DSL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}
impl std::error::Error for ParseError {}

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

/// Export authored values without consulting or materialising catalogue defaults.
pub fn export_flow_dsl(flow: &FlowDocument) -> anyhow::Result<String> {
    anyhow::ensure!(
        flow.version == FLOW_DOCUMENT_VERSION,
        "Unsupported flow document version {}",
        flow.version
    );
    for ids in [
        flow.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        flow.edges.iter().map(|e| e.id.as_str()).collect(),
    ] {
        let mut seen = HashSet::new();
        for id in ids {
            anyhow::ensure!(seen.insert(id), "Duplicate identity {id:?}");
        }
    }
    let mut value = serde_json::to_value(flow)?;
    value.sort_all_objects();
    let object = value.as_object_mut().expect("FlowDocument is an object");
    let key = object.remove("flow_key").unwrap();
    let nodes = object.remove("nodes").unwrap();
    let edges = object.remove("edges").unwrap();
    let mut source = format!("flow {FLOW_DSL_VERSION} {key} {{\n");
    for (name, value) in object {
        source.push_str(&format!("  {name} = {value};\n"));
    }
    for mut node in nodes.as_array().unwrap().iter().cloned() {
        let attributes = node.as_object_mut().unwrap();
        let id = attributes.remove("id").unwrap();
        let definition = attributes.remove("definition_name").unwrap();
        source.push_str(&format!("  node {id} {definition} {node}\n"));
    }
    for (authored, mut edge) in flow
        .edges
        .iter()
        .zip(edges.as_array().unwrap().iter().cloned())
    {
        let attributes = edge.as_object_mut().unwrap();
        let id = attributes.remove("id").unwrap();
        let from = attributes.remove("source_node_id").unwrap();
        let from_port = attributes.remove("source_port").unwrap();
        let to = attributes.remove("target_node_id").unwrap();
        let to_port = attributes.remove("target_port").unwrap();
        attributes.remove("kind");
        let connector = match authored.kind {
            FlowEdgeKind::Token => "->",
            FlowEdgeKind::Signal => "~>",
        };
        source.push_str(&format!(
            "  edge {id} {from}.{from_port} {connector} {to}.{to_port} {edge}\n"
        ));
    }
    source.push_str("}\n");
    Ok(source)
}

/// Parse a source document. Graph validation and document upgrades are explicit operations.
pub fn parse_flow_dsl(source: &str) -> Result<FlowDocument, ParseError> {
    let mut parser = Parser { source, offset: 0 };
    let start = parser.position();
    if parser.word()? != "flow" {
        return Err(parser.error_at(start, "Expected flow declaration"));
    }
    let version_at = parser.position();
    let version = parser.word()?;
    if version != FLOW_DSL_VERSION.to_string() {
        return Err(parser.error_at(
            version_at,
            format!("Unsupported flow syntax version {version}"),
        ));
    }
    let key = parser.string()?;
    parser.expect("{")?;
    let mut document = Map::new();
    document.insert("flow_key".into(), Value::String(key));
    let mut positions = HashMap::new();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut node_ids = HashSet::new();
    let mut edge_ids = HashSet::new();
    loop {
        let at = parser.position();
        if parser.consume("}") {
            break;
        }
        let word = parser.word()?;
        match word.as_str() {
            "node" => {
                let id = parser.string()?;
                if !node_ids.insert(id.clone()) {
                    return Err(parser.error_at(at, format!("Duplicate node ID {id:?}")));
                }
                let definition = parser.string()?;
                let mut attributes = parser.object()?;
                parser.insert(&mut attributes, "id", Value::String(id), at)?;
                parser.insert(
                    &mut attributes,
                    "definition_name",
                    Value::String(definition),
                    at,
                )?;
                positions.insert(format!("nodes[{}]", nodes.len()), at);
                nodes.push(Value::Object(attributes));
            }
            "edge" => {
                let id = parser.string()?;
                if !edge_ids.insert(id.clone()) {
                    return Err(parser.error_at(at, format!("Duplicate edge ID {id:?}")));
                }
                let from = parser.string()?;
                parser.expect(".")?;
                let from_port = parser.string()?;
                let kind = if parser.consume("->") {
                    "token"
                } else {
                    parser.expect("~>")?;
                    "signal"
                };
                let to = parser.string()?;
                parser.expect(".")?;
                let to_port = parser.string()?;
                let mut attributes = parser.object()?;
                for (key, value) in [
                    ("id", id),
                    ("source_node_id", from),
                    ("source_port", from_port),
                    ("target_node_id", to),
                    ("target_port", to_port),
                    ("kind", kind.into()),
                ] {
                    parser.insert(&mut attributes, key, Value::String(value), at)?;
                }
                positions.insert(format!("edges[{}]", edges.len()), at);
                edges.push(Value::Object(attributes));
            }
            "nodes" | "edges" | "flow_key" => {
                return Err(
                    parser.error_at(at, format!("{word} must be declared using source syntax"))
                )
            }
            _ => {
                parser.expect("=")?;
                let value = parser.json()?;
                parser.expect(";")?;
                parser.insert(&mut document, &word, value, at)?;
                positions.insert(word, at);
            }
        }
    }
    parser.skip();
    if parser.offset != source.len() {
        return Err(parser.error("Unexpected text after flow"));
    }
    document.insert("nodes".into(), Value::Array(nodes));
    document.insert("edges".into(), Value::Array(edges));
    let inspection = inspect_value(Value::Object(document)).map_err(|error| {
        let path = error
            .downcast_ref::<serde_path_to_error::Error<serde_json::Error>>()
            .map(|error| error.path().to_string())
            .unwrap_or_else(|| "version".into());
        let declaration = path.split('.').next().unwrap_or("");
        parser.error_at(
            *positions.get(declaration).unwrap_or(&start),
            error.to_string(),
        )
    })?;
    match inspection {
        DocumentInspection::Supported(flow) => Ok(*flow),
        DocumentInspection::Incompatible { version, .. } => Err(parser.error_at(
            *positions.get("version").unwrap_or(&start),
            format!("Unsupported flow document version {version}; supported version is {FLOW_DOCUMENT_VERSION}"),
        )),
    }
}

struct Parser<'a> {
    source: &'a str,
    offset: usize,
}
impl Parser<'_> {
    fn skip(&mut self) {
        loop {
            let rest = &self.source[self.offset..];
            self.offset += rest.len() - rest.trim_start().len();
            if self.source[self.offset..].starts_with("//") {
                self.offset += self.source[self.offset..]
                    .find('\n')
                    .unwrap_or(self.source.len() - self.offset);
            } else {
                break;
            }
        }
    }
    fn position(&mut self) -> usize {
        self.skip();
        self.offset
    }
    fn error(&self, message: impl Into<String>) -> ParseError {
        self.error_at(self.offset, message)
    }
    fn error_at(&self, at: usize, message: impl Into<String>) -> ParseError {
        let prefix = &self.source[..at];
        ParseError {
            line: prefix.bytes().filter(|c| *c == b'\n').count() + 1,
            column: prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
            message: message.into(),
        }
    }
    fn consume(&mut self, token: &str) -> bool {
        self.skip();
        if self.source[self.offset..].starts_with(token) {
            self.offset += token.len();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, token: &str) -> Result<(), ParseError> {
        if self.consume(token) {
            Ok(())
        } else {
            Err(self.error(format!("Expected {token:?}")))
        }
    }
    fn word(&mut self) -> Result<String, ParseError> {
        let start = self.position();
        while self
            .source
            .as_bytes()
            .get(self.offset)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
        {
            self.offset += 1;
        }
        if self.offset == start {
            return Err(self.error("Expected a declaration"));
        }
        Ok(self.source[start..self.offset].into())
    }
    fn string(&mut self) -> Result<String, ParseError> {
        let at = self.position();
        if !self.source[self.offset..].starts_with('"') {
            return Err(self.error("Expected a quoted identifier"));
        }
        match self.json()? {
            Value::String(value) => Ok(value),
            _ => Err(self.error_at(at, "Expected a quoted identifier")),
        }
    }
    fn object(&mut self) -> Result<Map<String, Value>, ParseError> {
        let at = self.position();
        match self.json()? {
            Value::Object(value) => Ok(value),
            _ => Err(self.error_at(at, "Expected a JSON attribute object")),
        }
    }
    fn insert(
        &self,
        object: &mut Map<String, Value>,
        key: &str,
        value: Value,
        at: usize,
    ) -> Result<(), ParseError> {
        if object.contains_key(key) {
            return Err(self.error_at(at, format!("Duplicate or reserved attribute {key:?}")));
        }
        object.insert(key.into(), value);
        Ok(())
    }
    fn json(&mut self) -> Result<Value, ParseError> {
        let start = self.position();
        let bytes = self.source.as_bytes();
        let first = *bytes
            .get(start)
            .ok_or_else(|| self.error("Expected a JSON value"))?;
        let compound = first == b'{' || first == b'[';
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for (relative, ch) in self.source[start..].char_indices() {
            let index = start + relative;
            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
            } else if ch == '"' {
                in_string = true;
            } else if compound && (ch == '{' || ch == '[') {
                depth += 1;
            } else if compound && (ch == '}' || ch == ']') {
                depth = depth.saturating_sub(1);
            } else if !compound && first != b'"' && (ch.is_whitespace() || ch == ';' || ch == '}') {
                break;
            }
            self.offset = index + ch.len_utf8();
            if (compound && depth == 0 && !in_string) || (first == b'"' && !in_string) {
                break;
            }
        }
        let text = &self.source[start..self.offset];
        serde_json::from_str::<UniqueValue>(text)
            .map(|value| value.0)
            .map_err(|error| {
                let line_start = text
                    .split_inclusive('\n')
                    .take(error.line().saturating_sub(1))
                    .map(str::len)
                    .sum::<usize>();
                let mut at =
                    (start + line_start + error.column().saturating_sub(1)).min(self.source.len());
                while !self.source.is_char_boundary(at) {
                    at -= 1;
                }
                self.error_at(at, format!("Invalid JSON: {error}"))
            })
    }
}

// serde_json::Value accepts duplicate keys. Source must reject them at every nesting level.
struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON value with unique object keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some((key, UniqueValue(value))) =
                    map.next_entry::<String, UniqueValue>()?
                {
                    if values.insert(key.clone(), value).is_some() {
                        return Err(de::Error::custom(format!("Duplicate JSON key {key:?}")));
                    }
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}
