//! Copyright (c) Scott A Dixon
//!
//! The kit's records and token types. Each record carries the application's extension at
//! `extension`; timestamps are milliseconds since the Unix epoch on the port's clock.
use battersea_model::{
    ContentBlock, EngineLocalToolDefinition, EngineLocalToolExecutor, EngineTokenUsage, Role,
    ToolCallEvidence,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);
        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self(id.into())
            }
        }
        impl From<String> for $name {
            fn from(id: String) -> Self {
                Self(id)
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}
id!(
    /// A transcript, named by the application.
    TranscriptId
);
id!(
    /// A turn, named by the port when it staged the turn.
    TurnId
);
id!(
    /// A compaction job, named by the port when it requested the job.
    JobId
);

/// One turn of a conversation: `conversation.turn`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Turn<X> {
    /// Assigned by the port when the turn is staged; absent before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<TurnId>,
    pub role: Role,
    pub content: Vec<ContentBlock>,
    /// Assigned by the port when the turn is staged; absent before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    /// The revisions applied to the committed turn, from zero.
    #[serde(default)]
    pub revision: u32,
    pub extension: X,
}
impl<X> Turn<X> {
    /// An unstaged turn of one text block.
    pub fn text(role: Role, text: impl Into<String>, extension: X) -> Self {
        Self {
            id: None,
            role,
            content: vec![ContentBlock::Text { text: text.into() }],
            created_at: None,
            revision: 0,
            extension,
        }
    }
    /// The turn's text blocks, concatenated.
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
    /// Whether the turn calls a tool.
    pub fn calls_tools(&self) -> bool {
        self.content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
    }
}

/// A transcript's memory of the turns it covers: `conversation.summary`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Summary<X> {
    pub text: String,
    /// The last turn the summary covers.
    pub through: TurnId,
    /// The compaction job that produced the summary, if one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobId>,
    pub extension: X,
}

/// New content for a committed turn: `conversation.revision`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Revision<X> {
    pub turn: TurnId,
    pub content: Vec<ContentBlock>,
    pub extension: X,
}

/// How a chat call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallOutcome {
    Completed,
    Failed,
    Cancelled,
    OverBudget,
}

/// A chat call's result: `conversation.call_status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallStatus {
    pub outcome: CallOutcome,
    /// The provider's stop reason, as the model crate spells it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[schemars(with = "Value")]
    #[serde(default)]
    pub usage: EngineTokenUsage,
    /// The tool rounds the call ran.
    #[serde(default)]
    pub rounds: u32,
}

/// One tool call of a round.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    pub ok: bool,
    #[serde(default)]
    pub result: Value,
    #[schemars(with = "Value")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<ToolCallEvidence>,
}

/// One round of tool calls: an element of `conversation.tool_rounds`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolRound {
    pub round: u32,
    pub calls: Vec<ToolCall>,
}

/// Where a compaction job stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Requested,
    Running,
    Completed,
    Failed,
}
impl JobState {
    /// Whether the job has ended.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

/// A compaction job the port holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompactionJob<X> {
    pub id: JobId,
    pub transcript: TranscriptId,
    pub evaluation: crate::compaction::PolicyEvaluation,
    pub state: JobState,
    pub extension: X,
}

/// What a chat call expects to spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SpendEstimate {
    pub input_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
}

/// Spend the port set aside for one call, settled with the call's usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Reservation {
    pub id: String,
    pub node: String,
    pub estimate: SpendEstimate,
}

/// One chat call the port records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallRecord {
    pub node: String,
    pub status: CallStatus,
    /// The reservation the call settled, if it reserved spend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation: Option<Reservation>,
    pub started_at: u64,
    pub finished_at: u64,
}

/// The tools a run offers the model. A plan without definitions runs the call without
/// tools.
#[derive(Clone, Default)]
pub struct ToolPlan {
    pub definitions: Vec<EngineLocalToolDefinition>,
    pub executor: Option<Arc<dyn EngineLocalToolExecutor>>,
    pub max_rounds: usize,
}
impl std::fmt::Debug for ToolPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolPlan")
            .field("definitions", &self.definitions)
            .field("executor", &self.executor.is_some())
            .field("max_rounds", &self.max_rounds)
            .finish()
    }
}

/// A transcript's memory: its latest summary, if any, and the turns after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Memory<X> {
    pub transcript: TranscriptId,
    pub summary: Option<Summary<X>>,
    pub turns: Vec<Turn<X>>,
}

