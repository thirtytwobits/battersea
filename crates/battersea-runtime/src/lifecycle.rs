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
    fn durability(
        &self,
        _runtime: &Self::State,
    ) -> Result<Option<crate::durable::Durability>, Self::Error> {
        Ok(None)
    }
    fn restore_host_state(
        &self,
        _scheduler: crate::SchedulerState,
        _state: crate::recovery::VersionedState,
    ) -> Result<Self::State, Self::Error> {
        Err(Self::Error::invalid_request(
            "Host does not implement checkpoint restoration.",
        ))
    }
    fn validate_activation(&self, runtime: &Self::State) -> Result<(), Self::Error>;
    async fn accept_activation(&self, runtime: &Self::State, node: &str)
        -> Result<(), Self::Error>;
    async fn preflight_activation(&self, runtime: &mut Self::State) -> Result<(), Self::Error>;
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
        battersea_flow::execution::validate_execution_contract(&runtime.flow, &runtime.definitions)
            .map_err(Self::Error::invalid_request)?;
        self.handlers()
            .validate(runtime.definitions.values())
            .map_err(Self::Error::invalid_request)?;
        self.validate_activation(runtime)?;
        let value_bytes = crate::pump::measure(
            &values,
            runtime.flow.execution.limits.node_retained_bytes as usize,
        )
        .map_err(|e| Self::Error::invalid_request(e.to_string()))?;
        runtime.activation_values_charge = Some(
            runtime
                .retention
                .reserve(0, value_bytes, Some(node))
                .map_err(Self::Error::invalid_request)?,
        );
        runtime.activated_node_id = Some(node.into());
        runtime.activation_values = values.clone();
        runtime.activation_token = token.clone();
        self.accept_activation(runtime, node).await?;
        if let Some(durability) = self.durability(runtime)? {
            let identity = crate::durable::identity(runtime, durability.configuration_revision)
                .map_err(crate::durable::recovery_error::<Self>)?;
            let checkpoint = crate::durable::capture(self, runtime)?;
            let journal = crate::recovery::Journal::accept(durability.store, identity, checkpoint)
                .map_err(crate::durable::recovery_error::<Self>)?;
            runtime.recovery = Some(crate::durable::DurableExecution::new(journal));
        }
        self.publish_execution_record(
            runtime,
            node,
            EventKind::ActivationAccepted,
            "Activation accepted.".into(),
            None,
        )
        .await;
        self.drive_activation(runtime, node, token).await
    }

    fn restore_activation(
        &self,
        journal: &crate::recovery::Journal,
    ) -> Result<Self::State, Self::Error> {
        use crate::durable::{recovery_error, ExecutionCheckpoint};
        let accepted = ExecutionCheckpoint::from_state(&journal.record().accepted_checkpoint)
            .map_err(recovery_error::<Self>)?;
        let checkpoint = ExecutionCheckpoint::from_state(&journal.record().checkpoint)
            .map_err(recovery_error::<Self>)?;
        checkpoint
            .scheduler
            .validate_plan(&accepted.scheduler)
            .map_err(recovery_error::<Self>)?;
        let accepted_scheduler = accepted.restore(self)?;
        let scheduler = checkpoint.restore(self)?;
        let runtime = self.restore_host_state(scheduler, checkpoint.host)?;
        let durability = self
            .durability(&runtime)?
            .ok_or_else(|| Self::Error::invalid_request("Resume requires durable storage."))?;
        let expected =
            crate::durable::identity(&accepted_scheduler, durability.configuration_revision)
                .map_err(recovery_error::<Self>)?;
        journal
            .validate_identity(&expected)
            .map_err(recovery_error::<Self>)?;
        self.validate_activation(&runtime)?;
        Ok(runtime)
    }

    async fn resume_activation(
        &self,
        runtime: &mut Self::State,
        mut journal: crate::recovery::Journal,
        expected_revision: u64,
        token: CancellationToken,
    ) -> Result<(), Self::Error> {
        // Restore again after acquiring ownership, so caller mutations cannot select new work.
        *runtime = self.restore_activation(&journal)?;
        if runtime.cancellation_requested {
            return Err(Self::Error::cancelled(
                "Checkpoint records a cancelled activation.",
            ));
        }
        journal
            .claim_resume(expected_revision)
            .map_err(crate::durable::recovery_error::<Self>)?;
        runtime.event_attempt = journal.record().revision;
        let node = runtime
            .activated_node_id
            .clone()
            .ok_or_else(|| Self::Error::invalid_request("Checkpoint has no activation entry."))?;
        runtime.activation_token = token.clone();
        runtime.recovery = Some(crate::durable::DurableExecution::new(journal));
        self.drive_activation(runtime, &node, token).await
    }

    async fn continue_activation(
        &self,
        runtime: &mut Self::State,
        node: &str,
        token: &CancellationToken,
    ) -> Result<(), Self::Error> {
        use crate::durable::{begin, finish, publish, Continuation};
        loop {
            self.check_cancelled(runtime, token)?;
            match runtime.continuation {
                Continuation::Preflight => {
                    begin::<Self>(runtime, node, "preflight")?;
                    self.preflight_activation(runtime).await?;
                    loop {
                        while self.progress_delivery(runtime, token).await? {}
                        if !self.evaluate_ready_logic_nodes(runtime).await? {
                            break;
                        }
                    }
                    runtime.continuation = Continuation::Activate;
                    finish(self, runtime)?;
                }
                Continuation::Activate => {
                    begin::<Self>(runtime, node, "activate")?;
                    let values = runtime.activation_values.clone();
                    self.execute_flow_node(runtime, node, Some(&values), token)
                        .await?;
                    if runtime.producers.get(node).is_some_and(|producer| {
                        producer.phase == battersea_flow::FlowPortPhase::Snapshot
                    }) {
                        self.await_snapshot_phase(runtime, node, token).await?;
                    }
                    runtime.continuation = Continuation::Sources;
                    finish(self, runtime)?;
                }
                Continuation::Sources => {
                    while self.progress_delivery(runtime, token).await? {}
                    self.materialize_snapshot_sources(runtime, node, token)
                        .await?;
                    runtime.continuation = Continuation::Drain;
                    publish(self, runtime)?;
                }
                Continuation::Drain => {
                    self.drain_flow_work(runtime, token).await?;
                    runtime.continuation = Continuation::Complete;
                    publish(self, runtime)?;
                }
                Continuation::Complete => {
                    begin::<Self>(runtime, node, "complete_execution")?;
                    self.complete_execution(runtime).await?;
                    runtime.continuation = Continuation::Finished;
                    finish(self, runtime)?;
                }
                Continuation::Finished => return Ok(()),
            }
        }
    }

    async fn drive_activation(
        &self,
        runtime: &mut Self::State,
        node: &str,
        token: CancellationToken,
    ) -> Result<(), Self::Error> {
        let work = self.continue_activation(runtime, node, &token);
        let mut result = tokio::select! {
            biased;
            _ = token.cancelled() => Err(Self::Error::cancelled("Flow activation cancelled.")),
            result = AssertUnwindSafe(work).catch_unwind() => result.unwrap_or_else(|_| Err(Self::Error::interrupted("Activation task panicked."))),
        };
        if result.is_ok() && runtime.recovery.is_some() {
            match crate::durable::capture(self, runtime) {
                Ok(checkpoint) => {
                    let recovery = runtime.recovery.as_mut().unwrap();
                    if let Err(error) = recovery.journal.execution_complete(checkpoint) {
                        recovery.failed = true;
                        result = Err(crate::durable::recovery_error::<Self>(error));
                    }
                }
                Err(error) => {
                    runtime.recovery.as_mut().unwrap().failed = true;
                    result = Err(error);
                }
            }
        }
        if result.as_ref().is_err_and(|error| {
            error.code() == "interrupted" || error.code() == "execution_recovery_required"
        }) {
            if let Some(recovery) = runtime.recovery.as_mut() {
                recovery.failed = true;
            }
        }
        if runtime
            .recovery
            .as_ref()
            .is_some_and(|recovery| recovery.failed)
        {
            runtime.producers.clear();
            return Err(Self::Error::recovery_required(
                result
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "Checkpoint publication failed.".into()),
            ));
        }
        if let (Some(recovery), Err(error)) = (runtime.recovery.as_mut(), &result) {
            recovery
                .journal
                .abandon("activation-driver", error.message())
                .map_err(crate::durable::recovery_error::<Self>)?;
        }
        runtime.producers.clear();
        runtime.data_queue.clear();
        runtime.control_queue.clear();
        runtime.input_tokens.clear();
        runtime.active_delivery_groups.clear();
        runtime.activation_values.clear();
        runtime.activation_values_charge.take();
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
