use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    AutomationWrite,
    AutomationRejected,
    ActionInvoke,
    LogicEvaluate,
    NodeComplete,
    NodeError,
    NodeSkipped,
    NodeStart,
    SignalEmit,
    SignalReceive,
    TokenEmit,
    TokenReceive,
    TokenSkip,
    ActivationAccepted,
    ActivationExecutionFinished,
}
impl EventKind {
    pub fn category(self) -> &'static str {
        match self {
            Self::AutomationWrite => "flow.automation.write",
            Self::AutomationRejected => "flow.automation.rejected",
            Self::ActionInvoke => "flow.action.invoke",
            Self::LogicEvaluate => "flow.logic.evaluate",
            Self::NodeComplete => "flow.node.complete",
            Self::NodeError => "flow.node.error",
            Self::NodeSkipped => "flow.node.skipped",
            Self::NodeStart => "flow.node.start",
            Self::SignalEmit => "flow.signal.emit",
            Self::SignalReceive => "flow.signal.receive",
            Self::TokenEmit => "flow.token.emit",
            Self::TokenReceive => "flow.token.receive",
            Self::TokenSkip => "flow.token.skip",
            Self::ActivationAccepted => "activation.accepted",
            Self::ActivationExecutionFinished => "activation.execution_finished",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub run_id: String,
    pub flow_key: String,
    pub sequence: u64,
    pub node_id: String,
    pub kind: EventKind,
    pub summary: String,
    pub detail: Option<Value>,
}
