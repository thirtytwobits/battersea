use crate::*;
use battersea_flow::{FlowNode, FlowNodeDefinition};
use serde_json::Value;
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
pub async fn execute_registered_flow_node<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    activation_values: Option<&HashMap<String, Value>>,
    token: &CancellationToken,
) -> Result<(), H::Error> {
    if runtime.stopped_nodes.contains(&node.id) {
        return Ok(());
    }
    let result = core
        .handlers()
        .resolve(&definition.handler_id)?
        .execute_node(core, runtime, node, definition, activation_values, token)
        .await;
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            crate::failure::handle_error(core, runtime, node, definition, "execute_node", error)
                .await
        }
    }
}

/// Materialises the registered source and applies its authored failure policy.
pub async fn materialize_registered_flow_node<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    token: &CancellationToken,
) -> Result<(), H::Error> {
    if runtime.stopped_nodes.contains(&node.id) {
        return Ok(());
    }
    let result = core
        .handlers()
        .resolve(&definition.handler_id)?
        .materialize_node(core, runtime, node, definition, token)
        .await;
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            crate::failure::handle_error(core, runtime, node, definition, "materialize", error)
                .await
        }
    }
}

/// Delivers to the registered sink and applies its authored failure policy.
pub async fn execute_registered_sink<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    input_port: &str,
    token_value: Retained<Token>,
) -> Result<(), H::Error> {
    if runtime.stopped_nodes.contains(&node.id) {
        return Ok(());
    }
    let result = core
        .handlers()
        .resolve(&definition.handler_id)?
        .execute_sink(core, runtime, node, input_port, token_value)
        .await;
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            crate::failure::handle_error(core, runtime, node, definition, "sink", error).await
        }
    }
}

/// Delivers a streaming input and applies the receiving node's failure policy.
pub async fn receive_registered_input_token<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    input_port: &str,
    token_value: &Token,
    token: &CancellationToken,
) -> Result<(), H::Error> {
    if runtime.stopped_nodes.contains(&node.id) {
        return Ok(());
    }
    let result = core
        .handlers()
        .resolve(&definition.handler_id)?
        .receive_input_token(
            core,
            runtime,
            node,
            definition,
            input_port,
            token_value,
            token,
        )
        .await;
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            crate::failure::handle_error(core, runtime, node, definition, "receive_input", error)
                .await
        }
    }
}

/// Invokes the registered action and applies its node's failure policy.
pub async fn execute_registered_action<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    action_port: &str,
) -> Result<(), H::Error> {
    if runtime.stopped_nodes.contains(&node.id) {
        return Ok(());
    }
    runtime
        .node_phases
        .insert(node.id.clone(), battersea_flow::FlowPortPhase::Execution);
    let result = core
        .handlers()
        .resolve(&definition.handler_id)?
        .execute_action(core, runtime, node, definition, action_port)
        .await;
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            crate::failure::handle_error(core, runtime, node, definition, "action", error).await
        }
    }
}

/// Invokes a controller action. Within an activation, applies the node's failure
/// policy; standalone calls return their handler result.
pub async fn execute_registered_controller_action<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    controller_action: &str,
) -> Result<(), H::Error> {
    if runtime.stopped_nodes.contains(&node.id) {
        return Ok(());
    }
    runtime
        .node_phases
        .insert(node.id.clone(), battersea_flow::FlowPortPhase::Execution);
    let result = core
        .handlers()
        .resolve(&definition.handler_id)?
        .execute_controller_action(core, runtime, node, definition, controller_action)
        .await;
    match result {
        Ok(()) => Ok(()),
        Err(error) if runtime.activated_node_id.is_none() => Err(error),
        Err(error) => {
            crate::failure::handle_error(
                core,
                runtime,
                node,
                definition,
                "controller_action",
                error,
            )
            .await
        }
    }
}
