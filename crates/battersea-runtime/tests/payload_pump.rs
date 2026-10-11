//! A transport frame limit must not become a maximum tool-result size.
use battersea_runtime::{
    payload_pump::PayloadPump,
    pump::{PumpError, PumpLimits},
    ProviderEvent, RetentionBudget,
};
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn payloads_larger_than_the_entire_mailbox_arrive_intact_in_order() {
    let input = vec![
        json!(null),
        json!({"tool":"session_get", "result":"東京\n\"\\".repeat(100_000)}),
        json!([true, 7, "done"]),
    ];
    let budget = RetentionBudget::new(100, 32 * 1024 * 1024, 16 * 1024 * 1024);
    let mut pump = PayloadPump::spawn(
        stream::iter(input.clone()),
        PumpLimits::new(2, 8192, 4096).unwrap(),
        budget.clone(),
        "chat".into(),
        CancellationToken::new(),
    )
    .unwrap();
    let mut output = Vec::new();
    while let Some(delivery) = pump.next().await.unwrap() {
        assert_eq!(
            delivery.encoded_bytes(),
            serde_json::to_vec(delivery.value()).unwrap().len()
        );
        output.push(delivery.value().clone());
        assert!(budget.usage().bytes > 0, "borrowed event is still charged");
    }
    assert_eq!(output, input);
    assert_eq!(budget.usage().bytes, 0);
}

#[tokio::test]
async fn failures_follow_complete_events_without_delivering_partial_payloads() {
    let budget = RetentionBudget::new(100, 8192, 4096);
    let mut pump = PayloadPump::spawn(
        stream::iter([json!("first"), json!("x".repeat(10_000))]),
        PumpLimits::new(2, 1024, 512).unwrap(),
        budget.clone(),
        "chat".into(),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(pump.next().await.unwrap().unwrap().value(), &json!("first"));
    assert!(matches!(pump.next().await, Err(PumpError::Capacity(_))));
    assert!(pump.next().await.unwrap().is_none());
    drop(pump);
    tokio::task::yield_now().await;
    assert_eq!(budget.usage().bytes, 0);
}

#[tokio::test]
async fn cancelling_a_full_framed_mailbox_releases_the_payload() {
    let budget = RetentionBudget::new(100, 16 * 1024 * 1024, 16 * 1024 * 1024);
    let token = CancellationToken::new();
    let mut pump = PayloadPump::spawn(
        stream::iter([json!("large".repeat(100_000))]).chain(stream::pending()),
        PumpLimits::new(1, 1024, 1024).unwrap(),
        budget.clone(),
        "chat".into(),
        token.clone(),
    )
    .unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(budget.usage().bytes > 0);
    token.cancel();
    assert!(matches!(pump.next().await, Err(PumpError::Cancelled)));
    assert_eq!(budget.usage().bytes, 0);
    assert!(pump.next().await.unwrap().is_none());
}

#[tokio::test]
async fn interrupted_receive_retains_assembly_and_never_exposes_a_fragment() {
    let input = json!({"session":"safe\n東京".repeat(100_000)});
    let budget = RetentionBudget::new(100, 32 * 1024 * 1024, 16 * 1024 * 1024);
    let mut pump = PayloadPump::spawn(
        stream::iter([input.clone()]),
        PumpLimits::new(1, 1024, 1024).unwrap(),
        budget.clone(),
        "chat".into(),
        CancellationToken::new(),
    )
    .unwrap();
    for _ in 0..10 {
        assert!(futures_util::poll!(Box::pin(pump.next())).is_pending());
        tokio::task::yield_now().await;
    }
    let delivery = pump.next().await.unwrap().unwrap();
    assert_eq!(delivery.value(), &input);
    drop(delivery);
    assert!(pump.next().await.unwrap().is_none());
    assert_eq!(budget.usage().bytes, 0);
}

#[tokio::test]
async fn provider_event_variants_round_trip_without_losing_metadata() {
    let input = vec![
        ProviderEvent::Data {
            value: json!({"kind":"tool_call_completed","result":"x".repeat(1_100_000)}),
        },
        ProviderEvent::Signal {
            port: "done".into(),
        },
        ProviderEvent::Failure {
            message: "provider failed".into(),
        },
    ];
    let expected = serde_json::to_value(&input).unwrap();
    let budget = RetentionBudget::new(100, 16 * 1024 * 1024, 16 * 1024 * 1024);
    let mut pump = PayloadPump::spawn(
        stream::iter(input),
        PumpLimits::new(2, 8192, 4096).unwrap(),
        budget,
        "node".into(),
        CancellationToken::new(),
    )
    .unwrap();
    let mut output = Vec::<Value>::new();
    while let Some(delivery) = pump.next().await.unwrap() {
        output.push(serde_json::to_value(delivery.value()).unwrap());
    }
    assert_eq!(json!(output), expected);
}

#[tokio::test]
async fn holding_a_complete_delivery_backpressures_the_next_provider_poll() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let polls = Arc::new(AtomicUsize::new(0));
    let observed = polls.clone();
    let source = stream::repeat(json!({"result":"payload"})).inspect(move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
    });
    let budget = RetentionBudget::new(100, 8192, 8192);
    let mut pump = PayloadPump::spawn(
        source,
        PumpLimits::new(4, 2048, 512).unwrap(),
        budget.clone(),
        "node".into(),
        CancellationToken::new(),
    )
    .unwrap();
    let first = pump.next().await.unwrap().unwrap();
    let after_first = polls.load(Ordering::SeqCst);
    assert!(futures_util::poll!(Box::pin(pump.next())).is_pending());
    assert_eq!(polls.load(Ordering::SeqCst), after_first);
    drop(first);
    let next = pump.next().await.unwrap().unwrap();
    assert!(polls.load(Ordering::SeqCst) > after_first);
    drop(next);
    drop(pump);
    tokio::task::yield_now().await;
    assert_eq!(budget.usage().bytes, 0);
}

#[tokio::test]
async fn small_transport_frames_preserve_unicode_and_escaping() {
    for capacity in [8, 25, 32, 64, 127, 4096] {
        let input = json!({"text":"\u{0000}\n\"\\東京🦀".repeat(100)});
        let budget = RetentionBudget::new(100, 1_000_000, 1_000_000);
        let mut pump = PayloadPump::spawn(
            stream::iter([input.clone()]),
            PumpLimits::new(1, capacity, capacity).unwrap(),
            budget.clone(),
            "node".into(),
            CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(pump.next().await.unwrap().unwrap().value(), &input);
        assert!(pump.next().await.unwrap().is_none());
        assert_eq!(budget.usage().bytes, 0);
    }
}
