use battersea_flow::{FlowDocument, FlowNodeDefinition, FlowValidationResult};
use schemars::generate::SchemaSettings;
use std::collections::{BTreeMap, BTreeSet};

fn required_fields(settings: SchemaSettings) -> BTreeMap<String, BTreeSet<String>> {
    let mut generator = settings.into_generator();
    generator.subschema_for::<FlowDocument>();
    generator.subschema_for::<FlowNodeDefinition>();
    generator.subschema_for::<FlowValidationResult>();
    generator
        .definitions()
        .iter()
        .map(|(name, schema)| {
            let required = schema
                .get("required")
                .and_then(|required| required.as_array())
                .into_iter()
                .flatten()
                .map(|field| field.as_str().unwrap().to_owned())
                .collect();
            (name.clone(), required)
        })
        .collect()
}

#[test]
fn a_flow_type_requires_to_be_read_only_fields_it_always_writes() {
    let written = required_fields(SchemaSettings::draft2020_12().for_serialize());
    let read = required_fields(SchemaSettings::draft2020_12().for_deserialize());
    for name in [
        "FlowDocument",
        "FlowPort",
        "FlowParameterEditor",
        "FlowValidationIssue",
    ] {
        assert!(read.contains_key(name), "{name} is not reached");
    }
    for (name, required) in &read {
        let omitted: Vec<_> = required.difference(&written[name]).collect();
        assert!(
            omitted.is_empty(),
            "{name} requires {omitted:?} to be read but may omit them when written"
        );
    }
}
