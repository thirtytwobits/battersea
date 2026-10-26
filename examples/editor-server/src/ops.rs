//! Battersea's operations over the example's stored flows and activations, served to the CLI and
//! to agent hosts through the daemon's MCP server.
// async-trait adds must_use to boxed futures, which already carry it.
#![allow(clippy::double_must_use)]
use crate::{
    history, key, list, read, runtime::codes, snapshot, start, upgrade, Activation, App, Error,
    History, Position,
};
use async_trait::async_trait;
use axum::extract::State;
use battersea_flow::{FlowDocument, FlowValidationResult};
use battersea_ops::{
    codes as ops, Activation as Request, FlowHost, FlowSummary, FlowUpgrade, RunHost, RunStatus,
};
use clapham::{OAuthContext, RecordSink, Refusal};
use serde_json::{json, Value};

/// The example application, as a host of Battersea's operations.
pub struct Host(pub App);

/// An application refusal under the operations' declared codes. `missing` names what an unknown
/// key refers to.
fn refused(error: Error, missing: &str) -> Refusal {
    let code = match error.reason.unwrap_or(error.code.as_str()) {
        codes::INVALID_KEY | codes::UNKNOWN_ACTIVATION => missing,
        codes::INVALID_DOCUMENT | codes::UPGRADE_REFUSED | codes::FLOW_KEY_MISMATCH => {
            ops::INVALID_FLOW
        }
        "internal" => ops::HOST_FAILURE,
        code => code,
    };
    ops::refusal(code, error.message)
}

fn flow(error: Error) -> Refusal {
    refused(error, ops::UNKNOWN_FLOW)
}

fn run(error: Error) -> Refusal {
    refused(error, ops::UNKNOWN_RUN)
}

/// The records a watcher that has delivered up to `delivered` sends next from `history`, and
/// the position it has then delivered up to. When the view has evicted deltas the watcher has not
/// seen, a `history.gap` record says so before the retained events.
fn deliver(
    run_id: &str,
    delivered: Option<Position>,
    history: History,
) -> (Vec<Value>, Option<Position>) {
    let mut records = Vec::new();
    let mut delivered = delivered;
    if let Some((revision, _)) = delivered {
        if history.follows > revision {
            records.push(json!({
                "activation_id": run_id,
                "phase": "history.gap",
                "detail": {
                    "after_sequence": revision.to_string(),
                    "retained_after_sequence": history.follows.to_string(),
                },
            }));
            delivered = Some((history.follows, usize::MAX));
        }
    }
    for (position, event) in history.events {
        if delivered.is_none_or(|delivered| position > delivered) {
            records.push(event);
            delivered = Some(position);
        }
    }
    (records, delivered)
}

impl Host {
    /// Refuses a flow key that names no stored flow as `unknown_flow`.
    fn exists(&self, flow_key: &str) -> Result<(), Refusal> {
        let path = self
            .0
            .runtime
            .root
            .join("flows")
            .join(format!("{}.json", key(flow_key).map_err(flow)?));
        if path.is_file() {
            Ok(())
        } else {
            Err(ops::refusal(
                ops::UNKNOWN_FLOW,
                format!("No flow \"{flow_key}\"."),
            ))
        }
    }
    fn status(&self, run_id: &str) -> Result<RunStatus, Refusal> {
        let (status, _) = snapshot(&self.0, run_id).map_err(run)?;
        Ok(RunStatus {
            run_id: run_id.into(),
            terminal: status != "running",
            status,
            detail: None,
        })
    }
}

