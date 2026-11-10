//! The runtime view of one run's edge marking, from contracts/observability.md.
use battersea_runtime::{EventKind, ExecutionEvent};
use battersea_telemetry::view::*;
use serde_json::{json, Value};

fn kind(phase: &str) -> EventKind {
    match phase {
        "flow.token.emit" => EventKind::TokenEmit,
        "flow.token.receive" => EventKind::TokenReceive,
        "flow.token.close" => EventKind::TokenClose,
        "flow.signal.emit" => EventKind::SignalEmit,
        "flow.signal.receive" => EventKind::SignalReceive,
        "flow.edge.marking" => EventKind::EdgeMarking,
        other => panic!("fixtures/edge-marking.json holds an unexpected phase {other}"),
    }
}

fn view() -> View {
    View::new(
        "test-process".into(),
        Retention {
            max_records: 64,
            max_record_bytes: 4096,
            max_delta_bytes: 4096,
            max_deltas: 64,
            max_age_ms: 1_000,
        },
    )
    .unwrap()
}

fn play(records: &[Value]) -> (View, Vec<Record>) {
    let mut view = view();
    let mut upserts = Vec::new();
    for (sequence, record) in records.iter().enumerate() {
        let event = ExecutionEvent {
            run_id: "run".into(),
            flow_key: "flow".into(),
            attempt: 1,
            sequence: sequence as u64,
            node_id: record["nodeId"].as_str().unwrap().into(),
            kind: kind(record["phase"].as_str().unwrap()),
            summary: record["summary"].as_str().unwrap().into(),
            detail: Some(record["detail"].clone()),
        };
        let delta = view
            .observe_execution(&event, None, sequence as u64, None, |_| Ok(()))
            .unwrap();
        for change in delta.changes {
            if let Change::Upsert { record } = change {
                upserts.push(*record);
            }
        }
    }
    (view, upserts)
}

/// The editor reads `fixtures/edge-marking-view.json`, so the fixture is what the view publishes
/// for the run in battersea-runtime's `fixtures/edge-marking.json`.
#[test]
fn the_view_holds_one_marking_per_activation_naming_only_occupied_or_closed_edges() {
    let run: Value = serde_json::from_str(include_str!(
        "../../battersea-runtime/fixtures/edge-marking.json"
    ))
    .unwrap();
    let (view, upserts) = play(run["records"].as_array().unwrap());
    let marking = view
        .snapshot()
        .records
        .into_iter()
        .find(|r| r.id == "marking:run");
    assert_eq!(
        marking.map(|r| r.state),
        Some(State::Marking {
            edges: [
                (
                    "a".to_string(),
                    EdgeMarking {
                        queued: 0,
                        close: Some(EdgeClose {
                            delivered: true,
                            failed: false
                        })
                    }
                ),
                (
                    "b".to_string(),
                    EdgeMarking {
                        queued: 0,
                        close: Some(EdgeClose {
                            delivered: true,
                            failed: false
                        })
                    }
                ),
            ]
            .into(),
        }),
        "the spent signal edge leaves the marking; the closed token edges remain"
    );
    let published = serde_json::to_value(&upserts).unwrap();
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/edge-marking-view.json")).unwrap();
    assert_eq!(
        fixture,
        published,
        "fixtures/edge-marking-view.json must hold the published records:\n{}",
        serde_json::to_string_pretty(&published).unwrap()
    );
}

#[test]
fn a_marking_record_replaces_only_the_edges_it_names_and_removes_emptied_ones() {
    let marking = |edges: Value| json!({"phase":"flow.edge.marking","nodeId":"n","summary":"s","detail":{"edges":edges}});
    let (view, _) = play(&[
        marking(json!({"a":{"queued":2},"b":{"queued":1}})),
        marking(json!({"a":{"queued":0}})),
        marking(json!({"b":{"queued":1,"close":{"delivered":false,"failed":true}}})),
    ]);
    let record = view
        .snapshot()
        .records
        .into_iter()
        .find(|r| r.id == "marking:run")
        .unwrap();
    assert_eq!(record.context.node_id, None);
    assert_eq!(
        serde_json::to_value(record.state).unwrap(),
        json!({"kind":"marking","edges":{"b":{"queued":1,"close":{"delivered":false,"failed":true}}}})
    );
}

#[test]
fn a_live_activations_marking_is_kept_while_terminal_history_is_evicted() {
    let mut view = View::new(
        "test-process".into(),
        Retention {
            max_records: 3,
            max_record_bytes: 4096,
            max_delta_bytes: 4096,
            max_deltas: 64,
            max_age_ms: 1_000,
        },
    )
    .unwrap();
    let event = |sequence: u64, kind: EventKind, detail: Value| ExecutionEvent {
        run_id: "run".into(),
        flow_key: "flow".into(),
        attempt: 1,
        sequence,
        node_id: "source".into(),
        kind,
        summary: String::new(),
        detail: Some(detail),
    };
    let mut observe = |sequence: u64, kind: EventKind, detail: Value| {
        view.observe_execution(&event(sequence, kind, detail), None, sequence, None, |_| {
            Ok(())
        })
        .unwrap();
    };
    observe(0, EventKind::ActivationAccepted, Value::Null);
    observe(
        1,
        EventKind::EdgeMarking,
        json!({"edges":{"a":{"queued":1}}}),
    );
    for (sequence, port) in ["p", "q", "r"].iter().enumerate() {
        observe(
            2 + sequence as u64,
            EventKind::TokenEmit,
            json!({"sourcePort":port,"tokenType":"text","targets":[]}),
        );
    }
    let ids: Vec<String> = view.snapshot().records.into_iter().map(|r| r.id).collect();
    assert!(ids.contains(&"marking:run".to_string()), "{ids:?}");
    assert!(ids.contains(&"activation:run".to_string()), "{ids:?}");
}
