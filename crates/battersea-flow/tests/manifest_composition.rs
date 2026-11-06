//! Distinguishing cases from contracts/kits.md: formatter blocks resolve across manifests.
use battersea_flow::{catalog::Catalog, registry::Registry, FlowPromptTemplateNode};
use serde_json::json;

fn registry() -> Registry {
    let mut registry = Registry::default();
    registry
        .register_token_type("example.text", json!({"type": "string"}))
        .unwrap();
    registry.register_handler("example.alpha").unwrap();
    registry
}

fn kit(greeting: &str) -> String {
    format!(
        r#"
formatter_blocks:
  greeting:
    template:
      kind: text
      text: "{greeting}"
node_definitions:
  - class_name: Alpha
    handler_id: example.alpha
    short_description: Alpha
    long_description: Alpha
    kind: inline
    input_ports:
      - name: input
        kind: input
        token_type: example.text
        mode: final_value
        phase: execution
    output_ports:
      - name: output
        kind: output
        token_type: example.text
        mode: final_value
        phase: execution
        formatter:
          kind: prompt_template
          default:
            ref: greeting
"#
    )
}

fn override_only(greeting: &str) -> String {
    format!(
        r#"
formatter_blocks:
  greeting:
    template:
      kind: text
      text: "{greeting}"
node_definitions: []
"#
    )
}

fn greeting_text(catalog: &Catalog) -> String {
    let definition = &catalog.definitions()["Alpha"];
    let formatter = definition.output_ports[0].formatter.as_ref().unwrap();
    match &formatter.default.as_ref().unwrap().template {
        FlowPromptTemplateNode::Text(text) => text.text.clone(),
        other => panic!("unexpected template {other:?}"),
    }
}

#[test]
fn a_later_manifest_replaces_an_earlier_formatter_block_of_the_same_name() {
    let kit = kit("from the kit");
    let application = override_only("from the application");
    let alone = Catalog::from_manifests(&[("kit", &kit)], registry()).unwrap();
    assert_eq!(greeting_text(&alone), "from the kit");
    let overridden =
        Catalog::from_manifests(&[("kit", &kit), ("application", &application)], registry())
            .unwrap();
    assert_eq!(greeting_text(&overridden), "from the application");
    let reversed =
        Catalog::from_manifests(&[("application", &application), ("kit", &kit)], registry())
            .unwrap();
    assert_eq!(greeting_text(&reversed), "from the kit");
}

#[test]
fn a_later_definition_of_the_same_class_is_refused() {
    let kit = kit("from the kit");
    let error = Catalog::from_manifests(&[("first", &kit), ("second", &kit)], registry())
        .err()
        .expect("duplicate class names must be refused");
    assert!(
        error.to_string().contains("duplicate class_name"),
        "{error}"
    );
}

#[test]
fn a_reference_to_a_block_no_manifest_defines_names_its_source() {
    let kit = kit("from the kit");
    let (_, definitions) = kit.split_once("node_definitions:").unwrap();
    let orphan = format!("node_definitions:{definitions}");
    let error = Catalog::from_manifests(&[("orphan", &orphan)], registry())
        .err()
        .expect("an unresolved reference must be refused");
    assert!(error.to_string().starts_with("orphan: "), "{error}");
    assert!(error.to_string().contains("greeting"), "{error}");
}
