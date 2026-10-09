use crate::{ExecutionError, ExecutionHost, Token};
use async_trait::async_trait;
use battersea_flow::{FlowNode, FlowNodeDefinition};
use serde_json::Value;
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

// async_trait adds must_use to boxed futures; dispatch arguments retain the port contract.
#[allow(clippy::double_must_use, clippy::too_many_arguments)]
#[async_trait]
pub trait NodeHandler<H: ExecutionHost>: Send + Sync {
    /// Returns the stable registry id used to resolve this handler from flow
    /// node definitions at runtime.
    fn handler_id(&self) -> &'static str;

    /// Materialises one source-style node without requiring queued inputs.
    ///
    /// The default implementation rejects materialisation for handlers that do
    /// not expose source-like behaviour.
    async fn materialize_node(
        &self,
        _core: &H,
        _runtime: &mut H::State,
        _node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _token: &CancellationToken,
    ) -> Result<(), H::Error> {
        Err(H::Error::invalid_request(format!(
            "Flow handler \"{}\" does not support source materialisation.",
            self.handler_id()
        )))
    }

    /// Executes a directly activatable node.
    ///
    /// The default implementation rejects direct activation with
    /// `invalid_request` for handlers that only support sink or action entry
    /// points.
    async fn execute_node(
        &self,
        _core: &H,
        _runtime: &mut H::State,
        _node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _activation_values: Option<&HashMap<String, Value>>,
        _token: &CancellationToken,
    ) -> Result<(), H::Error> {
        Err(H::Error::invalid_request(format!(
            "Flow handler \"{}\" does not support direct node execution.",
            self.handler_id()
        )))
    }

    /// Executes one sink input on a sink-capable node.
    ///
    /// The default implementation rejects sink execution with
    /// `invalid_request` for handlers that do not expose sink ports.
    async fn execute_sink(
        &self,
        _core: &H,
        _runtime: &mut H::State,
        _node: &FlowNode,
        _input_port: &str,
        _token_value: Token,
    ) -> Result<(), H::Error> {
        Err(H::Error::invalid_request(format!(
            "Flow handler \"{}\" does not support sink execution.",
            self.handler_id()
        )))
    }

    /// Executes one action port on an action-capable node.
    ///
    /// The default implementation rejects action execution with
    /// `invalid_request` for handlers that do not expose action ports.
    async fn execute_action(
        &self,
        _core: &H,
        _runtime: &mut H::State,
        _node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _action_port: &str,
    ) -> Result<(), H::Error> {
        Err(H::Error::invalid_request(format!(
            "Flow handler \"{}\" does not support action execution.",
            self.handler_id()
        )))
    }

    /// Executes one UI-controller action on a controller-capable node.
    ///
    /// Controller actions are not graph ports; bound UI widgets invoke them
    /// through the session runtime controller path.
    async fn execute_controller_action(
        &self,
        _core: &H,
        _runtime: &mut H::State,
        _node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _controller_action: &str,
    ) -> Result<(), H::Error> {
        Err(H::Error::invalid_request(format!(
            "Flow handler \"{}\" does not support controller actions.",
            self.handler_id()
        )))
    }

    /// Observes one received input token before the runtime queues or consumes it.
    ///
    /// Handlers can use this to emit incremental outputs while still allowing
    /// the final queued execution pass to consume the full buffered inputs.
    async fn receive_input_token(
        &self,
        _core: &H,
        _runtime: &mut H::State,
        _node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _input_port: &str,
        _token_value: &Token,
        _token: &CancellationToken,
    ) -> Result<(), H::Error> {
        Ok(())
    }
}
