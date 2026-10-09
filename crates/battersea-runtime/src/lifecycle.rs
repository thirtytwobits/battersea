//! Accepted execution identity, terminal outcomes and the common activation driver.
use crate::{EventKind, ExecutionError, ExecutionHost};
use async_trait::async_trait;
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, panic::AssertUnwindSafe};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunIdentity {
    pub run_id: String,
    pub flow_key: String,
    pub graph_revision: String,
    pub catalogue_revision: String,
    pub configuration_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed { code: String, message: String },
    Cancelled { message: String },
    Interrupted { message: String },
}
impl Outcome {
    pub fn from_error(error: &impl ExecutionError) -> Self {
        match error.code() {
            "cancelled" => Self::Cancelled {
                message: error.message().into(),
            },
            "interrupted" => Self::Interrupted {
                message: error.message().into(),
            },
            _ => Self::Failed {
                code: error.code().into(),
                message: error.message().into(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum RunPhase {
    Accepted,
    /// Execution drained; the host still owes its durable commit.
    CompletionPending,
    Terminal {
        outcome: Outcome,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub identity: RunIdentity,
    pub phase: RunPhase,
}

/// Storage and product effects at the activation boundary. Acceptance must be
/// durable before returning. A pending completion may be reconciled without
/// running the graph again; it must never authorise provider redispatch.
// async_trait adds must_use to boxed futures; dispatch arguments retain the port contract.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait ActivationHost: ExecutionHost {
    fn validate_activation(&self, runtime: &Self::State) -> Result<(), Self::Error>;
    async fn accept_activation(&self, runtime: &Self::State, node: &str)
        -> Result<(), Self::Error>;
    async fn preflight_activation(&self, runtime: &mut Self::State) -> Result<(), Self::Error>;
    fn controller_activation_effects(
        &self,
        runtime: &mut Self::State,
        node: &str,
        values: &HashMap<String, Value>,
    );
    async fn complete_execution(&self, runtime: &mut Self::State) -> Result<(), Self::Error>;
    async fn retain_execution_phase(
        &self,
        runtime: &Self::State,
        phase: RunPhase,
    ) -> Result<(), Self::Error>;

    async fn run_activation(
        &self,
        runtime: &mut Self::State,
        node: &str,
        values: HashMap<String, Value>,
        token: CancellationToken,
    ) -> Result<(), Self::Error> {
        self.handlers()
            .validate(runtime.definitions.values())
            .map_err(Self::Error::invalid_request)?;
        self.validate_activation(runtime)?;
        runtime.activation_values = values.clone();
        runtime.activation_token = token.clone();
        self.accept_activation(runtime, node).await?;
        self.publish_execution_record(
            runtime,
            node,
            EventKind::ActivationAccepted,
            "Activation accepted.".into(),
            None,
        )
        .await;
        let work = async {
            if token.is_cancelled() {
                return Err(Self::Error::cancelled(
                    "Activation cancelled before execution.",
                ));
            }
            self.preflight_activation(runtime).await?;
            while self.evaluate_ready_logic_nodes(runtime).await? {}
            self.execute_flow_node(runtime, node, Some(&values), &token)
                .await?;
            self.controller_activation_effects(runtime, node, &values);
            self.materialize_snapshot_sources(runtime, node, &token)
                .await?;
            self.drain_flow_work(runtime, &token).await?;
            if token.is_cancelled() {
                return Err(Self::Error::cancelled(
                    "Activation cancelled before completion.",
                ));
            }
            self.complete_execution(runtime).await
        };
        let result = AssertUnwindSafe(work)
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(Self::Error::interrupted("Activation task panicked.")));
        let phase = match &result {
            Ok(()) => RunPhase::CompletionPending,
            Err(error) => RunPhase::Terminal {
                outcome: Outcome::from_error(error),
            },
        };
        // A failed phase write leaves the accepted intent pending on disk.
        self.retain_execution_phase(runtime, phase.clone()).await?;
        self.publish_execution_record(
            runtime,
            node,
            EventKind::ActivationExecutionFinished,
            "Activation execution finished.".into(),
            Some(serde_json::to_value(phase).expect("serialisable phase")),
        )
        .await;
        result
    }
}
