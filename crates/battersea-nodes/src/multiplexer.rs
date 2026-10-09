//! Copyright (c) Scott A Dixon
//!
//! Flow runtime handler that duplicates one prompt-fragment input to every
//! configured prompt-fragment output.

use async_trait::async_trait;
use battersea_flow::ports::expanded_output_ports_for_node;
use battersea_flow::{FlowNode, FlowNodeDefinition};
use serde_json::Value;
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

use battersea_runtime::{ExecutionError, ExecutionHost, NodeHandler as FlowRuntimeHandler};

pub struct MultiplexerHandler;

#[async_trait]
impl<H: ExecutionHost> FlowRuntimeHandler<H> for MultiplexerHandler {
    fn handler_id(&self) -> &'static str {
        "battersea.multiplexer"
    }

    async fn execute_node(
        &self,
        core: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _activation_values: Option<&HashMap<String, Value>>,
        token: &CancellationToken,
    ) -> Result<(), H::Error> {
        let input_token = core.take_flow_input_token(runtime, &node.id, "input")?;
        let output_ports = expanded_output_ports_for_node(&runtime.definitions, node)
            .map_err(H::Error::internal)?;

        runtime.executed_nodes.insert(node.id.clone());
        for output_port in output_ports {
            core.emit_flow_token(
                runtime,
                &node.id,
                &output_port.name,
                input_token.clone(),
                token,
            )
            .await?;
        }

        Ok(())
    }
}
