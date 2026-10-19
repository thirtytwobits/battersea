//! The JSON Schema of the tool record, which downstream generators compose.
use schemars::{generate::SchemaSettings, JsonSchema};
use serde_json::{json, Value};

#[allow(dead_code)]
#[derive(JsonSchema)]
struct ToolsContract {
    tool: crate::Tool,
}

/// The schema whose `$defs` declare [`crate::Tool`] and the types it holds.
pub fn contract_schema() -> Value {
    let schema = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<ToolsContract>();
    let mut schema = serde_json::to_value(schema).expect("serialisable schema");
    schema["$id"] = json!("https://battersea.dev/schemas/tools.schema.json");
    schema
}
