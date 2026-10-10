use schemars::{generate::SchemaSettings, JsonSchema};
use serde_json::{json, Value};
#[allow(dead_code)]
#[derive(JsonSchema)]
struct RuntimeContract {
    config: crate::config::TelemetryConfig,
    snapshot: crate::view::Snapshot,
    update: crate::view::Update,
    retention: crate::view::Retention,
    catalogue: crate::accounting::PriceCatalogue,
    ledger: crate::accounting::Ledger,
}
pub fn contract_schema() -> Value {
    let schema = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<RuntimeContract>();
    let mut schema = serde_json::to_value(schema).expect("serialisable schema");
    portable_integers(&mut schema);
    schema["$id"] = json!("https://battersea.dev/schemas/runtime.schema.json");
    schema
}

/// Compose the same Rust-owned configuration definition into a host schema.
pub fn compose_config_schema(mut host: Value) -> crate::Result<Value> {
    let runtime = contract_schema();
    let definitions = host
        .get_mut("$defs")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| crate::Error::Invalid("Host schema requires $defs".into()))?;
    for (name, definition) in runtime["$defs"].as_object().expect("schema definitions") {
        if definitions.contains_key(name) {
            return Err(crate::Error::Invalid(format!(
                "Duplicate schema definition {name}"
            )));
        }
        definitions.insert(name.clone(), definition.clone());
    }
    Ok(host)
}

pub(crate) const MAX_JSON_INTEGER: u64 = 9_007_199_254_740_991;
/// Integer quantities cross Rust/JavaScript exactly. Revisions use decimal strings.
pub(crate) fn validate_integers(value: &Value) -> crate::Result<()> {
    match value {
        Value::Number(number) if number.as_u64().is_some_and(|n| n > MAX_JSON_INTEGER) => {
            return Err(crate::Error::Invalid(
                "Integer exceeds the exact JSON range".into(),
            ))
        }
        Value::Array(values) => {
            for value in values {
                validate_integers(value)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_integers(value)?;
            }
        }
        _ => (),
    }
    Ok(())
}
fn portable_integers(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            if fields.get("format").and_then(Value::as_str) == Some("uint64")
                || fields.get("format").and_then(Value::as_str) == Some("uint")
            {
                fields.remove("format");
                fields.insert("maximum".into(), json!(MAX_JSON_INTEGER));
            }
            for value in fields.values_mut() {
                portable_integers(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                portable_integers(value);
            }
        }
        _ => (),
    }
}
