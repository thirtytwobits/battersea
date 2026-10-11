//! Lossless provider payloads carried by bounded transport frames.
use crate::{
    pump::{EventPump, PumpError, PumpLimits},
    retention::{Reservation, RetentionBudget},
};
use futures_util::{Stream, StreamExt};
use serde::{de::DeserializeOwned, Serialize};
use std::{marker::PhantomData, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

#[derive(Serialize)]
#[serde(transparent)]
struct Frame {
    data: String,
    #[serde(skip)]
    last: bool,
    #[serde(skip)]
    error: bool,
    #[serde(skip)]
    permit: Option<Arc<OwnedSemaphorePermit>>,
}

struct Pending {
    json: String,
    offset: usize,
    permit: Arc<OwnedSemaphorePermit>,
    _charge: Reservation,
}

/// A complete provider event. Its reservation remains live until handling ends.
pub struct PayloadDelivery<T> {
    value: T,
    encoded_bytes: usize,
    // Release the payload charge before waking the next producer poll.
    _charge: Reservation,
    _permit: Option<Arc<OwnedSemaphorePermit>>,
}
impl<T> PayloadDelivery<T> {
    pub fn value(&self) -> &T {
        &self.value
    }
    pub fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
}

/// Mailbox limits apply to transport frames. Complete payloads use the node's
/// retention budget, so a tool result can be larger than the mailbox itself.
/// Partial assembly survives cancellation of a receive future; cancellation of
/// the activation or dropping the pump releases both producer and assembly state.
pub struct PayloadPump<T> {
    pump: EventPump<Frame>,
    budget: RetentionBudget,
    node: String,
    json: String,
    charges: Vec<Reservation>,
    failed: bool,
    _payload: PhantomData<T>,
}

impl<T: Serialize + DeserializeOwned + Send + 'static> PayloadPump<T> {
    pub fn spawn(
        source: impl Stream<Item = T> + Send + 'static,
        limits: PumpLimits,
        budget: RetentionBudget,
        node: String,
        cancellation: CancellationToken,
    ) -> Result<Self, PumpError> {
        let overhead = serde_json::to_vec(&Frame {
            data: String::new(),
            last: false,
            error: false,
            permit: None,
        })
        .map_err(|e| PumpError::Serialization(e.to_string()))?
        .len();
        // A JSON string requires at most six encoded bytes per source byte.
        // Small frames still carry one complete UTF-8 scalar at a time.
        let chunk = limits
            .max_event_bytes()
            .checked_sub(overhead)
            .map(|bytes| bytes / 6)
            .filter(|bytes| *bytes > 0)
            .ok_or(PumpError::InvalidLimits)?;
        let available = Arc::new(Semaphore::new(1));
        let producer_budget = budget.clone();
        let producer_node = node.clone();
        let source = futures_util::stream::unfold(
            (Box::pin(source), None::<Pending>, false),
            move |(mut source, mut pending, stopped)| {
                let budget = producer_budget.clone();
                let node = producer_node.clone();
                let available = available.clone();
                async move {
                    if stopped {
                        return None;
                    }
                    if pending.is_none() {
                        // Retain at most one logical event until its consumer releases it.
                        let permit = Arc::new(available.acquire_owned().await.ok()?);
                        let value = source.next().await?;
                        let encoded = (|| {
                            let charge = budget.retain_payload(&node, &value)?;
                            let json = serde_json::to_string(&value).map_err(|e| e.to_string())?;
                            Ok::<_, String>(Pending {
                                json,
                                offset: 0,
                                permit,
                                _charge: charge,
                            })
                        })();
                        match encoded {
                            Ok(value) => pending = Some(value),
                            Err(message) => {
                                return Some((
                                    Frame {
                                        data: message,
                                        last: true,
                                        error: true,
                                        permit: None,
                                    },
                                    (source, None, true),
                                ))
                            }
                        }
                    }
                    let payload = pending.as_mut().unwrap();
                    let mut end = (payload.offset + chunk).min(payload.json.len());
                    while !payload.json.is_char_boundary(end) {
                        end -= 1;
                    }
                    if end == payload.offset {
                        end += payload.json[payload.offset..]
                            .chars()
                            .next()
                            .unwrap()
                            .len_utf8();
                    }
                    let data = payload.json[payload.offset..end].to_owned();
                    payload.offset = end;
                    let last = end == payload.json.len();
                    let permit = Some(payload.permit.clone());
                    if last {
                        pending = None;
                    }
                    Some((
                        Frame {
                            data,
                            last,
                            error: false,
                            permit,
                        },
                        (source, pending, false),
                    ))
                }
            },
        );
        Ok(Self {
            pump: EventPump::spawn(source, limits, cancellation),
            budget,
            node,
            json: String::new(),
            charges: Vec::new(),
            failed: false,
            _payload: PhantomData,
        })
    }

    /// Only complete payloads are observable. An error releases partial assembly
    /// and is returned once, after all earlier complete payloads.
    pub async fn next(&mut self) -> Result<Option<PayloadDelivery<T>>, PumpError> {
        if self.failed {
            return Ok(None);
        }
        let result = self.receive().await;
        if result.is_err() {
            self.failed = true;
            self.json = String::new();
            self.charges.clear();
            self.pump.cancel();
            let _ = self.pump.next().await;
        }
        result
    }

    async fn receive(&mut self) -> Result<Option<PayloadDelivery<T>>, PumpError> {
        loop {
            let Some(delivery) = self.pump.next().await? else {
                return if self.json.is_empty() {
                    Ok(None)
                } else {
                    Err(PumpError::ProducerStopped)
                };
            };
            let frame = delivery.value();
            if frame.error {
                return Err(PumpError::Capacity(frame.data.clone()));
            }
            self.charges.push(
                self.budget
                    .retain_payload(&self.node, &frame.data)
                    .map_err(PumpError::Capacity)?,
            );
            self.json.push_str(&frame.data);
            if frame.last {
                // Charge the decoded value before releasing its encoded assembly.
                let charge = self
                    .budget
                    .retain_payload(&self.node, &self.json)
                    .map_err(PumpError::Capacity)?;
                let value = serde_json::from_str(&self.json)
                    .map_err(|e| PumpError::Serialization(e.to_string()))?;
                let encoded_bytes = self.json.len();
                self.json = String::new();
                self.charges.clear();
                return Ok(Some(PayloadDelivery {
                    value,
                    encoded_bytes,
                    _permit: frame.permit.clone(),
                    _charge: charge,
                }));
            }
        }
    }
}
