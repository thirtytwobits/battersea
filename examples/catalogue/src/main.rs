use battersea_flow::{
    catalog::Catalog,
    document::{load_document, serialize_canonical_document},
    registry::{ParameterType, Registry},
    FlowParameterEditorKind,
};
use serde_json::json;

fn example() -> Result<String, Box<dyn std::error::Error>> {
    let mut registry = Registry::default();
    registry.register_handler("example.note")?;
    registry.register_token_type("example.note", json!({"type": "object", "required": ["text"], "properties": {"text": {"type": "string"}}, "additionalProperties": false}))?;
    registry.register_parameter_type(
        "example.label",
        ParameterType::new(
            json!({"type": "string", "minLength": 1}),
            Some(FlowParameterEditorKind::String),
            |_| Ok(()),
        ),
    )?;
    let manifest = json!({"node_definitions": [{
        "class_name": "Note", "short_description": "Note", "long_description": "An application note.",
        "handler_id": "example.note", "kind": "source",
        "parameters": [{"name": "label", "datatype": {"kind": "example.label"}, "editor": {}}],
        "output_ports": [{"name": "note", "kind": "output", "token_type": "example.note"}]
    }]}).to_string();
    let catalog = Catalog::from_manifest(&manifest, registry)?;
    catalog
        .registry()
        .validate_token("example.note", &json!({"text": "Authored content"}))?;
    let flow = load_document(&json!({
        "version": 1, "flow_key": "notes", "title": "Notes",
        "nodes": [{"id": "note", "definition_name": "Note", "instance_name": "First note", "parameter_values": {"label": "Example"}}],
        "layout": {"example-editor": {"position": [12, 34], "zoom": 1.5}},
        "metadata": {"example": {"flags": [true, null]}}
    }).to_string())?;
    let validation = catalog.validate(&flow);
    if !validation.valid {
        return Err(format!("Invalid example: {:?}", validation.issues).into());
    }
    let canonical = serialize_canonical_document(&flow)?;
    let restored = load_document(&canonical)?;
    if restored != flow {
        return Err("The flow must round-trip with its opaque editor metadata".into());
    }
    Ok(canonical)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", example()?);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_independent_application_registers_and_round_trips_its_contract() {
        super::example().unwrap();
    }
}
