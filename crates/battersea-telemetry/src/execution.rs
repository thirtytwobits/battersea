use crate::{
    content::Content,
    otlp::{activation_resource_id, node_record_id},
    view::*,
    Result,
};
use battersea_runtime::{EventKind, ExecutionEvent, Outcome, RunPhase};

impl View {
    /// Convert scheduler observations at one masking boundary. The view remains
    /// useful without a tracing subscriber or exporter installed.
    pub fn observe_execution(
        &mut self,
        event: &ExecutionEvent,
        session_id: Option<String>,
        now_ms: u64,
        mask: Option<&crate::content::ContentMask<'_>>,
        capture: impl FnOnce(&Delta) -> Result<()>,
    ) -> Result<Delta> {
        let detail = event.detail.as_ref();
        let text = |key: &str| {
            detail
                .and_then(|v| v.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let content = || {
            detail
                .and_then(|v| v.get("value"))
                .map(|v| Content::observe(v.to_string().as_bytes(), mask))
        };
        let (id, mut state) = match event.kind {
            EventKind::ActivationAccepted => (
                activation_resource_id(&event.run_id),
                State::Activation {
                    status: Status::Accepted,
                },
            ),
            EventKind::ActivationExecutionFinished => {
                let phase: RunPhase =
                    serde_json::from_value(event.detail.clone().unwrap_or_default())
                        .map_err(|e| crate::Error::Invalid(e.to_string()))?;
                let status = match phase {
                    RunPhase::Accepted => Status::Accepted,
                    RunPhase::CompletionPending => Status::CompletionPending,
                    RunPhase::Terminal { outcome } => match outcome {
                        Outcome::Succeeded => Status::Succeeded,
                        Outcome::Failed { .. } => Status::Failed,
                        Outcome::Cancelled { .. } => Status::Cancelled,
                        Outcome::Interrupted { .. } => Status::Interrupted,
                    },
                };
                (
                    activation_resource_id(&event.run_id),
                    State::Activation { status },
                )
            }
            EventKind::NodeStart
            | EventKind::NodeComplete
            | EventKind::NodeSkipped
            | EventKind::NodeError => {
                let status = match event.kind {
                    EventKind::NodeStart => Status::Running,
                    EventKind::NodeError => Status::Failed,
                    EventKind::NodeComplete
                        if detail
                            .and_then(|v| v.get("complete"))
                            .and_then(|v| v.as_bool())
                            == Some(false) =>
                    {
                        Status::Waiting
                    }
                    _ => Status::Succeeded,
                };
                (
                    node_record_id(&event.run_id, &event.node_id),
                    State::Node {
                        status,
                        error_code: detail
                            .and_then(|v| v.pointer("/error/code"))
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                    },
                )
            }
            EventKind::TokenEmit
            | EventKind::TokenReceive
            | EventKind::TokenSkip
            | EventKind::TokenDrop
            | EventKind::TokenClose
            | EventKind::SignalEmit
            | EventKind::SignalReceive => {
                let direction = if text("direction") == "input"
                    || matches!(
                        event.kind,
                        EventKind::TokenReceive | EventKind::SignalReceive | EventKind::TokenSkip
                    ) {
                    Direction::Input
                } else {
                    Direction::Output
                };
                let port = text(if event.kind == EventKind::SignalEmit {
                    "signalPort"
                } else if direction == Direction::Input {
                    "targetPort"
                } else {
                    "sourcePort"
                });
                let action = match event.kind {
                    EventKind::TokenReceive | EventKind::SignalReceive => PortAction::Receive,
                    EventKind::TokenSkip => PortAction::Skip,
                    EventKind::TokenDrop => PortAction::Drop,
                    EventKind::TokenClose => PortAction::Close,
                    _ => PortAction::Emit,
                };
                (
                    format!(
                        "port:{}:{}:{direction:?}:{port}",
                        event.run_id, event.node_id
                    ),
                    State::Port {
                        port,
                        token_type: if matches!(
                            event.kind,
                            EventKind::SignalEmit | EventKind::SignalReceive
                        ) {
                            "signal".into()
                        } else {
                            text("tokenType")
                        },
                        direction,
                        action,
                        content: content(),
                    },
                )
            }
            _ => (
                format!(
                    "event:{}:{}:{}",
                    event.run_id, event.attempt, event.sequence
                ),
                State::Diagnostic {
                    attributes: Default::default(),
                    category: event.kind.category().into(),
                    message: event.kind.category().into(),
                    content: content(),
                },
            ),
        };
        let previous = self.get(&id);
        if let State::Port { token_type, .. } = &mut state {
            if token_type.is_empty() {
                if let Some(Record {
                    state:
                        State::Port {
                            token_type: known, ..
                        },
                    ..
                }) = previous
                {
                    *token_type = known.clone();
                }
            }
        }
        let now_ms = previous.map_or(now_ms, |record| now_ms.max(record.updated_at_ms));
        let record = Record {
            id: id.clone(),
            context: Context {
                activation_id: event.run_id.clone(),
                flow_key: event.flow_key.clone(),
                node_id: if matches!(state, State::Activation { .. }) {
                    None
                } else {
                    Some(event.node_id.clone())
                },
                session_id,
            },
            started_at_ms: previous.map_or(now_ms, |r| r.started_at_ms),
            updated_at_ms: now_ms,
            state,
        };
        self.record(record, capture)
    }
}
