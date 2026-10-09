//! M6.1 execution contract: lossless admission, retained bytes and cancellation.
use battersea_runtime::pump::{EventPump, PumpError, PumpLimits};
use futures_util::{stream, Stream, StreamExt};
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

struct ObservedStream {
    polled: Arc<AtomicUsize>,
    notified: Arc<Notify>,
    dropped: Arc<AtomicUsize>,
    payload: String,
}

impl Stream for ObservedStream {
    type Item = String;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.polled.fetch_add(1, Ordering::SeqCst);
        self.notified.notify_one();
        Poll::Ready(Some(self.payload.clone()))
    }
}

impl Drop for ObservedStream {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

fn observed(payload: &str) -> ObservedStream {
    ObservedStream {
        polled: Arc::default(),
        notified: Arc::default(),
        dropped: Arc::default(),
        payload: payload.into(),
    }
}

#[tokio::test]
async fn lossless_pump_preserves_input_order_and_reports_completion_once() {
    let input = vec!["one", "a differently sized event", "三", ""];
    let maximum = input
        .iter()
        .map(|v| serde_json::to_vec(v).unwrap().len())
        .max()
        .unwrap();
    let mut pump = EventPump::spawn(
        stream::iter(input.clone()),
        PumpLimits::new(2, maximum * 2, maximum).unwrap(),
        CancellationToken::new(),
    );
    let mut output = Vec::new();
    while let Some(event) = pump.next().await.unwrap() {
        assert_eq!(
            event.encoded_bytes(),
            serde_json::to_vec(event.value()).unwrap().len()
        );
        output.push(*event.value());
    }
    assert_eq!(output, input);
    assert!(pump.next().await.unwrap().is_none());
}

#[tokio::test]
async fn received_payload_keeps_its_item_reservation_until_released() {
    let source = observed("payload");
    let polls = source.polled.clone();
    let mut pump = EventPump::spawn(
        source,
        PumpLimits::new(1, 128, 64).unwrap(),
        CancellationToken::new(),
    );
    let delivery = pump.next().await.unwrap().unwrap();
    tokio::task::yield_now().await;
    // Polling a next delivery must remain pending even though the channel slot is empty.
    assert!(futures_util::poll!(Box::pin(pump.next())).is_pending());
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    drop(delivery);
    let next = pump.next().await.unwrap().unwrap();
    assert!(polls.load(Ordering::SeqCst) > 1);
    drop(next);
}

#[tokio::test]
async fn byte_capacity_blocks_polling_even_when_item_slots_are_available() {
    let payload = "a payload containing multibyte UTF-8: 三";
    let size = serde_json::to_vec(payload).unwrap().len();
    let source = observed(payload);
    let polls = source.polled.clone();
    let mut pump = EventPump::spawn(
        source,
        PumpLimits::new(8, size, size).unwrap(),
        CancellationToken::new(),
    );
    let delivery = pump.next().await.unwrap().unwrap();
    tokio::task::yield_now().await;
    assert!(futures_util::poll!(Box::pin(pump.next())).is_pending());
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    drop(delivery);
    assert!(pump.next().await.unwrap().is_some());
}

#[tokio::test]
async fn cancellation_bypasses_a_full_queue_and_drops_the_provider() {
    let source = observed("value");
    let notified = source.notified.clone();
    let dropped = source.dropped.clone();
    let token = CancellationToken::new();
    let mut pump = EventPump::spawn(source, PumpLimits::new(1, 64, 64).unwrap(), token.clone());
    notified.notified().await;
    token.cancel();
    assert!(matches!(pump.next().await, Err(PumpError::Cancelled)));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(pump.next().await.unwrap().is_none());
}

#[tokio::test]
async fn cancellation_interrupts_an_idle_provider_poll() {
    let token = CancellationToken::new();
    let mut pump = EventPump::spawn(
        stream::pending::<String>(),
        PumpLimits::new(1, 64, 64).unwrap(),
        token.clone(),
    );
    assert!(futures_util::poll!(Box::pin(pump.next())).is_pending());
    token.cancel();
    assert!(matches!(pump.next().await, Err(PumpError::Cancelled)));
}

#[tokio::test]
async fn oversize_event_fails_without_delivery_and_releases_the_source() {
    let payload = "larger than the permitted encoded payload";
    let size = serde_json::to_vec(payload).unwrap().len();
    let source = observed(payload);
    let dropped = source.dropped.clone();
    let mut pump = EventPump::spawn(
        source,
        PumpLimits::new(1, size, size - 1).unwrap(),
        CancellationToken::new(),
    );
    assert!(matches!(
        pump.next().await,
        Err(PumpError::PayloadTooLarge { .. })
    ));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(pump.next().await.unwrap().is_none());
}

#[tokio::test]
async fn producer_panic_is_observed_after_earlier_admitted_events() {
    let source =
        stream::iter(["admitted"]).chain(stream::poll_fn(|_| -> Poll<Option<&'static str>> {
            panic!("provider fault")
        }));
    let mut pump = EventPump::spawn(
        source,
        PumpLimits::new(2, 128, 64).unwrap(),
        CancellationToken::new(),
    );
    assert!(pump.next().await.unwrap().is_some());
    assert!(matches!(
        pump.next().await,
        Err(PumpError::ProducerPanicked)
    ));
    assert!(pump.next().await.unwrap().is_none());
}

