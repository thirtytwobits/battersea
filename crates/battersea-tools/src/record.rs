//! The Model Context Protocol's tool record.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The Model Context Protocol revision whose tool record [`Tool`] is.
pub const MCP_PROTOCOL_REVISION: &str = "2026-07-28";

/// The `_meta` key under which a tool lists the agent profiles permitted to call it.
pub const PROFILES_META_KEY: &str = "io.github.thirtytwobits.battersea/profiles";

/// The `_meta` key under which Clapham's MCP form names an `administrative` effect.
const EFFECT_META_KEY: &str = "io.github.thirtytwobits.clapham/effect";

/// The `_meta` key under which Clapham's MCP form marks an operation that streams records.
const RECORDS_META_KEY: &str = "io.github.thirtytwobits.clapham/records";

/// A tool, as the Model Context Protocol's tool record describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tool {
    /// The tool's identity: 1 to 128 ASCII letters, digits, `_`, `-` and `.`.
    pub name: String,
    /// The name people read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub title: Option<String>,
    /// What the tool does, which a model reads to decide whether and how to call it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub description: Option<String>,
    /// The JSON Schema a call's arguments satisfy. Its root is an object schema.
    pub input_schema: Map<String, Value>,
    /// The JSON Schema the tool's structured result satisfies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Map<String, Value>")]
    pub output_schema: Option<Map<String, Value>>,
    /// What calling the tool does to its environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "ToolAnnotations")]
    pub annotations: Option<ToolAnnotations>,
    /// Icons a user interface can show for the tool.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub icons: Vec<Icon>,
    /// Metadata beyond the record, each key prefixed by its owner's reverse-DNS name.
    #[serde(rename = "_meta", default, skip_serializing_if = "Map::is_empty")]
    pub meta: Map<String, Value>,
}

impl Tool {
    /// The record Clapham's MCP server presents for `operation`: its name, title, description
    /// and schemas, every hint stated from its effect, and Clapham's `_meta` keys for an
    /// `administrative` effect and for streamed records.
    pub fn for_operation(operation: &clapham::Operation) -> Self {
        use clapham::Effect;
        let effect = operation.effect;
        let mut meta = Map::new();
        if effect == Effect::Administrative {
            meta.insert(EFFECT_META_KEY.into(), Value::from("administrative"));
        }
        if operation.streams {
            meta.insert(RECORDS_META_KEY.into(), Value::from(true));
        }
        // A boolean output schema has an exact object equivalent.
        let output_schema = operation
            .output
            .as_ref()
            .map(|schema| match schema.as_value() {
                Value::Object(object) => object.clone(),
                Value::Bool(true) => Map::new(),
                _ => Map::from_iter([("not".into(), Value::Object(Map::new()))]),
            });
        Self {
            name: operation.name.to_string(),
            title: operation.title.clone(),
            description: Some(operation.description.clone()),
            input_schema: operation.input.clone(),
            output_schema,
            annotations: Some(ToolAnnotations {
                title: None,
                read_only_hint: Some(effect == Effect::Query),
                destructive_hint: Some(effect == Effect::Destructive),
                idempotent_hint: Some(effect == Effect::Query || operation.idempotent),
                open_world_hint: Some(operation.open_world),
            }),
            icons: Vec::new(),
            meta,
        }
    }

    /// A tool named `name` whose arguments satisfy `input_schema`.
    pub fn new(name: impl Into<String>, input_schema: Map<String, Value>) -> Self {
        Self {
            name: name.into(),
            title: None,
            description: None,
            input_schema,
            output_schema: None,
            annotations: None,
            icons: Vec::new(),
            meta: Map::new(),
        }
    }

    /// The tool, titled `title`.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// The tool, described by `description`.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The tool, annotated with `annotations`.
    pub fn with_annotations(mut self, annotations: ToolAnnotations) -> Self {
        self.annotations = Some(annotations);
        self
    }

    /// The tool, permitted to the agent profiles `profiles`.
    pub fn with_profiles<I, S>(mut self, profiles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let profiles = profiles
            .into_iter()
            .map(Into::into)
            .collect::<Vec<String>>();
        self.meta
            .insert(PROFILES_META_KEY.to_string(), Value::from(profiles));
        self
    }

    /// The agent profiles permitted to call the tool, or `None` when its profiles entry is
    /// not a list of names. A tool that lists no profiles is permitted to none.
    pub fn profiles(&self) -> Option<Vec<&str>> {
        match self.meta.get(PROFILES_META_KEY) {
            None => Some(Vec::new()),
            Some(Value::Array(profiles)) => profiles.iter().map(Value::as_str).collect(),
            Some(_) => None,
        }
    }

    /// Whether the agent profile `profile` may call the tool.
    pub fn permits(&self, profile: &str) -> bool {
        self.profiles()
            .is_some_and(|profiles| profiles.contains(&profile))
    }

    /// The tool's annotations, each hint at its protocol default where the record omits it.
    pub fn effect(&self) -> ToolEffect {
        self.annotations.clone().unwrap_or_default().effect()
    }
}

/// Hints at what calling a tool does to its environment. A hint is a claim of the tool's
/// author, not a guarantee.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolAnnotations {
    /// The name people read, where the tool itself has no title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub title: Option<String>,
    /// The tool changes nothing. Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub read_only_hint: Option<bool>,
    /// A tool that changes something may destroy or overwrite. Defaults to true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub destructive_hint: Option<bool>,
    /// A tool that changes something changes nothing more when called again with the same
    /// arguments. Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub idempotent_hint: Option<bool>,
    /// The tool reaches entities outside its own domain. Defaults to true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub open_world_hint: Option<bool>,
}

impl ToolAnnotations {
    /// Annotations hinting that the tool changes nothing and acts only within its own domain.
    pub fn read_only_closed_world() -> Self {
        Self {
            read_only_hint: Some(true),
            open_world_hint: Some(false),
            ..Self::default()
        }
    }

    /// Each hint, at its protocol default where it is omitted.
    pub fn effect(&self) -> ToolEffect {
        let read_only = self.read_only_hint.unwrap_or(false);
        ToolEffect {
            read_only,
            destructive: !read_only && self.destructive_hint.unwrap_or(true),
            idempotent: read_only || self.idempotent_hint.unwrap_or(false),
            open_world: self.open_world_hint.unwrap_or(true),
        }
    }
}

/// What calling a tool does to its environment, as its annotations hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolEffect {
    /// It changes nothing.
    pub read_only: bool,
    /// It may destroy or overwrite.
    pub destructive: bool,
    /// Calling it again with the same arguments changes nothing more.
    pub idempotent: bool,
    /// It reaches entities outside its own domain.
    pub open_world: bool,
}

/// An icon a user interface can show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Icon {
    /// An HTTP(S) URL or a `data:` URI of the image.
    pub src: String,
    /// The image's MIME type, where its source omits it or gives a generic one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub mime_type: Option<String>,
    /// The sizes the icon suits, each `WxH` or `any`. None listed means any size.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sizes: Vec<String>,
    /// The background the icon is designed for. None means either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "IconTheme")]
    pub theme: Option<IconTheme>,
}

/// The background an icon is designed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum IconTheme {
    Light,
    Dark,
}
