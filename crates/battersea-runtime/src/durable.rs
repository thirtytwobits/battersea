//! Checkpoint publication at resumable activation-driver boundaries.
use crate::{
    checkpoint::SchedulerCheckpoint, recovery::*, ExecutionError, ExecutionHost, RunIdentity,
    SchedulerState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const EXECUTION_CHECKPOINT_VERSION: u32 = 1;

pub struct Durability {
    pub store: Arc<dyn JournalStore>,
    pub configuration_revision: String,
}

pub fn identity(
    state: &SchedulerState,
    configuration_revision: String,
) -> Result<RunIdentity, RecoveryError> {
    let mut graph = state.flow.clone();
    graph.nodes = graph
        .nodes
        .iter()
        .map(|node| {
            state
                .nodes_by_id
                .get(&node.id)
                .cloned()
                .ok_or_else(|| RecoveryError::Invalid("Missing accepted node.".into()))
        })
        .collect::<Result<_, _>>()?;
    let graph = battersea_flow::document::serialize_canonical_document(&graph)
        .map_err(|error| RecoveryError::Invalid(error.to_string()))?;
    let definitions: BTreeMap<_, _> = state.definitions.iter().collect();
    let definitions = serde_json::to_vec(&definitions)
        .map_err(|error| RecoveryError::Invalid(error.to_string()))?;
    Ok(RunIdentity {
        run_id: state.run_id.clone(),
        flow_key: state.flow.flow_key.clone(),
        graph_revision: format!("{:x}", Sha256::digest(graph.as_bytes())),
        catalogue_revision: format!("{:x}", Sha256::digest(&definitions)),
        configuration_revision,
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Continuation {
    #[default]
    Preflight,
    Activate,
    Sources,
    Drain,
    Complete,
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCheckpoint {
    pub version: u32,
    pub scheduler: SchedulerCheckpoint,
    pub continuation: Continuation,
    pub entry_node: String,
    pub handler_versions: BTreeMap<String, u32>,
    pub host: VersionedState,
}
impl ExecutionCheckpoint {
    pub fn from_state(state: &VersionedState) -> Result<Self, RecoveryError> {
        if state.version != EXECUTION_CHECKPOINT_VERSION {
            return Err(RecoveryError::Incompatible(format!(
                "execution state version {}",
                state.version
            )));
        }
        let checkpoint: Self = serde_json::from_value(state.value.clone())
            .map_err(|error| RecoveryError::Invalid(error.to_string()))?;
        if checkpoint.version != EXECUTION_CHECKPOINT_VERSION {
            return Err(RecoveryError::Incompatible(format!(
                "execution checkpoint version {}",
                checkpoint.version
            )));
        }
        Ok(checkpoint)
    }
    pub fn into_state(self) -> Result<VersionedState, RecoveryError> {
        Ok(VersionedState {
            version: EXECUTION_CHECKPOINT_VERSION,
            value: serde_json::to_value(self)
                .map_err(|error| RecoveryError::Invalid(error.to_string()))?,
        })
    }
    pub fn restore<H: ExecutionHost>(&self, host: &H) -> Result<SchedulerState, H::Error> {
        let mut scheduler = self.scheduler.restore().map_err(recovery_error::<H>)?;
        if !scheduler.nodes_by_id.contains_key(&self.entry_node)
            || scheduler.activated_node_id.as_deref() != Some(&self.entry_node)
        {
            return Err(H::Error::invalid_request(
                "Checkpoint activation entry changed.",
            ));
        }
        if self.handler_versions != handler_versions(host, &scheduler)? {
            return Err(H::Error::invalid_request(
                "Checkpoint handler-state versions are incompatible.",
            ));
        }
        scheduler.continuation = self.continuation;
        Ok(scheduler)
    }
}

#[derive(Debug)]
pub(crate) struct DurableExecution {
    pub journal: Journal,
    pub depth: usize,
    pub pending: Vec<String>,
    pub failed: bool,
}
impl DurableExecution {
    pub fn new(journal: Journal) -> Self {
        Self {
            journal,
            depth: 0,
            pending: Vec::new(),
            failed: false,
        }
    }
}

pub(crate) fn recovery_error<H: ExecutionHost>(error: RecoveryError) -> H::Error {
    H::Error::recovery_required(error.to_string())
}

pub(crate) fn handler_versions<H: ExecutionHost>(
    host: &H,
    runtime: &SchedulerState,
) -> Result<BTreeMap<String, u32>, H::Error> {
    let mut versions = BTreeMap::new();
    for node in runtime.nodes_by_id.values() {
        let definition = runtime
            .definitions
            .get(&node.definition_name)
            .ok_or_else(|| H::Error::invalid_request("Missing checkpoint definition."))?;
        let handler = host
            .handlers()
            .get(&definition.handler_id)
            .map_err(H::Error::invalid_request)?;
        let version = handler.checkpoint_version();
        if version == 0 {
            return Err(H::Error::invalid_request(
                "Handler checkpoint versions must be positive.",
            ));
        }
        versions.insert(definition.handler_id.clone(), version);
    }
    Ok(versions)
}

pub(crate) fn capture<H: ExecutionHost>(
    host: &H,
    runtime: &H::State,
) -> Result<VersionedState, H::Error> {
    ExecutionCheckpoint {
        version: EXECUTION_CHECKPOINT_VERSION,
        scheduler: SchedulerCheckpoint::capture(runtime).map_err(recovery_error::<H>)?,
        continuation: runtime.continuation,
        entry_node: runtime
            .activated_node_id
            .clone()
            .ok_or_else(|| H::Error::internal("Checkpoint has no entry node."))?,
        handler_versions: handler_versions(host, runtime)?,
        host: host.checkpoint_host_state(runtime)?,
    }
    .into_state()
    .map_err(recovery_error::<H>)
}

pub(crate) fn begin<H: ExecutionHost>(
    runtime: &mut H::State,
    node: &str,
    operation: &str,
) -> Result<(), H::Error> {
    let Some(recovery) = runtime.recovery.as_mut() else {
        return Ok(());
    };
    if recovery.failed {
        return Err(H::Error::interrupted(
            "Execution requires recovery after a failed checkpoint operation.",
        ));
    }
    if recovery.depth == 0 && recovery.pending.is_empty() {
        match recovery.journal.begin_effect(
            node,
            operation,
            serde_json::json!({"boundary":"driver_step"}),
        ) {
            Ok(id) => recovery.pending.push(id),
            Err(error) => {
                recovery.failed = true;
                return Err(recovery_error::<H>(error));
            }
        }
    }
    recovery.depth += 1;
    Ok(())
}

pub(crate) fn finish<H: ExecutionHost>(host: &H, runtime: &mut H::State) -> Result<(), H::Error> {
    let Some(recovery) = runtime.recovery.as_mut() else {
        return Ok(());
    };
    recovery.depth = recovery
        .depth
        .checked_sub(1)
        .ok_or_else(|| H::Error::internal("Checkpoint step depth underflow."))?;
    if recovery.depth != 0 || !runtime.producers.is_empty() {
        return Ok(());
    }
    publish(host, runtime)
}

pub(crate) fn publish<H: ExecutionHost>(host: &H, runtime: &mut H::State) -> Result<(), H::Error> {
    if runtime.recovery.is_none() || !runtime.producers.is_empty() {
        return Ok(());
    }
    let checkpoint = match capture(host, runtime) {
        Ok(checkpoint) => checkpoint,
        Err(error) => {
            runtime.recovery.as_mut().unwrap().failed = true;
            return Err(error);
        }
    };
    let recovery = runtime.recovery.as_mut().unwrap();
    let results = recovery
        .pending
        .iter()
        .map(|id| (id.clone(), serde_json::json!({"checkpointed":true})))
        .collect();
    if let Err(error) = recovery.journal.checkpoint(checkpoint, results) {
        recovery.failed = true;
        return Err(recovery_error::<H>(error));
    }
    recovery.pending.clear();
    Ok(())
}
