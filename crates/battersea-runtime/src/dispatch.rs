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
    core.handlers()
        .get(&definition.handler_id)
        .map_err(H::Error::invalid_request)?
        .execute_node(core, runtime, node, definition, activation_values, token)
        .await
}

/// Resolves the registered handler for `definition` and forwards source-style
/// materialisation to it unchanged.
pub async fn materialize_registered_flow_node<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    token: &CancellationToken,
) -> Result<(), H::Error> {
    core.handlers()
        .get(&definition.handler_id)
        .map_err(H::Error::invalid_request)?
        .materialize_node(core, runtime, node, definition, token)
        .await
}

/// Resolves the registered handler for `definition` and forwards sink
/// execution to it unchanged.
pub async fn execute_registered_sink<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    input_port: &str,
    token_value: Retained<Token>,
) -> Result<(), H::Error> {
    core.handlers()
        .get(&definition.handler_id)
        .map_err(H::Error::invalid_request)?
        .execute_sink(core, runtime, node, input_port, token_value)
        .await
}

/// Resolves the registered handler for `definition` and forwards one received
/// input token observation unchanged.
pub async fn receive_registered_input_token<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    input_port: &str,
    token_value: &Token,
    token: &CancellationToken,
) -> Result<(), H::Error> {
    core.handlers()
        .get(&definition.handler_id)
        .map_err(H::Error::invalid_request)?
        .receive_input_token(
            core,
            runtime,
            node,
            definition,
            input_port,
            token_value,
            token,
        )
        .await
}

/// Resolves the registered handler for `definition` and forwards action
/// execution to it unchanged.
pub async fn execute_registered_action<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    action_port: &str,
) -> Result<(), H::Error> {
    core.handlers()
        .get(&definition.handler_id)
        .map_err(H::Error::invalid_request)?
        .execute_action(core, runtime, node, definition, action_port)
        .await
}

/// Resolves the registered handler for `definition` and forwards UI-controller
/// action execution to it unchanged.
pub async fn execute_registered_controller_action<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    controller_action: &str,
) -> Result<(), H::Error> {
    core.handlers()
        .get(&definition.handler_id)
        .map_err(H::Error::invalid_request)?
        .execute_controller_action(core, runtime, node, definition, controller_action)
        .await
}
