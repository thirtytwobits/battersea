//! Copyright (c) Scott A Dixon
//!
//! Runtime handlers for activation-local signal logic nodes.

use async_trait::async_trait;
use battersea_flow::{FlowNode, FlowNodeDefinition};

use crate::{NodeError as EngineError, NodeResult as EngineResult};
use battersea_runtime::{
    error_codes, ExecutionError, ExecutionHost, NodeHandler as FlowRuntimeHandler,
};

pub struct LogicMultiplexerHandler;
pub struct LogicDemultiplexerHandler;
pub struct AndGateHandler;
pub struct OrGateHandler;
pub struct InvertGateHandler;

fn record_logic_action<H: ExecutionHost>(
    core: &H,
    runtime: &mut H::State,
    node: &FlowNode,
    action_port: &str,
) -> Result<(), H::Error> {
    let definition = battersea_flow::catalog::node_definition(&runtime.definitions, node)
        .ok_or_else(|| H::Error::internal("Flow runtime lost a logic definition."))?;
    let action_ports = battersea_flow::ports::expanded_action_ports_for_node(
        &runtime.definitions,
        node,
    )
    .map_err(|text| {
        H::Error::internal(
            battersea_runtime::Failure::from("Node ports cannot be expanded.").caused_by_text(text),
        )
    })?;
    if !action_ports.iter().any(|port| port.name == action_port) {
        return Err(H::Error::invalid_request(
            error_codes::UNSUPPORTED_ACTION_PORT,
            battersea_runtime::Failure::from("The definition does not support the action port.")
                .with_context("definition", definition.class_name.as_str())
                .with_context("port", action_port),
        ));
    }
    core.record_logic_action_signal(runtime, node, action_port);
    Ok(())
}

macro_rules! impl_logic_handler {
    ($handler:ident, $id:literal) => {
        #[async_trait]
        impl<H: ExecutionHost> FlowRuntimeHandler<H> for $handler {
            fn handler_id(&self) -> &'static str {
                $id
            }

            async fn execute_action(
                &self,
                core: &H,
                runtime: &mut H::State,
                node: &FlowNode,
                _definition: &FlowNodeDefinition,
                action_port: &str,
            ) -> Result<(), H::Error> {
                record_logic_action(core, runtime, node, action_port)
            }
        }
    };
}

impl_logic_handler!(LogicMultiplexerHandler, "battersea.logic.multiplexer");
impl_logic_handler!(LogicDemultiplexerHandler, "battersea.logic.demultiplexer");
impl_logic_handler!(AndGateHandler, "battersea.logic.and");
impl_logic_handler!(OrGateHandler, "battersea.logic.or");
impl_logic_handler!(InvertGateHandler, "battersea.logic.invert");

/// Computes signal outputs without publishing or recording an activation.
pub fn prepare_logic_outputs(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    action_ports: &[battersea_flow::FlowActionPortDefinition],
    signal_ports: &[battersea_flow::FlowSignalPortDefinition],
    fired_ports: &std::collections::HashSet<String>,
    enabled: bool,
) -> EngineResult<Vec<String>> {
    Ok(match definition.handler_id.as_str() {
        "battersea.logic.multiplexer" => {
            if enabled && fired_ports.contains("input") {
                signal_ports.iter().map(|port| port.name.clone()).collect()
            } else {
                Vec::new()
            }
        }
        "battersea.logic.demultiplexer" => {
            let data_fired = fired_ports.iter().any(|port| port.starts_with("input-"));
            if enabled && data_fired {
                vec!["output".to_string()]
            } else {
                Vec::new()
            }
        }
        "battersea.logic.and" => {
            let input_ports = action_ports
                .iter()
                .filter(|port| port.name.starts_with("input-"))
                .map(|port| port.name.clone())
                .collect::<Vec<_>>();
            let all_true = !input_ports.is_empty()
                && input_ports.iter().all(|port| fired_ports.contains(port));
            let inverted =
                battersea_flow::ports::effective_parameter_value(node, definition, "not")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
            if enabled && (if inverted { !all_true } else { all_true }) {
                vec!["output".to_string()]
            } else {
                Vec::new()
            }
        }
        "battersea.logic.or" => {
            let input_ports = action_ports
                .iter()
                .filter(|port| port.name.starts_with("input-"))
                .map(|port| port.name.clone())
                .collect::<Vec<_>>();
            let true_count = input_ports
                .iter()
                .filter(|port| fired_ports.contains(*port))
                .count();
            let exclusive =
                battersea_flow::ports::effective_parameter_value(node, definition, "exclusive")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
            let inverted =
                battersea_flow::ports::effective_parameter_value(node, definition, "not")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
            let base = if exclusive {
                true_count == 1
            } else {
                true_count > 0
            };
            if enabled && (if inverted { !base } else { base }) {
                vec!["output".to_string()]
            } else {
                Vec::new()
            }
        }
        "battersea.logic.invert" => {
            if enabled && !fired_ports.contains("input") {
                vec!["output".to_string()]
            } else {
                Vec::new()
            }
        }
        _ => {
            return Err(EngineError::invalid_request(
                error_codes::UNSUPPORTED_HANDLER_OPERATION,
                battersea_runtime::Failure::from("Flow handler does not support logic evaluation.")
                    .with_context("handler", definition.handler_id.as_str()),
            ));
        }
    })
}
