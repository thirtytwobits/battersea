use battersea_telemetry::{accounting::*, content::Content, view::*, Error};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
fn money(micros: u64) -> Money {
    Money {
        currency: "USD".into(),
        micros,
    }
}
fn view() -> View {
    View::new(
        "test-process".into(),
        Retention {
            max_records: 2,
            max_record_bytes: 4096,
            max_delta_bytes: 4096,
            max_deltas: 2,
            max_age_ms: 100,
        },
    )
    .unwrap()
}
fn record(id: &str, time: u64) -> Record {
    Record {
        id: id.into(),
        context: Context {
            activation_id: "run".into(),
            flow_key: "flow".into(),
            node_id: None,
            session_id: None,
        },
        started_at_ms: time,
        updated_at_ms: time,
        state: State::Activation {
            status: Status::Succeeded,
        },
    }
}
#[test]
fn snapshot_plus_contiguous_deltas_matches_authority_and_rejects_gaps_atomically() {
    let mut view = view();
    let mut client = view.snapshot();
    let first = view.record(record("first", 1), |_| Ok(())).unwrap();
    let second = view.record(record("second", 2), |_| Ok(())).unwrap();
    let before = client.clone();
    assert_eq!(client.apply(&second), Err(Error::ResyncRequired));
    assert_eq!(client, before);
    client.apply(&first).unwrap();
    client.apply(&second).unwrap();
    assert_eq!(client, view.snapshot());
    assert_eq!(client.apply(&second), Err(Error::ResyncRequired));
}
#[test]
fn retention_evicts_by_count_bytes_age_and_resyncs_expired_or_foreign_cursors() {
    let mut view = view();
    let initial = view.snapshot().cursor;
    for (i, id) in ["a", "b", "c"].into_iter().enumerate() {
        view.record(record(id, i as u64), |_| Ok(())).unwrap();
    }
    assert!(view.snapshot().records.len() <= 2);
    assert!(matches!(view.updates(&initial), Update::Resync { .. }));
    let mut cursor = view.snapshot().cursor;
    cursor.epoch = "another-process".into();
    assert!(matches!(view.updates(&cursor), Update::Resync { .. }));
    let before = view.snapshot();
    let _ = view.updates(&before.cursor);
    assert_eq!(before, view.snapshot());
    view.maintain(200, |_| Ok(())).unwrap();
    assert!(view.snapshot().records.is_empty());
    let mut too_large = record("large", 201);
    too_large.context.flow_key = "x".repeat(5000);
    assert_eq!(view.record(too_large, |_| Ok(())), Err(Error::Capacity));
}
#[test]
fn lossless_capture_failure_is_visible_and_does_not_advance_the_cursor() {
    let mut view = view();
    let before = view.snapshot();
    assert!(matches!(
        view.record(record("a", 1), |_| Err(Error::Delivery("disk full".into()))),
        Err(Error::Delivery(_))
    ));
    assert_eq!(view.snapshot(), before);
}
#[test]
fn content_is_opt_in_and_masked_before_any_consumer_receives_it() {
    let raw = b"private credential";
    let metadata = Content::observe(raw, None);
    assert!(metadata.masked.is_none());
    assert_eq!(metadata.byte_length, raw.len() as u64);
    let masked = Content::observe(raw, Some(&|_| "[removed]".into()));
    assert_eq!(metadata.sha256, masked.sha256);
    assert!(!serde_json::to_string(&masked)
        .unwrap()
        .contains(std::str::from_utf8(raw).unwrap()));
}
fn catalogue() -> PriceCatalogue {
    PriceCatalogue {
        version: "fixture-v1".into(),
        source: "contract fixture".into(),
        effective_date: "2026-10-09".into(),
        prices: BTreeMap::from([(
            "vendor/model".into(),
            Price {
                currency: "USD".into(),
                input_micros_per_million: 1_000_000,
                output_micros_per_million: 2_000_000,
                cached_input_micros_per_million: Some(100_000),
                cache_write_input_micros_per_million: None,
                request_micros: 0,
            },
        )]),
    }
}
#[test]
fn provider_cost_precedes_prices_and_missing_billable_usage_stays_unknown() {
    let catalogue = catalogue();
    catalogue.validate().unwrap();
    let reported = money(17);
    assert_eq!(
        catalogue
            .cost(
                "missing",
                "model",
                &Usage::default(),
                Some(reported.clone())
            )
            .unwrap()
            .amount(),
        Some(&reported)
    );
    let partial = Usage {
        input_tokens: Some(100),
        output_tokens: Some(10),
        ..Default::default()
    };
    assert!(catalogue
        .cost("vendor", "model", &partial, None)
        .unwrap()
        .amount()
        .is_none());
    let complete = Usage {
        cached_input_tokens: Some(20),
        ..partial
    };
    let cost = catalogue.cost("vendor", "model", &complete, None).unwrap();
    let expected = money((100 - 20) + 20 / 10 + 10 * 2);
    assert_eq!(cost.amount(), Some(&expected));
    let zero = Usage {
        input_tokens: Some(0),
        output_tokens: Some(0),
        cached_input_tokens: Some(0),
        cache_write_input_tokens: Some(0),
    };
    assert_eq!(
        catalogue
            .cost("vendor", "model", &zero, None)
            .unwrap()
            .amount(),
        Some(&money(0))
    );
}
#[test]
fn simultaneous_admission_cannot_spend_the_same_remaining_budget() {
    let ledger = Arc::new(Mutex::new(
        Ledger::new(BTreeMap::from([("shared".into(), money(100))]), 20).unwrap(),
    ));
    let workers: Vec<_> = (0..20)
        .map(|i| {
            let ledger = ledger.clone();
            std::thread::spawn(move || {
                ledger
                    .lock()
                    .unwrap()
                    .reserve(&i.to_string(), vec!["shared".into()], money(60), |_| Ok(()))
                    .is_ok()
            })
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .filter_map(|h| h.join().ok())
            .filter(|v| *v)
            .count(),
        1
    );
    let l = ledger.lock().unwrap();
    let b = &l.budgets["shared"];
    assert!(b.spent_micros + b.reserved_micros <= b.limit.micros);
}
#[test]
fn reservations_survive_unknown_cost_restart_and_failed_persistence() {
    let mut ledger = Ledger::new(BTreeMap::from([("shared".into(), money(100))]), 10).unwrap();
    let before = ledger.clone();
    assert!(ledger
        .reserve("failed", vec!["shared".into()], money(60), |_| Err(
            Error::Delivery("disk full".into())
        ))
        .is_err());
    assert_eq!(ledger, before);
    ledger
        .reserve("paid", vec!["shared".into()], money(60), |_| Ok(()))
        .unwrap();
    ledger.settle("paid", Cost::default(), |_| Ok(())).unwrap();
    let bytes = serde_json::to_vec(&ledger).unwrap();
    let mut restored = Ledger::restore(&bytes).unwrap();
    assert!(restored
        .reserve("second", vec!["shared".into()], money(60), |_| Ok(()))
        .is_err());
    let actual = Cost::ProviderReported { amount: money(40) };
    restored.settle("paid", actual.clone(), |_| Ok(())).unwrap();
    let settled = restored.clone();
    restored.settle("paid", actual, |_| Ok(())).unwrap();
    assert_eq!(restored, settled);
    assert!(restored
        .reserve("paid", vec!["shared".into()], money(1), |_| Ok(()))
        .is_err());
    restored
        .reserve("second", vec!["shared".into()], money(60), |_| Ok(()))
        .unwrap();
}
#[test]
fn multi_budget_admission_is_atomic_and_corrupt_journals_are_rejected() {
    let mut ledger = Ledger::new(
        BTreeMap::from([("large".into(), money(100)), ("small".into(), money(10))]),
        10,
    )
    .unwrap();
    let before = ledger.clone();
    assert!(ledger
        .reserve(
            "a",
            vec!["large".into(), "small".into()],
            money(20),
            |_| Ok(())
        )
        .is_err());
    assert_eq!(ledger, before);
    ledger
        .reserve("a", vec!["large".into()], money(20), |_| Ok(()))
        .unwrap();
    ledger.budgets.get_mut("large").unwrap().reserved_micros = 0;
    assert!(Ledger::restore(&serde_json::to_vec(&ledger).unwrap()).is_err());
}

#[tokio::test]
async fn otlp_collector_receives_masked_typed_spans_and_metrics_and_rejection_is_visible() {
    use axum::{routing::post, Json, Router};
    use battersea_telemetry::otlp::{Batch, Exporter};
    use serde_json::{json, Value};
    let received = Arc::new(Mutex::new(Vec::<Value>::new()));
    let trace_received = received.clone();
    let metric_received = received.clone();
    let router = Router::new()
        .route(
            "/v1/traces",
            post(move |Json(body): Json<Value>| {
                let received = trace_received.clone();
                async move {
                    received.lock().unwrap().push(body);
                    Json(json!({}))
                }
            }),
        )
        .route(
            "/v1/metrics",
            post(move |Json(body): Json<Value>| {
                let received = metric_received.clone();
                async move {
                    received.lock().unwrap().push(body);
                    Json(json!({"partialSuccess":{"rejectedDataPoints":"1"}}))
                }
            }),
        );
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(socket, router).await.unwrap() });
    let mut completed = record("request:test", 10);
    completed.updated_at_ms = 25;
    completed.state = State::Request {
        request_id: "test".into(),
        provider: "vendor".into(),
        model: "model".into(),
        operation: "chat".into(),
        status: Status::Succeeded,
        retry: RetryState::None,
        input: Some(Box::new(Content::observe(
            b"private credential",
            Some(&|_| "masked".into()),
        ))),
        output: None,
        usage: Box::default(),
        cost: Box::default(),
        elapsed_ms: 15,
        first_chunk_ms: Some(5),
        chunk_timing: TimingSummary {
            count: 2,
            total_ms: 5,
            minimum_ms: Some(2),
            maximum_ms: Some(3),
        },
    };
    let batch = Batch::from_records("test", &[completed], 0, 25).unwrap();
    let exporter = Exporter::new(
        &format!("http://{address}"),
        std::time::Duration::from_secs(2),
        65536,
    )
    .unwrap();
    assert!(matches!(
        exporter.export(&batch).await,
        Err(Error::Delivery(_))
    ));
    let bodies = received.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    assert!(!serde_json::to_string(&*bodies)
        .unwrap()
        .contains("private credential"));
    let span = &bodies[0]["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert_eq!(span["traceId"].as_str().unwrap().len(), 32);
    assert_eq!(span["spanId"].as_str().unwrap().len(), 16);
    assert!(
        span["endTimeUnixNano"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > span["startTimeUnixNano"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
    );
    let metric_names: Vec<_> = bodies[1]["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert!(metric_names
        .iter()
        .any(|n| n.ends_with("time_to_first_chunk")));
    assert!(metric_names.iter().any(|n| n.ends_with("in_flight")));
    server.abort();
}

#[test]
fn imported_prices_preserve_provenance_and_omit_unrepresentable_tiers() {
    let input = br#"{
      "vendor/standard": {"mode":"chat", "litellm_provider":"vendor", "input_cost_per_token":0.0000015, "output_cost_per_token":2.5e-6},
      "tiered": {"mode":"chat", "litellm_provider":"vendor", "input_cost_per_token":0.000001, "output_cost_per_token":0.000002, "input_cost_per_token_above_128k_tokens":0.000004}
    }"#;
    let catalogue = PriceCatalogue::from_litellm_json(input, "2026-10-09".into()).unwrap();
    let usage = Usage {
        input_tokens: Some(2),
        output_tokens: Some(2),
        ..Default::default()
    };
    let cost = catalogue.cost("vendor", "standard", &usage, None).unwrap();
    assert_eq!(cost.amount(), Some(&money(3 + 5)));
    assert!(catalogue
        .cost("vendor", "tiered", &usage, None)
        .unwrap()
        .amount()
        .is_none());
    assert!(
        matches!(cost,Cost::Calculated { catalogue_version, .. } if catalogue_version == catalogue.version)
    );
}
#[test]
fn scheduler_observations_update_typed_live_state_without_tracing() {
    use battersea_runtime::{EventKind, ExecutionEvent};
    let mut view = view();
    let mut event = ExecutionEvent {
        attempt: 0,
        run_id: "run".into(),
        flow_key: "flow".into(),
        sequence: 0,
        node_id: "node".into(),
        kind: EventKind::NodeStart,
        summary: "Starting".into(),
        detail: None,
    };
    view.observe_execution(&event, None, 1, None, |_| Ok(()))
        .unwrap();
    event.kind = EventKind::NodeComplete;
    event.sequence += 1;
    view.observe_execution(&event, None, 2, None, |_| Ok(()))
        .unwrap();
    assert!(matches!(
        view.snapshot().records.as_slice(),
        [Record {
            started_at_ms: 1,
            updated_at_ms: 2,
            state: State::Node {
                status: Status::Succeeded,
                ..
            },
            ..
        }]
    ));
}

#[test]
fn usage_snapshots_are_idempotent_within_a_turn_and_add_across_tool_turns() {
    use battersea_model::adapter::EngineTokenUsage;
    use battersea_telemetry::usage::UsageAccumulator;
    let mut accumulator = UsageAccumulator::new(3).unwrap();
    let first = EngineTokenUsage {
        input_tokens: Some(17),
        output_tokens: Some(4),
        ..Default::default()
    };
    accumulator.observe(first).unwrap();
    accumulator.observe(first).unwrap();
    assert_eq!(
        accumulator.totals().unwrap().input_tokens,
        first.input_tokens
    );
    accumulator
        .observe(EngineTokenUsage {
            turn_index: 1,
            ..Default::default()
        })
        .unwrap();
    assert!(accumulator.totals().unwrap().input_tokens.is_none());
    let second = EngineTokenUsage {
        turn_index: 1,
        input_tokens: Some(23),
        output_tokens: Some(7),
        ..Default::default()
    };
    accumulator.observe(second).unwrap();
    assert_eq!(
        accumulator.totals().unwrap().input_tokens,
        first
            .input_tokens
            .zip(second.input_tokens)
            .map(|(a, b)| a + b)
    );
    let before = accumulator.totals().unwrap();
    assert!(accumulator.observe(first).is_err());
    assert_eq!(accumulator.totals().unwrap(), before);
}

#[test]
fn tool_turn_costs_include_each_request_fee_and_each_rounding_boundary() {
    use battersea_model::adapter::EngineTokenUsage;
    use battersea_telemetry::usage::UsageAccumulator;
    let mut catalogue = catalogue();
    let price = catalogue.prices.get_mut("vendor/model").unwrap();
    price.request_micros = 7;
    price.input_micros_per_million = 1;
    price.output_micros_per_million = 1;
    price.cached_input_micros_per_million = None;
    let mut observed = UsageAccumulator::new(3).unwrap();
    let mut expected = 0;
    for turn in 0..3 {
        let usage = EngineTokenUsage {
            turn_index: turn,
            input_tokens: Some(1),
            output_tokens: Some(1),
            ..Default::default()
        };
        observed.observe(usage).unwrap();
        observed.observe(usage).unwrap();
        expected += catalogue
            .cost(
                "vendor",
                "model",
                &Usage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    ..Default::default()
                },
                None,
            )
            .unwrap()
            .amount()
            .unwrap()
            .micros;
    }
    assert_eq!(
        observed
            .cost(&catalogue, "vendor", "model")
            .unwrap()
            .amount()
            .unwrap()
            .micros,
        expected
    );
    assert!(observed
        .cost(&catalogue, "missing", "model")
        .unwrap()
        .amount()
        .is_none());
}
#[test]
fn explicit_reconciliation_preserves_evidence_and_is_idempotent_after_restart() {
    let mut ledger = Ledger::new(BTreeMap::from([("shared".into(), money(100))]), 3).unwrap();
    ledger
        .reserve("charged", vec!["shared".into()], money(80), |_| Ok(()))
        .unwrap();
    ledger
        .settle("charged", Cost::default(), |_| Ok(()))
        .unwrap();
    let before = ledger.clone();
    assert!(ledger
        .reconcile("charged", None, " ".into(), |_| Ok(()))
        .is_err());
    assert_eq!(before, ledger);
    ledger
        .reconcile(
            "charged",
            Some(money(30)),
            "Provider invoice confirms final charge".into(),
            |_| Ok(()),
        )
        .unwrap();
    let mut restored = Ledger::restore(&serde_json::to_vec(&ledger).unwrap()).unwrap();
    restored
        .reconcile(
            "charged",
            Some(money(30)),
            "Provider invoice confirms final charge".into(),
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(ledger, restored);
    assert!(restored
        .reconcile("charged", None, "Provider voided charge".into(), |_| Ok(()))
        .is_err());
    ledger
        .reserve("void", vec!["shared".into()], money(60), |_| Ok(()))
        .unwrap();
    ledger
        .reconcile(
            "void",
            None,
            "Provider confirms request was never admitted".into(),
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(ledger.budgets["shared"].reserved_micros, 0);
    assert_eq!(ledger.budgets["shared"].spent_micros, money(30).micros);
}

#[test]
fn lossless_writer_capacity_and_partial_io_failure_never_report_success() {
    use battersea_telemetry::capture::Capture;
    use std::io::{self, Write};
    let delta = view().record(record("event", 1), |_| Ok(())).unwrap();
    let mut bounded = Capture::new(Vec::new(), 65536, 1).unwrap();
    bounded.append(&delta).unwrap();
    assert!(matches!(bounded.append(&delta), Err(Error::Capacity)));
    let bytes = bounded.into_inner();
    let recovered: Delta = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(recovered, delta);
    struct PartialWriter {
        bytes: Vec<u8>,
        fail: bool,
    }
    impl Write for PartialWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail {
                return Err(io::Error::other("fixture failure"));
            }
            self.fail = true;
            let count = bytes.len().min(3);
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut capture = Capture::new(
        PartialWriter {
            bytes: vec![],
            fail: false,
        },
        65536,
        3,
    )
    .unwrap();
    assert!(matches!(capture.append(&delta), Err(Error::Delivery(_))));
    assert!(matches!(capture.append(&delta), Err(Error::Delivery(_))));
    let partial = capture.into_inner().bytes;
    assert!(serde_json::from_slice::<Delta>(&partial).is_err());
    assert!(!partial.ends_with(b"\n"));
}

#[test]
fn closing_a_port_preserves_its_type_and_reports_direction_without_payload() {
    use battersea_runtime::{EventKind, ExecutionEvent};
    let mut view = view();
    let mut event = ExecutionEvent {
        attempt: 0,
        run_id: "run".into(),
        flow_key: "flow".into(),
        node_id: "node".into(),
        sequence: 0,
        kind: EventKind::TokenEmit,
        summary: String::new(),
        detail: Some(
            serde_json::json!({"sourcePort":"output","tokenType":"text","value":"private"}),
        ),
    };
    view.observe_execution(&event, None, 1, None, |_| Ok(()))
        .unwrap();
    event.kind = EventKind::TokenClose;
    event.sequence += 1;
    event.detail = Some(serde_json::json!({"sourcePort":"output","direction":"output"}));
    view.observe_execution(&event, None, 2, None, |_| Ok(()))
        .unwrap();
    assert!(view.records().any(|r|matches!(&r.state,State::Port {port,token_type,direction:Direction::Output,action:PortAction::Close,content:None} if port=="output" && token_type=="text")));
    assert!(!serde_json::to_string(&view.snapshot())
        .unwrap()
        .contains("private"));
}

#[test]
fn signal_observations_preserve_the_output_and_input_port_identities() {
    use battersea_runtime::{EventKind, ExecutionEvent};
    let mut view = view();
    let output = "finished";
    let input = "commit";
    for (sequence, kind, detail, expected, direction) in [
        (
            0,
            EventKind::SignalEmit,
            serde_json::json!({"signalPort":output}),
            output,
            Direction::Output,
        ),
        (
            1,
            EventKind::SignalReceive,
            serde_json::json!({"sourcePort":output,"targetPort":input}),
            input,
            Direction::Input,
        ),
    ] {
        let event = ExecutionEvent {
            attempt: 0,
            run_id: "run".into(),
            flow_key: "flow".into(),
            node_id: "node".into(),
            sequence,
            kind,
            summary: String::new(),
            detail: Some(detail),
        };
        view.observe_execution(&event, None, sequence + 1, None, |_| Ok(()))
            .unwrap();
        assert!(view.records().any(|r| matches!(&r.state, State::Port { port, direction: observed, token_type, .. } if port == expected && observed == &direction && !token_type.is_empty())));
    }
}
