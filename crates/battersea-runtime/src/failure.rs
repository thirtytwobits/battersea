//! Driver-owned handling of explicitly recoverable node failures.
use crate::*;
use battersea_flow::{
    failure::*, FlowFailurePolicy, FlowNode, FlowNodeDefinition, FlowNodeFailure, FlowPortMode,
    FlowPortPhase,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

/// A host confirms that a node failure is recoverable and all effects have settled.
/// Messages must be sanitised before constructing this classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverableNodeFailure {
    pub code: String,
    pub message: String,
}
impl RecoverableNodeFailure {
    pub fn validate(&self) -> bool {
        !self.code.trim().is_empty()
            && self.code.len() <= FAILURE_CODE_BYTES
            && self.message.len() <= FAILURE_MESSAGE_BYTES
            && !error_codes::ALL.contains(&self.code.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandledNodeFailure {
    pub sequence: u64,
    pub policy: FlowFailurePolicy,
    pub failure: FlowNodeFailure,
}

pub(crate) fn identity(run: &str, node: &str, sequence: u64) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(run, node, sequence)).expect("failure identity"))
    )
}

pub(crate) async fn handle_error<H: ExecutionHost>(
    host: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    operation: &str,
    error: H::Error,
) -> Result<(), H::Error> {
    if runtime.unhandled_node.is_some() {
        return Err(error);
    }
    let cancellation = runtime.activation_token.clone();
    host.check_cancelled(runtime, &cancellation)?;
    let failure = error.recoverable_node_failure();
    let eligible = !runtime.handling_failures.contains(&node.id)
        && error
            .reason_code()
            .is_some_and(|code| !error_codes::ALL.contains(&code))
        && failure
            .as_ref()
            .is_some_and(RecoverableNodeFailure::validate);
    if eligible && node.failure_policy != FlowFailurePolicy::Fail {
        handle_failure(host, runtime, node, definition, operation, failure.unwrap()).await
    } else {
        runtime.unhandled_node = Some(node.id.clone());
        host.publish_execution_record(runtime, &node.id, EventKind::NodeError,
            error.message().into(), Some(json!({"policy":node.failure_policy,"handled":false,
                "operation":operation,"error":{"code":error.reason_code().unwrap_or(error.code()),"message":error.message()}}))).await;
        Err(error)
    }
}