#[async_trait]
impl FlowHost for Host {
    async fn list(&self) -> Result<Vec<FlowSummary>, Refusal> {
        let summaries = list(State(self.0.clone())).await.map_err(flow)?.0;
        serde_json::from_value(summaries)
            .map_err(|error| ops::refusal(ops::HOST_FAILURE, error.to_string()))
    }
    async fn get(&self, flow_key: &str) -> Result<FlowDocument, Refusal> {
        self.exists(flow_key)?;
        read(&self.0, flow_key).map_err(|error| ops::refusal(ops::INVALID_FLOW, error.message))
    }
    async fn validate(&self, flow_key: &str) -> Result<FlowValidationResult, Refusal> {
        let document = self.get(flow_key).await?;
        Ok(self.0.runtime.catalog.validate(&document))
    }
    async fn plan_upgrade(&self, flow_key: &str) -> Result<FlowUpgrade, Refusal> {
        self.exists(flow_key)?;
        let plan = upgrade::plan(&self.0, flow_key).map_err(flow)?;
        Ok(FlowUpgrade {
            flow_key: flow_key.into(),
            source_revision: plan.revision,
            document: plan.document,
            applied: false,
        })
    }
    async fn apply_upgrade(
        &self,
        flow_key: &str,
        expected_revision: &str,
    ) -> Result<FlowUpgrade, Refusal> {
        self.exists(flow_key)?;
        let receipt = upgrade::apply_at(&self.0, flow_key, expected_revision).map_err(flow)?;
        Ok(FlowUpgrade {
            flow_key: flow_key.into(),
            source_revision: receipt.source_revision,
            document: receipt.flow,
            applied: true,
        })
    }
}

#[async_trait]
impl RunHost for Host {
    async fn activate(
        &self,
        _: &OAuthContext,
        activation: Request,
        events: &RecordSink,
    ) -> Result<RunStatus, Refusal> {
        // Classify a missing or unreadable flow before the activation reads it.
        self.get(&activation.flow_key).await?;
        let run_id = start(
            &self.0,
            Activation {
                flow_key: activation.flow_key,
                node_id: activation.node_id,
                parameters: activation.parameters.into_iter().collect(),
            },
        )
        .await
        .map_err(flow)?;
        self.watch(&run_id, events).await
    }
    async fn status(&self, run_id: &str) -> Result<RunStatus, Refusal> {
        Host::status(self, run_id)
    }
    async fn watch(&self, run_id: &str, events: &RecordSink) -> Result<RunStatus, Refusal> {
        let mut delivered = None;
        loop {
            // Register for the next change before reading, so no change between them is lost.
            let changed = self.0.runtime.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let history = history(&self.0, run_id).map_err(run)?;
            let status = history.status.clone();
            let records;
            (records, delivered) = deliver(run_id, delivered, history);
            for record in records {
                events.send(record).await?;
            }
            if status != "running" {
                return Host::status(self, run_id);
            }
            changed.await;
        }
    }
    async fn cancel(&self, run_id: &str) -> Result<RunStatus, Refusal> {
        key(run_id).map_err(run)?;
        self.0
            .runs
            .lock()
            .unwrap()
            .get(run_id)
            .ok_or_else(|| ops::refusal(ops::UNKNOWN_RUN, format!("No run \"{run_id}\".")))?
            .token
            .cancel();
        Host::status(self, run_id)
    }
    async fn resume(&self, _: &str) -> Result<RunStatus, Refusal> {
        Err(ops::refusal(
            battersea_runtime::error_codes::UNSUPPORTED_HOST_OPERATION,
            "The example retains no durable execution to resume.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(follows: u64, positions: &[Position]) -> History {
        History {
            status: "running".into(),
            events: positions
                .iter()
                .map(|position| (*position, json!({"sequence": position.0.to_string()})))
                .collect(),
            follows,
        }
    }

    #[test]
    fn a_watcher_sends_each_event_once_as_the_view_slides() {
        let (records, delivered) = deliver("run", None, history(0, &[(1, 0), (2, 0), (2, 1)]));
        assert_eq!(records.len(), 3);
        assert_eq!(delivered, Some((2, 1)));
        // The view evicted revision 1 and retained the rest; nothing is sent twice or skipped.
        let (records, delivered) = deliver("run", delivered, history(1, &[(2, 0), (2, 1), (3, 0)]));
        assert_eq!(records, vec![json!({"sequence": "3"})]);
        assert_eq!(delivered, Some((3, 0)));
    }

    #[test]
    fn a_watcher_reports_events_the_view_evicted_before_it_saw_them() {
        let (records, delivered) = deliver("run", Some((3, 0)), history(6, &[(7, 0)]));
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["phase"], "history.gap");
        assert_eq!(records[0]["detail"]["after_sequence"], "3");
        assert_eq!(records[0]["detail"]["retained_after_sequence"], "6");
        assert_eq!(records[1], json!({"sequence": "7"}));
        // A later read reports the gap once.
        let (records, _) = deliver("run", delivered, history(6, &[(7, 0), (8, 0)]));
        assert_eq!(records, vec![json!({"sequence": "8"})]);
    }
}
