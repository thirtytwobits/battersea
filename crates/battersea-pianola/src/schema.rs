//! Strict roll-schema composition for host-owned setup, actions and assertions.
use serde_json::{json, Value};
use std::collections::HashSet;
pub fn compose_schema(extension: Value) -> Result<Value, String> {
    let mut schema: Value =
        serde_json::from_str(include_str!("../schemas/pianola-roll.schema.json"))
            .map_err(|e| e.to_string())?;
    let setup = extension
        .get("setup")
        .filter(|v| v.is_object())
        .ok_or("setup schema is required")?;
    let actions = extension
        .get("actions")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
        .ok_or("at least one action schema is required")?;
    schema["properties"]["setup"] = setup.clone();
    let step = &mut schema["properties"]["steps"]["items"]["properties"];
    step["do"]["oneOf"] = Value::Array(actions.clone());
    let expectations = step["expect"]["items"]["oneOf"].as_array_mut().unwrap();
    let mut kinds: HashSet<String> = expectations
        .iter()
        .filter_map(|v| {
            v["properties"]["kind"]["const"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    for entry in extension
        .get("expectations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = entry["properties"]["kind"]["const"]
            .as_str()
            .ok_or("assertion kind must be a const string")?;
        if !kinds.insert(kind.into()) {
            return Err(format!("duplicate assertion kind: {kind}"));
        }
        expectations.push(entry.clone());
    }
    for entry in expectations {
        match entry["properties"]["kind"]["const"].as_str() {
            Some("count") => {
                if let Some(targets) = extension.get("count_targets").and_then(Value::as_array) {
                    entry["properties"]["target"]["enum"]
                        .as_array_mut()
                        .unwrap()
                        .extend(targets.clone());
                }
            }
            Some("agent_grade") => {
                if let Some(port) = extension.get("default_grade_port") {
                    entry["properties"]["subject"]["default"] = json!({"port":port});
                }
                if let Some(graders) = extension.get("graders") {
                    entry["properties"]["grader"]["properties"]["kind"] = json!({"enum":graders});
                }
            }
            _ => (),
        }
    }
    Ok(schema)
}
/// Syntax/schema errors are data; callers choose their presentation and exit code.
pub fn validate_yaml(yaml: &str, schema: &Value) -> Vec<String> {
    let instance: Value = match serde_yaml::from_str(yaml) {
        Ok(v) => v,
        Err(e) => return vec![format!("yaml: {e}")],
    };
    match jsonschema::validator_for(schema) {
        Ok(validator) => validator
            .iter_errors(&instance)
            .map(|e| format!("schema: {e}"))
            .collect(),
        Err(e) => vec![format!("schema compile error: {e}")],
    }
}