pub(crate) async fn handle_failure<H: ExecutionHost>(
    host: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    operation: &str,
    failure: RecoverableNodeFailure,
) -> Result<(), H::Error> {
    let token = runtime.activation_token.clone();
    host.check_cancelled(runtime, &token)?;
    if !failure.validate()
        || runtime.handling_failures.contains(&node.id)
        || runtime.unhandled_node.is_some()
    {
        return Err(H::Error::invalid_request(
            error_codes::INVALID_NODE_FAILURE,
            "Invalid or recursively handled node failure.",
        ));
    }
    if node.failure_policy == FlowFailurePolicy::Fail {
        runtime.unhandled_node = Some(node.id.clone());
        host.publish_execution_record(
            runtime,
            &node.id,
            EventKind::NodeError,
            failure.message.clone(),
            Some(
                json!({"policy":node.failure_policy,"handled":false,"operation":operation,
                "error":{"code":failure.code,"message":failure.message}}),
            ),
        )
        .await;
        return Err(H::Error::internal(failure.message));
    }
    if runtime.stopped_nodes.contains(&node.id) {
        return Ok(());
    }
    let phase = runtime
        .node_phases
        .get(&node.id)
        .copied()
        .unwrap_or(FlowPortPhase::Execution);
    let record = HandledNodeFailure {
        sequence: runtime.delivery_sequence,
        policy: node.failure_policy,
        failure: FlowNodeFailure {
            id: identity(&runtime.run_id, &node.id, runtime.delivery_sequence),
            node_id: node.id.clone(),
            phase,
            operation: operation.into(),
            code: failure.code,
            message: failure.message,
        },
    };
    host.publish_execution_record(
        runtime,
        &node.id,
        EventKind::NodeError,
        record.failure.message.clone(),
        Some(json!({"policy":record.policy,"handled":false,
            "operation":operation,"phase":phase,"failureId":record.failure.id,
            "error":{"code":record.failure.code,"message":record.failure.message}})),
    )
    .await;
    let retained = runtime
        .retention
        .retain(&node.id, record.clone())
        .map_err(|message| H::Error::invalid_request(error_codes::CAPACITY_EXHAUSTED, message))?;
    runtime.handled_failures.insert(node.id.clone(), retained);
    runtime.handling_failures.insert(node.id.clone());
    let result = async {
        stop_node::<H>(runtime, &node.id);
        if record.policy == FlowFailurePolicy::Emit {
            let port = definition
                .failure_ports
                .iter()
                .find(|p| p.phase == phase)
                .ok_or_else(|| {
                    H::Error::invalid_request(
                        error_codes::INVALID_EXECUTION_CONTRACT,
                        "Missing failure port.",
                    )
                })?;
            let previous_emission = runtime
                .failure_emission
                .replace((node.id.clone(), port.name.clone()));
            let emitted = host
                .emit_flow_token(
                    runtime,
                    &node.id,
                    &port.name,
                    Token {
                        token_type: NODE_FAILURE_TOKEN_TYPE.into(),
                        value: serde_json::to_value(&record.failure).expect("failure payload"),
                    },
                    &token,
                )
                .await;
            runtime.failure_emission = previous_emission;
            emitted?;
        }
        settle_outputs(host, runtime, node).await?;
        host.publish_execution_record(
            runtime,
            &node.id,
            EventKind::NodeFailureHandled,
            record.failure.message.clone(),
            Some(json!({"policy":record.policy,"handled":true,
                "operation":operation,"phase":phase,"failureId":record.failure.id,
                "error":{"code":record.failure.code,"message":record.failure.message}})),
        )
        .await;
        Ok(())
    }
    .await;
    runtime.handling_failures.remove(&node.id);
    if result.is_err() {
        runtime.unhandled_node = Some(node.id.clone());
    }
    result
}

fn stop_node<H: ExecutionHost>(runtime: &mut H::State, node: &str) {
    runtime.stopped_nodes.insert(node.into());
    runtime.executed_nodes.insert(node.into());
    runtime.materialized_sources.insert(node.into());
    runtime.logic_nodes_evaluated.insert(node.into());
    runtime.producers.remove(node);
    runtime.ready_inline_nodes.retain(|id| id != node);
    runtime.input_tokens.retain(|(id, _), _| id != node);
    runtime
        .data_queue
        .retain(|delivery| delivery.edge.target_node_id != node);
    runtime
        .control_queue
        .retain(|delivery| match &delivery.control {
            crate::delivery::Control::Controller(id) => id != node,
            crate::delivery::Control::Signal(edge) => edge.target_node_id != node,
            crate::delivery::Control::Close { .. } => true,
        });
}

async fn settle_outputs<H: ExecutionHost>(
    host: &H,
    runtime: &mut H::State,
    node: &FlowNode,
) -> Result<(), H::Error> {
    for port in battersea_flow::ports::expanded_output_ports_for_node(&runtime.definitions, node)
        .map_err(H::Error::internal)?
    {
        let key = (node.id.clone(), port.name.clone());
        if port.mode == FlowPortMode::Stream && !runtime.closed_outputs.contains(&key) {
            runtime.failed_outputs.insert(key);
        }
        host.close_flow_output(runtime, &node.id, &port.name)
            .await?;
    }
    host.mark_node_signal_ports_settled_except(runtime, node, &[])?;
    Ok(())
}

pub(crate) async fn skip_dependency<H: ExecutionHost>(
    host: &H,
    runtime: &mut H::State,
    node: &FlowNode,
) -> Result<(), H::Error> {
    if runtime.stopped_nodes.contains(&node.id) {
        return Ok(());
    }
    stop_node::<H>(runtime, &node.id);
    settle_outputs(host, runtime, node).await?;
    host.publish_execution_record(
        runtime,
        &node.id,
        EventKind::NodeSkipped,
        "Required input did not complete successfully.".into(),
        Some(json!({"reason":"failed_dependency"})),
    )
    .await;
    Ok(())
}
