use battersea_flow::document::{
    canonicalize_document, inspect_document, load_document, serialize_canonical_document,
    DocumentInspection, Upgrades,
};
use serde_json::json;

#[test]
fn unknown_versions_are_inspected_without_interpreting_or_upgrading_their_payload() {
    let future = json!({"version": 72, "flow_key": "future", "title": "Future graph", "nodes": {"new_format": true}}).to_string();
    let before = future.clone();
    let DocumentInspection::Incompatible { raw, .. } = inspect_document(&future).unwrap() else {
        panic!("unknown version must be retained");
    };
    assert_eq!(
        raw,
        serde_json::from_str::<serde_json::Value>(&future).unwrap()
    );
    assert!(load_document(&future).is_err());
    assert!(Upgrades::default().to_supported_version(&future).is_err());
    assert_eq!(future, before);
}

#[test]
fn canonical_round_trip_preserves_authored_policy_and_opaque_editor_values() {
    let source = json!({"version": 2, "execution": battersea_flow::FlowExecutionPolicy { source_order: vec![], limits: battersea_flow::FlowExecutionLimits::default() }, "flow_key": "test", "title": "Example", "whitespace_mode": "preserve", "output_encoding": "plain", "plain_fragment_delimiter": "space", "layout": {"editor_a": {"view": [1, 2, 3]}, "editor_b": {"future": {"x": true}}}, "metadata": {"extension": ["first", "second"]}}).to_string();
    let flow = load_document(&source).unwrap();
    let round_trip = load_document(&serialize_canonical_document(&flow).unwrap()).unwrap();
    assert_eq!(round_trip, canonicalize_document(&flow));
    assert_eq!(canonicalize_document(&round_trip), round_trip);
    assert_eq!(
        serialize_canonical_document(&round_trip).unwrap(),
        serialize_canonical_document(&flow).unwrap()
    );
}

#[test]
fn only_explicit_upgrade_invocation_runs_a_registered_transformation() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let called = Arc::new(AtomicBool::new(false));
    let flag = called.clone();
    let mut upgrades = Upgrades::default();
    upgrades
        .register(0, 2, move |mut value| {
            flag.store(true, Ordering::SeqCst);
            value["version"] = json!(2);
            value["execution"] = json!(battersea_flow::FlowExecutionPolicy {
                source_order: vec![],
                limits: battersea_flow::FlowExecutionLimits::default()
            });
            Ok(value)
        })
        .unwrap();
    let source = json!({"version": 0, "flow_key": "old", "title": "Old graph"}).to_string();
    assert!(matches!(
        inspect_document(&source).unwrap(),
        DocumentInspection::Incompatible { .. }
    ));
    assert!(!called.load(Ordering::SeqCst));
    let upgraded = upgrades.to_supported_version(&source).unwrap();
    assert!(called.load(Ordering::SeqCst));
    assert!(load_document(&upgraded).is_ok());
}
