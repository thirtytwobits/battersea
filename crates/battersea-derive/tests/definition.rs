use battersea_derive::NodeDefinition;
use flow_sdk::{
    catalog::{Catalog, NodeDefinition as _},
    registry::{ParameterType, Registry},
    FlowParameterEditorKind,
};
use serde_json::json;

#[derive(NodeDefinition)]
#[node_definition(crate = flow_sdk, manifest = r#"
node_definitions:
  - class_name: Echo
    short_description: Echo
    long_description: Emits a custom token
    handler_id: example.echo
    kind: source
    parameters:
      - name: label
        datatype: {kind: example.label}
        editor: {}
    output_ports:
      - {name: out, kind: output, token_type: example.message, mode: final_value, phase: snapshot}
"#)]
struct Echo<T: Send>
where
    T: Sync,
{
    _value: T,
}

fn registry() -> Registry {
    let mut registry = Registry::default();
    registry.register_handler("example.echo").unwrap();
    registry
        .register_token_type("example.message", json!({"type":"string"}))
        .unwrap();
    registry
        .register_parameter_type(
            "example.label",
            ParameterType::new(
                json!({"type":"string"}),
                Some(FlowParameterEditorKind::String),
                |_| Ok(()),
            ),
        )
        .unwrap();
    registry
}

#[test]
fn generic_derive_with_renamed_crate_matches_manifest_catalogue() {
    let definition = Echo::<String>::definition(&registry()).unwrap();
    let derived = Catalog::from_definitions(vec![definition], registry()).unwrap();
    let manifest = Catalog::from_manifest(Echo::<String>::DEFINITION_MANIFEST, registry()).unwrap();
    assert_eq!(derived.definitions(), manifest.definitions());
}

#[test]
fn host_registered_datatypes_are_required_by_both_paths() {
    assert!(Echo::<String>::definition(&Registry::default()).is_err());
    assert!(
        Catalog::from_manifest(Echo::<String>::DEFINITION_MANIFEST, Registry::default()).is_err()
    );
}

#[derive(NodeDefinition)]
#[node_definition(crate = flow_sdk, manifest = r#"
node_definitions:
  - {class_name: Invalid, short_description: Invalid, long_description: Invalid, kind: source, handler_id: example.echo, typo: true}
"#)]
struct Invalid;

#[test]
fn unknown_manifest_fields_fail_instead_of_being_ignored() {
    assert!(Invalid::definition(&registry()).is_err());
}
