//! Edge marking: what each edge holds between its source and its target.
//!
//! An edge holds the deliveries queued on it and, for a token edge, the close that ends its
//! stream. A close is pending while it waits behind the edge's queued deliveries and delivered
//! once the target's input has closed. The scheduler publishes the marking of every edge whose
//! contents change as a `flow.edge.marking` record; each entry replaces that edge's previous
//! entry.
use crate::{delivery::Control, EventKind, ExecutionHost, SchedulerState};
use battersea_flow::{FlowEdge, FlowEdgeKind};
use serde_json::{json, Map, Value};

impl SchedulerState {
    fn edge_marking(&self, edge: &FlowEdge) -> Value {
        let queued = self
            .data_queue
            .iter()
            .filter(|delivery| delivery.edge.id == edge.id)
            .count()
            + self
                .control_queue
                .iter()
                .filter(
                    |delivery| matches!(&delivery.control, Control::Signal(e) if e.id == edge.id),
                )
                .count();
        let mut marking = json!({ "queued": queued });
        if edge.kind == FlowEdgeKind::Token {
            let source = (edge.source_node_id.clone(), edge.source_port.clone());
            let target = (edge.target_node_id.clone(), edge.target_port.clone());
            let close = if self.closed_inputs.contains(&target) {
                Some(json!({"delivered": true, "failed": self.failed_inputs.contains(&target)}))
            } else if self.control_queue.iter().any(|delivery| {
                matches!(&delivery.control, Control::Close { node, port }
                    if *node == edge.source_node_id && *port == edge.source_port)
            }) {
                Some(json!({"delivered": false, "failed": self.failed_outputs.contains(&source)}))
            } else {
                None
            };
            if let Some(close) = close {
                marking["close"] = close;
            }
        }
        marking
    }

    /// The marking of each named edge, keyed by edge id. Unknown ids are omitted.
    pub(crate) fn marking_of<'a>(&self, edge_ids: impl IntoIterator<Item = &'a str>) -> Value {
        let mut edges = Map::new();
        for id in edge_ids {
            if let Some(edge) = self.flow.edges.iter().find(|edge| edge.id == id) {
                edges.insert(edge.id.clone(), self.edge_marking(edge));
            }
        }
        json!({ "edges": edges })
    }

    /// The marking of every edge in the flow.
    pub(crate) fn full_marking(&self) -> Value {
        let ids: Vec<&str> = self
            .flow
            .edges
            .iter()
            .map(|edge| edge.id.as_str())
            .collect();
        self.marking_of(ids)
    }
}

/// Publish the marking of the named edges.
pub(crate) async fn publish<H: ExecutionHost>(
    host: &H,
    runtime: &H::State,
    node_id: &str,
    summary: &str,
    edge_ids: Vec<String>,
) {
    if edge_ids.is_empty() {
        return;
    }
    let detail = runtime.marking_of(edge_ids.iter().map(String::as_str));
    host.publish_execution_record(
        runtime,
        node_id,
        EventKind::EdgeMarking,
        summary.into(),
        Some(detail),
    )
    .await;
}

/// Publish the marking of every edge in the flow.
pub(crate) async fn publish_all<H: ExecutionHost>(
    host: &H,
    runtime: &H::State,
    node_id: &str,
    summary: &str,
) {
    if runtime.flow.edges.is_empty() {
        return;
    }
    let detail = runtime.full_marking();
    host.publish_execution_record(
        runtime,
        node_id,
        EventKind::EdgeMarking,
        summary.into(),
        Some(detail),
    )
    .await;
}
