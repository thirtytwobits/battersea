//! A lossless provider-event pump with capacity reserved before polling the source.
//!
//! The pump owns no graph state. Its consumer applies events and keeps each delivery
//! alive until handling completes. Limits cover queued and borrowed deliveries in
//! compact JSON bytes, not allocator overhead or copies retained by the application.
use futures_util::{Stream, StreamExt};
use serde::Serialize;
use std::{fmt, io, sync::Arc};
use tokio::{
    sync::{mpsc, OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

/// Validated limits for one pump. All three bounds must be positive, and one
/// maximum-sized event must fit within the byte capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PumpLimits {
    items: usize,
    bytes: usize,
    max_event_bytes: u32,
}

impl PumpLimits {
    pub fn new(items: usize, bytes: usize, max_event_bytes: usize) -> Result<Self, PumpError> {
        if items == 0
            || items > Semaphore::MAX_PERMITS
            || bytes == 0
            || bytes > Semaphore::MAX_PERMITS
            || max_event_bytes == 0
            || max_event_bytes > bytes
            || max_event_bytes > u32::MAX as usize
        {
            return Err(PumpError::InvalidLimits);
        }
        Ok(Self {
            items,
            bytes,
            max_event_bytes: max_event_bytes as u32,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PumpError {
    InvalidLimits,
    PayloadTooLarge { limit: usize },
    Serialization(String),
    Cancelled,
    ProducerPanicked,
    ProducerStopped,
}

impl fmt::Display for PumpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => formatter.write_str("Invalid event-pump capacity limits."),
            Self::PayloadTooLarge { limit } => {
                write!(
                    formatter,
                    "Provider event exceeds the {limit}-byte payload limit."
                )
            }
            Self::Serialization(message) => {
                write!(formatter, "Cannot measure provider event: {message}")
            }
            Self::Cancelled => formatter.write_str("Event pump cancelled."),
            Self::ProducerPanicked => formatter.write_str("Provider event pump panicked."),
            Self::ProducerStopped => {
                formatter.write_str("Provider event pump stopped unexpectedly.")
            }
        }
    }
}

impl std::error::Error for PumpError {}

/// An event and its capacity reservation. Dropping this value releases its item
/// and byte permits. Retaining a clone of the payload requires application-owned
/// accounting; there is deliberately no operation that removes the reservation.
#[derive(Debug)]
pub struct EventDelivery<T> {
    value: T,
    encoded_bytes: usize,
    _item: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}

impl<T> EventDelivery<T> {
    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
}

/// Poll a provider stream on its own task, with lossless FIFO delivery.
///
/// One maximum-event reservation is acquired before each source poll and shrunk
/// to the measured size before delivery. This deliberately favours a strict bound
/// over packing small events into the last few bytes of a mailbox. A source may
/// temporarily construct an oversize event; it is rejected before admission.
/// Source implementations own limits on their parsing and transport buffers.
pub struct EventPump<T> {
    receiver: mpsc::Receiver<EventDelivery<T>>,
    task: Option<JoinHandle<Result<(), PumpError>>>,
    cancellation: CancellationToken,
}

impl<T: Serialize + Send + 'static> EventPump<T> {
    /// Requires a Tokio runtime. Cancelling `parent` stops this pump; dropping
    /// the pump stops only its child task and does not cancel `parent`.
    pub fn spawn(
        source: impl Stream<Item = T> + Send + 'static,
        limits: PumpLimits,
        parent: CancellationToken,
    ) -> Self {
        let cancellation = parent.child_token();
        let producer_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel(limits.items);
        let task = tokio::spawn(async move {
            let slots = Arc::new(Semaphore::new(limits.items));
            let bytes = Arc::new(Semaphore::new(limits.bytes));
            let mut source = Box::pin(source);
            let produce = async {
                loop {
                    let item = slots
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| PumpError::ProducerStopped)?;
                    let mut reservation = bytes
                        .clone()
                        .acquire_many_owned(limits.max_event_bytes)
                        .await
                        .map_err(|_| PumpError::ProducerStopped)?;
                    let Some(value) = source.next().await else {
                        return Ok(());
                    };
                    let encoded_bytes = measure(&value, limits.max_event_bytes as usize)?;
                    let unused = limits.max_event_bytes as usize - encoded_bytes;
                    drop(reservation.split(unused));
                    let event = EventDelivery {
                        value,
                        encoded_bytes,
                        _item: item,
                        _bytes: reservation,
                    };
                    if sender.send(event).await.is_err() {
                        return Ok(());
                    }
                }
            };
            tokio::select! {
                biased;
                _ = producer_cancellation.cancelled() => Err(PumpError::Cancelled),
                result = produce => result,
            }
        });
        Self {
            receiver,
            task: Some(task),
            cancellation,
        }
    }
}

impl<T> EventPump<T> {
    /// Receive the next charged delivery. Completion or failure follows all
    /// admitted events; cancellation instead discards queued events immediately.
    /// A terminal error is returned once, then subsequent calls return `None`.
    /// Cancelling this receive future does not detach the producer task.
    pub async fn next(&mut self) -> Result<Option<EventDelivery<T>>, PumpError> {
        if self.task.is_none() {
            return Ok(None);
        }
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => {
                self.receiver.close();
                while self.receiver.try_recv().is_ok() {}
                if let Some(task) = &mut self.task {
                    task.abort();
                    let _ = task.await;
                }
                self.task.take();
                Err(PumpError::Cancelled)
            }
            event = self.receiver.recv() => {
                if let Some(event) = event {
                    return Ok(Some(event));
                }
                let result = self.task.as_mut().expect("live producer handle").await;
                self.task.take();
                match result {
                    Ok(result) => result.map(|()| None),
                    Err(error) if error.is_panic() => Err(PumpError::ProducerPanicked),
                    Err(_) => Err(PumpError::ProducerStopped),
                }
            }
        }
    }
}

impl<T> Drop for EventPump<T> {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

/// Measure without constructing a second encoded copy of a retained payload.
/// Measure the encoded payload without allocating beyond the declared limit.
pub fn measure(value: &impl Serialize, limit: usize) -> Result<usize, PumpError> {
    struct Counter {
        bytes: usize,
        limit: usize,
        exceeded: bool,
    }
    impl io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit - self.bytes {
                self.exceeded = true;
                return Err(io::Error::other("event payload exceeds capacity"));
            }
            self.bytes += bytes.len();
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter {
        bytes: 0,
        limit,
        exceeded: false,
    };
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => Ok(counter.bytes),
        Err(_) if counter.exceeded => Err(PumpError::PayloadTooLarge { limit }),
        Err(error) => Err(PumpError::Serialization(error.to_string())),
    }
}