/// The kit's state in one run. The application includes it in its versioned host state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunState<X> {
    pub transcript: TranscriptId,
    /// The turns the run has staged, in staging order.
    pub staged: Vec<Turn<X>>,
    pub calls: Vec<CallRecord>,
    /// Reservations not yet settled.
    pub reservations: Vec<Reservation>,
    pub deliveries: Vec<String>,
}
impl<X> RunState<X> {
    /// The version of the state's serialised form.
    pub const STATE_VERSION: u32 = 1;
    /// A run on `transcript` that has done nothing.
    pub fn new(transcript: TranscriptId) -> Self {
        Self {
            transcript,
            staged: vec![],
            calls: vec![],
            reservations: vec![],
            deliveries: vec![],
        }
    }
}
impl<X: Serialize> RunState<X> {
    /// The state with its version, for the application's host state.
    pub fn to_versioned(&self) -> Result<Value, String> {
        Ok(json!({
            "version": Self::STATE_VERSION,
            "state": serde_json::to_value(self).map_err(|error| error.to_string())?,
        }))
    }
}
impl<X: serde::de::DeserializeOwned> RunState<X> {
    /// The state `to_versioned` wrote. Another version is refused.
    pub fn from_versioned(value: &Value) -> Result<Self, String> {
        let version = value["version"].as_u64();
        if version != Some(u64::from(Self::STATE_VERSION)) {
            return Err(format!(
                "conversation run state version {version:?} is not {}",
                Self::STATE_VERSION
            ));
        }
        serde_json::from_value(value["state"].clone()).map_err(|error| error.to_string())
    }
}

/// The kit's token types and their schemas, with `extension` embedded at each record's
/// `extension` key.
pub fn token_schemas(extension: &Value) -> Vec<(&'static str, Value)> {
    fn schema<T: JsonSchema>(extension: &Value) -> Value {
        let mut schema = serde_json::to_value(schemars::schema_for!(T)).expect("a schema");
        embed(&mut schema, extension);
        schema
    }
    vec![
        ("conversation.turn", schema::<Turn<Value>>(extension)),
        ("conversation.turns", schema::<Vec<Turn<Value>>>(extension)),
        ("conversation.summary", schema::<Summary<Value>>(extension)),
        (
            "conversation.revision",
            schema::<Revision<Value>>(extension),
        ),
        ("conversation.call_status", schema::<CallStatus>(extension)),
        (
            "conversation.tool_rounds",
            schema::<Vec<ToolRound>>(extension),
        ),
    ]
}

/// Replaces every `extension` property's schema with `extension`. A local reference in the
/// extension schema is rebased onto the place it is embedded, so a self-contained schema
/// stays valid; a schema with an `$id` of its own keeps its references.
fn embed(node: &mut Value, extension: &Value) {
    embed_at(node, extension, &mut String::new());
}
fn embed_at(node: &mut Value, extension: &Value, pointer: &mut String) {
    match node {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                let length = pointer.len();
                pointer.push('/');
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
                embed_at(value, extension, pointer);
                pointer.truncate(length);
            }
            if let Some(Value::Object(properties)) = map.get_mut("properties") {
                if let Some(slot) = properties.get_mut("extension") {
                    let mut embedded = extension.clone();
                    if embedded.get("$id").is_none() {
                        rebase(&mut embedded, &format!("{pointer}/properties/extension"));
                    }
                    *slot = embedded;
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                let length = pointer.len();
                pointer.push('/');
                pointer.push_str(&index.to_string());
                embed_at(item, extension, pointer);
                pointer.truncate(length);
            }
        }
        _ => {}
    }
}
/// Prefixes every local pointer `$ref` under `node` with `pointer`, outside subschemas with
/// an `$id`. A reference to a named anchor stays as it is: without an `$id`, the anchor
/// belongs to the token schema the extension is embedded in.
fn rebase(node: &mut Value, pointer: &str) {
    match node {
        Value::Object(map) => {
            if let Some(Value::String(target)) = map.get_mut("$ref") {
                if let Some(local) = target.strip_prefix('#') {
                    if local.is_empty() || local.starts_with('/') {
                        *target = format!("#{pointer}{local}");
                    }
                }
            }
            for (key, value) in map.iter_mut() {
                if key != "$ref" && value.get("$id").is_none() {
                    rebase(value, pointer);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                rebase(item, pointer);
            }
        }
        _ => {}
    }
}
