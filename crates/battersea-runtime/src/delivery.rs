use crate::{pump::EventPump, retention::Reservation, Retained, Token};
use battersea_flow::{FlowEdge, FlowPortPhase};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An owned provider task emits these events; only the driver mutates graph state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderEvent {
    Token { port: String, value: Token },
    Signal { port: String },
    Data { value: Value },
    Failure { message: String },
}

pub(crate) type Cause = Vec<(String, String)>;
#[derive(Debug)]
pub(crate) struct DataDelivery {
    pub emission: u64,
    pub sequence: u64,
    pub edge: FlowEdge,
    pub value: Retained<Token>,
    pub cause: Cause,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Control {
    Signal(FlowEdge),
    Controller(String),
    Close { node: String, port: String },
}
#[derive(Debug)]
pub(crate) struct ControlDelivery {
    pub sequence: u64,
    pub control: Control,
    pub cause: Cause,
    pub _charge: Reservation,
}
pub(crate) struct Producer {
    pub pump: EventPump<ProviderEvent>,
    pub phase: FlowPortPhase,
    // Reserve the mailbox's full capacity, including its borrowed delivery. This
    // keeps all provider mailboxes inside the activation's aggregate bound.
    pub _capacity: Reservation,
}
impl std::fmt::Debug for Producer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Producer")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}