#[tokio::test]
async fn dropping_consumer_stops_pump_without_cancelling_its_parent() {
    let source = observed("value");
    let dropped = source.dropped.clone();
    let parent = CancellationToken::new();
    let mut pump = EventPump::spawn(source, PumpLimits::new(1, 64, 64).unwrap(), parent.clone());
    let delivery = pump.next().await.unwrap().unwrap();
    drop(pump);
    // Join the runtime's scheduling opportunity; no wall-clock timing assertion.
    tokio::task::yield_now().await;
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(!parent.is_cancelled());
    drop(delivery);
}

#[test]
fn limits_must_be_positive_and_allow_one_maximum_event() {
    for (items, bytes, event) in [
        (0, 64, 32),
        (1, 0, 1),
        (1, 64, 0),
        (1, 32, 64),
        (usize::MAX, 64, 32),
    ] {
        assert!(PumpLimits::new(items, bytes, event).is_err());
    }
}

#[tokio::test]
async fn unused_byte_reservation_is_available_to_the_next_event() {
    let input = vec!["small", "a larger event with escaped characters: \"\n\t"];
    let sizes: Vec<_> = input
        .iter()
        .map(|v| serde_json::to_vec(v).unwrap().len())
        .collect();
    let mut pump = EventPump::spawn(
        stream::iter(input.clone()),
        PumpLimits::new(2, sizes.iter().sum(), *sizes.iter().max().unwrap()).unwrap(),
        CancellationToken::new(),
    );
    let first = pump.next().await.unwrap().unwrap();
    let second = pump.next().await.unwrap().unwrap();
    assert_eq!(
        [*first.value(), *second.value()].as_slice(),
        input.as_slice()
    );
}

#[tokio::test]
async fn already_cancelled_activation_does_not_poll_the_provider() {
    let source = observed("value");
    let polls = source.polled.clone();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut pump = EventPump::spawn(source, PumpLimits::new(1, 64, 64).unwrap(), cancellation);
    assert!(matches!(pump.next().await, Err(PumpError::Cancelled)));
    assert_eq!(polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn serialisation_failure_is_reported_without_delivery() {
    struct Invalid;
    impl serde::Serialize for Invalid {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("invalid event"))
        }
    }
    let mut pump = EventPump::spawn(
        stream::iter([Invalid]),
        PumpLimits::new(1, 64, 64).unwrap(),
        CancellationToken::new(),
    );
    assert!(matches!(
        pump.next().await,
        Err(PumpError::Serialization(_))
    ));
    assert!(pump.next().await.unwrap().is_none());
}
