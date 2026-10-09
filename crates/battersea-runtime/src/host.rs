//! Copyright (c) Scott A Dixon
//! Extracted from Primrose Hill flow_api.rs at 20ccd7134.
use crate::dispatch::*;
use crate::*;
use async_trait::async_trait;
use battersea_flow::catalog::{node_definition, FLOW_ACTION_DISABLE, FLOW_NODE_ACTIVATE_INTERFACE};
use battersea_flow::ports::*;
use battersea_flow::*;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tokio_util::sync::CancellationToken;

// async_trait adds must_use to boxed futures; dispatch arguments retain the port contract.
#[allow(clippy::double_must_use, clippy::too_many_arguments)]
#[async_trait]
pub trait ExecutionHost: Sized + Send + Sync {
    type State: std::ops::Deref<Target = SchedulerState> + std::ops::DerefMut + Send + Sync;
    type Error: ExecutionError;
    fn handlers(&self) -> &HandlerRegistry<Self>;
    async fn execution_event(&self, runtime: &Self::State, event: ExecutionEvent);
    async fn publish_execution_record(
        &self,
        runtime: &Self::State,
        node_id: &str,
        kind: EventKind,
        summary: String,
        detail: Option<Value>,
    ) {
        let event = ExecutionEvent {
            run_id: runtime.run_id.clone(),
            flow_key: runtime.flow.flow_key.clone(),
            sequence: runtime
                .event_sequence
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            node_id: node_id.into(),
            kind,
            summary,
            detail,
        };
        self.execution_event(runtime, event).await;
    }
    fn observe_token(&self, _runtime: &Self::State, _node: &str, _port: &str, _token: &Token) {}
    async fn resolve_parameter_write(
        &self,
        runtime: &Self::State,
        node: &FlowNode,
        parameter: &FlowParameterDefinition,
        value: &Value,
    ) -> Result<Value, Self::Error>;
    async fn commit_parameter_write(
        &self,
        runtime: &mut Self::State,
        node: &FlowNode,
        parameter: &FlowParameterDefinition,
        value: &Value,
    ) -> Result<(), Self::Error>;

    async fn apply_automation_write(
        &self,
        runtime: &mut Self::State,
        edge: &FlowEdge,
        source_node: &str,
        source_port: &str,
        target: &FlowNode,
        port: &FlowAutomationPortDefinition,
        token: Token,
    ) -> Result<(), Self::Error> {
        let Some(parameter) = node_definition(&runtime.definitions, target)
            .and_then(|d| d.parameters.iter().find(|p| p.name == port.parameter_name))
            .cloned()
        else {
            return Ok(());
        };
        let result = self
            .resolve_parameter_write(runtime, target, &parameter, &token.value)
            .await;
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                self.publish_execution_record(
                    runtime,
                    &target.id,
                    EventKind::AutomationRejected,
                    error.message().into(),
                    Some(json!({"parameterName": parameter.name})),
                )
                .await;
                return Ok(());
            }
        };
        let previous = runtime.nodes_by_id.get(&target.id).and_then(|n| {
            node_definition(&runtime.definitions, n).and_then(|d| {
                battersea_flow::ports::effective_parameter_value(n, d, &parameter.name)
            })
        });
        if previous.as_ref() == Some(&value) {
            return Ok(());
        }
        if let Err(error) = self
            .commit_parameter_write(runtime, target, &parameter, &value)
            .await
        {
            self.publish_execution_record(
                runtime,
                &target.id,
                EventKind::AutomationRejected,
                error.message().into(),
                None,
            )
            .await;
            return Err(error);
        }
        if let Some(node) = runtime.nodes_by_id.get_mut(&target.id) {
            node.parameter_values
                .insert(parameter.name.clone(), value.clone());
        }
        if let Some(node) = runtime.flow.nodes.iter_mut().find(|n| n.id == target.id) {
            node.parameter_values
                .insert(parameter.name.clone(), value.clone());
        }
        self.publish_execution_record(runtime, &target.id, EventKind::AutomationWrite, format!("Automation write {} -> {}.{}.", token.token_type, target.id, parameter.name), Some(json!({"edgeId": edge.id, "sourceNodeId": source_node, "sourcePort": source_port, "targetPort": edge.target_port, "parameterName": parameter.name, "tokenType": token.token_type, "value": value}))).await;
        Ok(())
    }

    fn prepare_logic_outputs(
        &self,
        node: &FlowNode,
        definition: &FlowNodeDefinition,
        actions: &[FlowActionPortDefinition],
        signals: &[FlowSignalPortDefinition],
        fired: &HashSet<String>,
        enabled: bool,
    ) -> Result<Vec<String>, Self::Error>;
    fn logic_gate_label(&self, node: &FlowNode, definition: &FlowNodeDefinition) -> String;
    fn flow_token_excerpt(value: &Value) -> Option<String> {
        if let Some(text) = value.as_str() {
            let excerpt = text.trim().chars().take(72).collect::<String>();
            return (!excerpt.is_empty()).then_some(excerpt);
        }

        let fragments = value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|fragment| !fragment.is_empty())
            .take(3)
            .collect::<Vec<_>>();
        if fragments.is_empty() {
            None
        } else {
            Some(fragments.join(" | ").chars().take(72).collect())
        }
    }

    fn definition_is_output_capable(definition: &FlowNodeDefinition) -> bool {
        !definition.output_ports.is_empty() || !definition.dynamic_output_ports.is_empty()
    }

    fn node_is_disabled_output_capable(
        &self,
        runtime: &Self::State,
        node: &FlowNode,
        definition: &FlowNodeDefinition,
    ) -> bool {
        Self::definition_is_output_capable(definition)
            && runtime.disabled_output_nodes.contains(&node.id)
    }

    fn discard_queued_input_tokens_for_node(&self, runtime: &mut Self::State, node_id: &str) {
        runtime
            .input_tokens
            .retain(|(queued_node_id, _), _| queued_node_id != node_id);
        runtime
            .ready_inline_nodes
            .retain(|queued_node_id| queued_node_id != node_id);
    }

    fn edge_source_is_disabled(
        &self,
        runtime: &Self::State,
        edge: &battersea_flow::FlowEdge,
    ) -> bool {
        runtime.disabled_output_nodes.contains(&edge.source_node_id)
    }

    fn enqueue_ready_inline_targets_after_source_disconnect(
        &self,
        runtime: &mut Self::State,
        source_node_id: &str,
    ) {
        let target_node_ids = runtime
            .outgoing_edges
            .iter()
            .filter(|((edge_source_node_id, _), _)| edge_source_node_id == source_node_id)
            .map(|(_, edge)| edge.target_node_id.clone())
            .collect::<HashSet<_>>();

        for target_node_id in target_node_ids {
            if runtime.executed_nodes.contains(&target_node_id)
                || runtime
                    .ready_inline_nodes
                    .iter()
                    .any(|queued_node_id| queued_node_id == &target_node_id)
            {
                continue;
            }

            let Some(target_node) = runtime.nodes_by_id.get(&target_node_id) else {
                continue;
            };
            let Some(definition) = node_definition(&runtime.definitions, target_node) else {
                continue;
            };
            if matches!(
                definition.kind,
                battersea_flow::FlowNodeClass::Inline
                    | battersea_flow::FlowNodeClass::Hybrid
                    | battersea_flow::FlowNodeClass::Instrument
            ) && self.inline_node_ready(runtime, &target_node_id)
            {
                runtime.ready_inline_nodes.push_back(target_node_id);
            }
        }
    }

    async fn materialize_snapshot_sources(
        &self,
        runtime: &mut Self::State,
        activated_node_id: &str,
        token: &CancellationToken,
    ) -> Result<(), Self::Error> {
        let source_ids = runtime
            .flow
            .nodes
            .iter()
            .filter(|node| node.id != activated_node_id)
            .filter(|node| {
                node_definition(&runtime.definitions, node).is_some_and(|definition| {
                    matches!(
                        definition.kind,
                        battersea_flow::FlowNodeClass::Source
                            | battersea_flow::FlowNodeClass::Hybrid
                    )
                })
            })
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        let source_ids = order_source_phase_materialization(runtime, source_ids);

        for node_id in source_ids {
            let node = runtime
                .nodes_by_id
                .get(&node_id)
                .cloned()
                .ok_or_else(|| Self::Error::internal("Flow runtime lost a source node."))?;
            let definition = node_definition(&runtime.definitions, &node)
                .ok_or_else(|| Self::Error::internal("Flow runtime lost a source definition."))?
                .clone();
            if battersea_flow::catalog::node_implements_interface(
                &runtime.definitions,
                &node,
                FLOW_NODE_ACTIVATE_INTERFACE,
            ) {
                continue;
            }
            if runtime.materialized_sources.contains(&node_id) {
                continue;
            }
            if self.node_is_disabled_output_capable(runtime, &node, &definition) {
                runtime.materialized_sources.insert(node_id.clone());
                runtime.executed_nodes.insert(node_id.clone());
                self.discard_queued_input_tokens_for_node(runtime, &node_id);
                self.mark_node_signal_ports_settled_except(runtime, &node, &[])?;
                continue;
            }
            materialize_registered_flow_node(self, runtime, &node, &definition, token).await?;
        }
        Ok(())
    }

    async fn execute_flow_node(
        &self,
        runtime: &mut Self::State,
        node_id: &str,
        activation_values: Option<&HashMap<String, Value>>,
        token: &CancellationToken,
    ) -> Result<(), Self::Error> {
        let node = runtime.nodes_by_id.get(node_id).cloned().ok_or_else(|| {
            Self::Error::invalid_request(format!("Unknown flow node \"{node_id}\"."))
        })?;
        let definition = node_definition(&runtime.definitions, &node)
            .ok_or_else(|| {
                Self::Error::invalid_request(format!(
                    "Unknown flow node definition \"{}\".",
                    node.definition_name
                ))
            })?
            .clone();
        if self.node_is_disabled_output_capable(runtime, &node, &definition) {
            self.publish_execution_record(
                runtime,
                node_id,
                EventKind::NodeSkipped,
                format!("Skipped disabled flow node \"{node_id}\"."),
                Some(json!({
                    "definitionName": node.definition_name,
                    "handlerId": definition.handler_id,
                    "reason": "disabled",
                })),
            )
            .await;
            runtime.executed_nodes.insert(node.id.clone());
            if matches!(
                definition.kind,
                battersea_flow::FlowNodeClass::Source | battersea_flow::FlowNodeClass::Hybrid
            ) {
                runtime.materialized_sources.insert(node.id.clone());
            }
            self.discard_queued_input_tokens_for_node(runtime, &node.id);
            self.mark_node_signal_ports_settled_except(runtime, &node, &[])?;
            return Ok(());
        }
        self.publish_execution_record(
            runtime,
            node_id,
            EventKind::NodeStart,
            format!("Executing flow node \"{node_id}\"."),
            Some(json!({
                "definitionName": node.definition_name,
                "handlerId": definition.handler_id,
                "activationValueCount": activation_values.map_or(0, HashMap::len),
            })),
        )
        .await;

        let result = execute_registered_flow_node(
            self,
            runtime,
            &node,
            &definition,
            activation_values,
            token,
        )
        .await;

        match result {
            Ok(()) => {
                self.publish_execution_record(
                    runtime,
                    node_id,
                    EventKind::NodeComplete,
                    format!("Completed flow node \"{node_id}\"."),
                    Some(json!({
                        "definitionName": node.definition_name,
                        "handlerId": definition.handler_id,
                        "executedNodeCount": runtime.executed_nodes.len(),
                    })),
                )
                .await;
                self.emit_post_activate_signal(runtime, &node, &definition)
                    .await?;
                self.mark_node_signal_ports_settled_except(runtime, &node, &[])?;
                Ok(())
            }
            Err(error) => {
                self.publish_execution_record(
                    runtime,
                    node_id,
                    EventKind::NodeError,
                    format!("Flow node \"{node_id}\" failed."),
                    Some(json!({
                        "definitionName": node.definition_name,
                        "handlerId": definition.handler_id,
                        "error": {
                            "code": error.code(),
                            "message": error.message(),
                        },
                    })),
                )
                .await;
                Err(error)
            }
        }
    }

    fn connected_input_ports(&self, runtime: &Self::State, node_id: &str) -> Vec<String> {
        let mut edges = runtime
            .incoming_edges
            .values()
            .filter(|edge| edge.target_node_id == node_id)
            .filter(|edge| !self.edge_source_is_disabled(runtime, edge))
            .collect::<Vec<_>>();
        edges.sort_by_key(|edge| (edge.order, edge.target_port.clone(), edge.id.clone()));
        edges
            .into_iter()
            .map(|edge| edge.target_port.clone())
            .collect()
    }

    fn take_flow_input_token(
        &self,
        runtime: &mut Self::State,
        node_id: &str,
        port_name: &str,
    ) -> Result<Token, Self::Error> {
        let entry = runtime
            .input_tokens
            .get_mut(&(node_id.to_string(), port_name.to_string()))
            .and_then(|tokens| (!tokens.is_empty()).then_some(tokens))
            .ok_or_else(|| {
                Self::Error::internal(format!(
                    "Flow node \"{}\" does not have a token for input port \"{}\".",
                    node_id, port_name
                ))
            })?;
        let token = entry.remove(0);
        runtime.consumed_tokens += 1;
        Ok(token)
    }

    fn inline_node_ready(&self, runtime: &Self::State, node_id: &str) -> bool {
        self.connected_input_ports(runtime, node_id)
            .into_iter()
            .all(|port_name| {
                runtime
                    .input_tokens
                    .get(&(node_id.to_string(), port_name))
                    .is_some_and(|tokens| !tokens.is_empty())
            })
    }

    fn take_all_flow_input_tokens(
        &self,
        runtime: &mut Self::State,
        node_id: &str,
        port_name: &str,
    ) -> Result<Vec<Token>, Self::Error> {
        let entry = runtime
            .input_tokens
            .get_mut(&(node_id.to_string(), port_name.to_string()))
            .ok_or_else(|| {
                Self::Error::internal(format!(
                    "Flow node \"{}\" does not have queued tokens for input port \"{}\".",
                    node_id, port_name
                ))
            })?;
        if entry.is_empty() {
            return Err(Self::Error::internal(format!(
                "Flow node \"{}\" does not have a token for input port \"{}\".",
                node_id, port_name
            )));
        }
        let tokens = std::mem::take(entry);
        runtime.consumed_tokens += tokens.len() as u32;
        Ok(tokens)
    }

    async fn emit_flow_token(
        &self,
        runtime: &mut Self::State,
        source_node_id: &str,
        source_port: &str,
        token_value: Token,
        cancel_token: &CancellationToken,
    ) -> Result<(), Self::Error> {
        let source_node = runtime
            .nodes_by_id
            .get(source_node_id)
            .cloned()
            .ok_or_else(|| Self::Error::internal("Flow runtime lost a source node."))?;
        let source_port_definition =
            expanded_output_ports_for_node(&runtime.definitions, &source_node)
                .map_err(Self::Error::internal)?
                .into_iter()
                .find(|port| port.name == source_port)
                .ok_or_else(|| {
                    Self::Error::internal(format!(
                        "Flow runtime lost source port \"{}\" on node \"{}\".",
                        source_port, source_node_id
                    ))
                })?;
        // "auto" source ports adopt the type of whatever the handler emits — the
        // declared port type is a placeholder for downstream rendering. Skip the
        // exact-type check; the receiving target validates type compatibility on
        // its own side. "oneof" source ports list the legal emission types in
        // `accepted_token_types`; any of those is accepted at emit time (this
        // mirrors the equivalent receive-side check via `port_accepts_token_type`).
        if source_port_definition.token_type != "auto"
            && !port_accepts_token_type(&source_port_definition, &token_value.token_type)
        {
            let accepted = if source_port_definition.accepted_token_types.is_empty() {
                source_port_definition.token_type.clone()
            } else {
                source_port_definition.accepted_token_types.join(" | ")
            };
            return Err(Self::Error::invalid_request(format!(
                "Flow handler for node \"{}\" emitted token type \"{}\" from port \"{}\", expected one of \"{}\".",
                source_node_id,
                token_value.token_type,
                source_port,
                accepted,
            )));
        }

        runtime.emitted_tokens += 1;

        self.observe_token(runtime, source_node_id, source_port, &token_value);

        let edge = runtime
            .outgoing_edges
            .get(&(source_node_id.to_string(), source_port.to_string()))
            .cloned();

        let summary = Self::flow_token_excerpt(&token_value.value)
            .map(|excerpt| {
                format!(
                    "Emitted {} on {}: {}",
                    token_value.token_type, source_port, excerpt
                )
            })
            .unwrap_or_else(|| format!("Emitted {} on {}.", token_value.token_type, source_port));

        self.publish_execution_record(
            runtime,
            source_node_id,
            EventKind::TokenEmit,
            summary,
            Some(json!({
                "sourcePort": source_port,
                "tokenType": token_value.token_type,
                "targetCount": usize::from(edge.is_some()),
                "targets": edge
                    .as_ref()
                    .map(|edge| vec![json!({
                        "edgeId": edge.id,
                        "targetNodeId": edge.target_node_id,
                        "targetPort": edge.target_port,
                    })])
                    .unwrap_or_default(),
                "value": token_value.value.clone(),
            })),
        )
        .await;

        if let Some(edge) = edge {
            let target_node = runtime
                .nodes_by_id
                .get(&edge.target_node_id)
                .cloned()
                .ok_or_else(|| Self::Error::internal("Flow runtime lost a target node."))?;

            // Automation port? Look it up first; the host node has a single
            // namespace per side, so input + automation port names cannot
            // collide (catalog validation enforces this).
            let automation_port =
                expanded_automation_ports_for_node(&runtime.definitions, &target_node)
                    .map_err(Self::Error::internal)?
                    .into_iter()
                    .find(|port| port.name == edge.target_port);
            if let Some(port) = automation_port {
                if !automation_port_accepts_token_type(&port, &token_value.token_type) {
                    return Err(Self::Error::invalid_request(format!(
                        "Flow token type mismatch on edge \"{}\": emitted \"{}\" but automation port \"{}\" expects \"{}\".",
                        edge.id,
                        token_value.token_type,
                        edge.target_port,
                        port.token_type,
                    )));
                }
                self.apply_automation_write(
                    runtime,
                    &edge,
                    source_node_id,
                    source_port,
                    &target_node,
                    &port,
                    token_value.clone(),
                )
                .await?;
                if cancel_token.is_cancelled() {
                    return Err(Self::Error::cancelled("Streaming request aborted."));
                }
                return Ok(());
            }

            let target_port_definition =
                expanded_input_ports_for_node(&runtime.definitions, &target_node)
                    .map_err(Self::Error::internal)?
                    .into_iter()
                    .find(|port| port.name == edge.target_port)
                    .ok_or_else(|| {
                        Self::Error::internal(format!(
                            "Flow runtime lost target port \"{}\" on node \"{}\".",
                            edge.target_port, edge.target_node_id
                        ))
                    })?;
            if !port_accepts_token_type(&target_port_definition, &token_value.token_type) {
                return Err(Self::Error::invalid_request(format!(
                    "Flow token type mismatch on edge \"{}\": emitted \"{}\" but target port \"{}\" expects \"{}\".",
                    edge.id,
                    token_value.token_type,
                    edge.target_port,
                    target_port_definition.token_type,
                )));
            }
            let definition = node_definition(&runtime.definitions, &target_node)
                .ok_or_else(|| Self::Error::internal("Flow runtime lost a target definition."))?
                .clone();
            if self.node_is_disabled_output_capable(runtime, &target_node, &definition) {
                self.publish_execution_record(
                    runtime,
                    &edge.target_node_id,
                    EventKind::TokenSkip,
                    format!(
                        "Skipped {} on {} because node \"{}\" is disabled.",
                        token_value.token_type, edge.target_port, edge.target_node_id
                    ),
                    Some(json!({
                        "edgeId": edge.id,
                        "sourceNodeId": source_node_id,
                        "sourcePort": source_port,
                        "targetPort": edge.target_port,
                        "tokenType": token_value.token_type,
                        "reason": "disabled",
                    })),
                )
                .await;
                return Ok(());
            }
            let received_summary = Self::flow_token_excerpt(&token_value.value)
                .map(|excerpt| {
                    format!(
                        "Received {} on {}: {}",
                        token_value.token_type, edge.target_port, excerpt
                    )
                })
                .unwrap_or_else(|| {
                    format!(
                        "Received {} on {}.",
                        token_value.token_type, edge.target_port
                    )
                });
            self.publish_execution_record(
                runtime,
                &edge.target_node_id,
                EventKind::TokenReceive,
                received_summary,
                Some(json!({
                    "edgeId": edge.id,
                    "sourceNodeId": source_node_id,
                    "sourcePort": source_port,
                    "targetPort": edge.target_port,
                    "tokenType": token_value.token_type,
                    "value": token_value.value.clone(),
                })),
            )
            .await;
            match definition.kind {
                battersea_flow::FlowNodeClass::Inline
                | battersea_flow::FlowNodeClass::Hybrid
                | battersea_flow::FlowNodeClass::Instrument => {
                    receive_registered_input_token(
                        self,
                        runtime,
                        &target_node,
                        &definition,
                        &edge.target_port,
                        &token_value,
                        cancel_token,
                    )
                    .await?;
                    runtime
                        .input_tokens
                        .entry((edge.target_node_id.clone(), edge.target_port.clone()))
                        .or_default()
                        .push(token_value.clone());
                    if self.inline_node_ready(runtime, &edge.target_node_id)
                        && !runtime.executed_nodes.contains(&edge.target_node_id)
                    {
                        runtime
                            .ready_inline_nodes
                            .push_back(edge.target_node_id.clone());
                    }
                }
                battersea_flow::FlowNodeClass::Sink => {
                    runtime.consumed_tokens += 1;
                    self.execute_sink_port(
                        runtime,
                        &target_node,
                        &edge.target_port,
                        token_value.clone(),
                    )
                    .await?;
                }
                battersea_flow::FlowNodeClass::Source => {
                    return Err(Self::Error::invalid_request(format!(
                        "Flow edge \"{}\" targets a source node, which is not allowed.",
                        edge.id
                    )));
                }
                battersea_flow::FlowNodeClass::Control => {
                    return Err(Self::Error::invalid_request(format!(
                        "Flow edge \"{}\" targets a control node, which is not allowed.",
                        edge.id
                    )));
                }
                battersea_flow::FlowNodeClass::Logic => {
                    return Err(Self::Error::invalid_request(format!(
                        "Flow edge \"{}\" targets a logic node with a token edge, which is not allowed.",
                        edge.id
                    )));
                }
            }
            if cancel_token.is_cancelled() {
                return Err(Self::Error::cancelled("Streaming request aborted."));
            }
        }

        Ok(())
    }

    async fn execute_sink_port(
        &self,
        runtime: &mut Self::State,
        node: &FlowNode,
        input_port: &str,
        token_value: Token,
    ) -> Result<(), Self::Error> {
        let definition = node_definition(&runtime.definitions, node)
            .ok_or_else(|| Self::Error::internal("Flow runtime lost a sink definition."))?
            .clone();
        execute_registered_sink(self, runtime, node, &definition, input_port, token_value).await?;
        self.emit_post_activate_signal(runtime, node, &definition)
            .await?;
        self.mark_node_signal_ports_settled_except(runtime, node, &[])?;
        Ok(())
    }

    async fn execute_flow_action(
        &self,
        runtime: &mut Self::State,
        node: &FlowNode,
        action_port: &str,
    ) -> Result<(), Self::Error> {
        let definition = node_definition(&runtime.definitions, node)
            .ok_or_else(|| Self::Error::internal("Flow runtime lost an action definition."))?
            .clone();
        self.publish_execution_record(
            runtime,
            &node.id,
            EventKind::ActionInvoke,
            format!(
                "Invoking action \"{}\" on node \"{}\".",
                action_port, node.id
            ),
            Some(json!({
                "actionPort": action_port,
                "definitionName": node.definition_name,
                "handlerId": definition.handler_id,
            })),
        )
        .await;

        let result = if action_port == FLOW_ACTION_DISABLE {
            if !Self::definition_is_output_capable(&definition) {
                Err(Self::Error::invalid_request(format!(
                    "Flow node \"{}\" does not support action port \"{}\".",
                    node.id, FLOW_ACTION_DISABLE
                )))
            } else {
                runtime.disabled_output_nodes.insert(node.id.clone());
                self.enqueue_ready_inline_targets_after_source_disconnect(runtime, &node.id);
                Ok(())
            }
        } else {
            execute_registered_action(self, runtime, node, &definition, action_port).await
        };
        if result.is_ok() && !matches!(definition.kind, battersea_flow::FlowNodeClass::Logic) {
            self.mark_node_signal_ports_settled_except(runtime, node, &[])?;
        }
        result
    }

    async fn drain_flow_work(
        &self,
        runtime: &mut Self::State,
        token: &CancellationToken,
    ) -> Result<(), Self::Error> {
        loop {
            let mut progressed = false;
            while let Some(ready_node_id) = runtime.ready_inline_nodes.pop_front() {
                if runtime.executed_nodes.contains(&ready_node_id) {
                    continue;
                }
                self.execute_flow_node(runtime, &ready_node_id, None, token)
                    .await?;
                progressed = true;
            }

            if self.evaluate_ready_logic_nodes(runtime).await? {
                progressed = true;
            }

            if !progressed {
                break;
            }
        }
        Ok(())
    }

    fn record_logic_action_signal(
        &self,
        runtime: &mut Self::State,
        node: &FlowNode,
        action_port: &str,
    ) {
        runtime
            .signal_action_latches
            .insert((node.id.clone(), action_port.to_string()));
    }

    fn mark_node_signal_ports_settled_except(
        &self,
        runtime: &mut Self::State,
        node: &FlowNode,
        emitted_ports: &[String],
    ) -> Result<(), Self::Error> {
        let emitted = emitted_ports.iter().collect::<HashSet<_>>();
        let signal_ports = expanded_signal_ports_for_node(&runtime.definitions, node)
            .map_err(Self::Error::internal)?;
        for port in signal_ports {
            if emitted.contains(&port.name) {
                continue;
            }
            runtime
                .signal_sources_settled
                .insert((node.id.clone(), port.name));
        }
        Ok(())
    }

    async fn evaluate_ready_logic_nodes(
        &self,
        runtime: &mut Self::State,
    ) -> Result<bool, Self::Error> {
        let logic_nodes = runtime
            .flow
            .nodes
            .iter()
            .filter(|node| {
                node_definition(&runtime.definitions, node).is_some_and(|definition| {
                    matches!(definition.kind, battersea_flow::FlowNodeClass::Logic)
                })
            })
            .cloned()
            .collect::<Vec<_>>();

        let mut progressed = false;
        for node in logic_nodes {
            if runtime.logic_nodes_evaluated.contains(&node.id) {
                continue;
            }
            if !self.logic_node_ready_to_evaluate(runtime, &node)? {
                continue;
            }
            self.evaluate_logic_node(runtime, &node).await?;
            progressed = true;
        }
        Ok(progressed)
    }

    fn logic_node_ready_to_evaluate(
        &self,
        runtime: &Self::State,
        node: &FlowNode,
    ) -> Result<bool, Self::Error> {
        let action_ports = expanded_action_ports_for_node(&runtime.definitions, node)
            .map_err(Self::Error::internal)?;
        let mut has_participating_input = false;
        for action_port in action_ports {
            let incoming_edge = runtime
                .incoming_signal_edges
                .get(&(node.id.clone(), action_port.name.clone()))
                .filter(|edge| !self.edge_source_is_disabled(runtime, edge));
            let Some(incoming_edge) = incoming_edge else {
                if action_port.name == "enable" {
                    continue;
                }
                continue;
            };
            let port_fired = runtime
                .signal_action_latches
                .contains(&(node.id.clone(), action_port.name.clone()));
            let port_settled = runtime.signal_sources_settled.contains(&(
                incoming_edge.source_node_id.clone(),
                incoming_edge.source_port.clone(),
            ));
            if port_fired || port_settled {
                has_participating_input = true;
            }
            if !port_settled {
                return Ok(false);
            }
        }
        Ok(has_participating_input)
    }

    async fn evaluate_logic_node(
        &self,
        runtime: &mut Self::State,
        node: &FlowNode,
    ) -> Result<(), Self::Error> {
        let definition = node_definition(&runtime.definitions, node)
            .ok_or_else(|| Self::Error::internal("Flow runtime lost a logic definition."))?
            .clone();
        let action_ports = expanded_action_ports_for_node(&runtime.definitions, node)
            .map_err(Self::Error::internal)?;
        let fired_ports = action_ports
            .iter()
            .filter(|port| {
                runtime
                    .signal_action_latches
                    .contains(&(node.id.clone(), port.name.clone()))
            })
            .map(|port| port.name.clone())
            .collect::<HashSet<_>>();
        let signal_ports = expanded_signal_ports_for_node(&runtime.definitions, node)
            .map_err(Self::Error::internal)?;
        let enabled = if runtime
            .incoming_signal_edges
            .get(&(node.id.clone(), "enable".to_string()))
            .is_some_and(|edge| !self.edge_source_is_disabled(runtime, edge))
        {
            fired_ports.contains("enable")
        } else {
            true
        };

        let output_ports = self.prepare_logic_outputs(
            node,
            &definition,
            &action_ports,
            &signal_ports,
            &fired_ports,
            enabled,
        )?;

        runtime.logic_nodes_evaluated.insert(node.id.clone());
        self.publish_execution_record(
            runtime,
            &node.id,
            EventKind::LogicEvaluate,
            format!(
                "Evaluated logic node \"{}\" as {}.",
                node.id,
                self.logic_gate_label(node, &definition)
            ),
            Some(json!({
                "definitionName": node.definition_name,
                "handlerId": definition.handler_id,
                "gate": self.logic_gate_label(node, &definition),
                "enabled": enabled,
                "receivedInputs": fired_ports.iter().cloned().collect::<Vec<_>>(),
                "emittedSignals": output_ports,
            })),
        )
        .await;

        let emitted = output_ports.clone();
        for output_port in output_ports {
            self.emit_flow_signal(runtime, node, &output_port).await?;
        }
        self.mark_node_signal_ports_settled_except(runtime, node, &emitted)?;
        Ok(())
    }

    fn request_activation_cancel(
        &self,
        runtime: &mut Self::State,
        reason: &str,
    ) -> Result<(), Self::Error> {
        runtime.cancellation_requested = true;
        runtime.activation_token.cancel();
        Err(Self::Error::cancelled(reason))
    }

    async fn emit_flow_signal(
        &self,
        runtime: &mut Self::State,
        node: &FlowNode,
        signal_port: &str,
    ) -> Result<(), Self::Error> {
        if !expanded_signal_ports_for_node(&runtime.definitions, node)
            .map_err(Self::Error::internal)?
            .iter()
            .any(|port| port.name == signal_port)
        {
            return Ok(());
        }
        runtime
            .signal_sources_settled
            .insert((node.id.clone(), signal_port.to_string()));

        let edge = runtime
            .outgoing_signal_edges
            .get(&(node.id.clone(), signal_port.to_string()))
            .cloned();

        self.publish_execution_record(
            runtime,
            &node.id,
            EventKind::SignalEmit,
            format!("Emitted signal {}.", signal_port),
            Some(json!({
                "signalPort": signal_port,
                "targetCount": usize::from(edge.is_some()),
                "targets": edge
                    .as_ref()
                    .map(|edge| vec![json!({
                        "edgeId": edge.id,
                        "targetNodeId": edge.target_node_id,
                        "targetPort": edge.target_port,
                    })])
                    .unwrap_or_default(),
            })),
        )
        .await;

        if let Some(edge) = edge {
            let target_node = runtime
                .nodes_by_id
                .get(&edge.target_node_id)
                .cloned()
                .ok_or_else(|| Self::Error::internal("Flow runtime lost an action target node."))?;
            self.publish_execution_record(
                runtime,
                &target_node.id,
                EventKind::SignalReceive,
                format!("Received signal {} on {}.", signal_port, edge.target_port),
                Some(json!({
                    "edgeId": edge.id,
                    "sourceNodeId": node.id,
                    "sourcePort": signal_port,
                    "targetPort": edge.target_port,
                })),
            )
            .await;
            runtime
                .signal_action_latches
                .insert((target_node.id.clone(), edge.target_port.clone()));
            let path_key = (target_node.id.clone(), edge.target_port.clone());
            if runtime.active_signal_path.contains(&path_key) {
                return Err(Self::Error::invalid_request(format!(
                    "Signal cycle detected: action \"{}\" on node \"{}\" is re-entered \
                     while still executing (edge \"{}\" from signal \"{}\" on node \"{}\").",
                    edge.target_port, target_node.id, edge.id, signal_port, node.id
                )));
            }
            runtime.active_signal_path.insert(path_key.clone());
            let action_result = self
                .execute_flow_action(runtime, &target_node, &edge.target_port)
                .await;
            runtime.active_signal_path.remove(&path_key);
            action_result?;
            if runtime.activation_token.is_cancelled() {
                return self.request_activation_cancel(
                    runtime,
                    "Flow activation cancelled by a signal-triggered action.",
                );
            }
        }

        Ok(())
    }

    async fn emit_post_activate_signal(
        &self,
        runtime: &mut Self::State,
        node: &FlowNode,
        _definition: &FlowNodeDefinition,
    ) -> Result<(), Self::Error> {
        self.emit_flow_signal(runtime, node, FLOW_SIGNAL_POST_ACTIVATE)
            .await
    }
}

pub fn order_source_phase_materialization<S: std::ops::Deref<Target = SchedulerState>>(
    runtime: &S,
    source_ids: Vec<String>,
) -> Vec<String> {
    let source_set: HashSet<String> = source_ids.iter().cloned().collect();
    let original_index: HashMap<String, usize> = source_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (id.clone(), index))
        .collect();

    let mut dependencies: HashMap<String, HashSet<String>> = HashMap::new();
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
    for node_id in &source_ids {
        let Some(node) = runtime.nodes_by_id.get(node_id) else {
            continue;
        };
        let Some(definition) = node_definition(&runtime.definitions, node) else {
            continue;
        };
        for edge in runtime
            .incoming_edges
            .values()
            .filter(|edge| edge.target_node_id == *node_id)
        {
            if edge.kind != FlowEdgeKind::Token {
                continue;
            }
            if !source_set.contains(&edge.source_node_id) {
                continue;
            }
            let Some(input_port) = definition
                .input_ports
                .iter()
                .find(|port| port.name == edge.target_port)
            else {
                continue;
            };
            if input_port.display_class != Some(battersea_flow::FlowPortDisplayClass::Source) {
                continue;
            }
            dependencies
                .entry(node_id.clone())
                .or_default()
                .insert(edge.source_node_id.clone());
            dependents
                .entry(edge.source_node_id.clone())
                .or_default()
                .push(node_id.clone());
        }
    }

    let mut ready: Vec<String> = source_ids
        .iter()
        .filter(|id| dependencies.get(*id).is_none_or(|deps| deps.is_empty()))
        .cloned()
        .collect();
    ready.sort_by_key(|id| original_index.get(id).copied().unwrap_or(usize::MAX));

    let mut result = Vec::with_capacity(source_ids.len());
    let mut emitted: HashSet<String> = HashSet::new();
    while let Some(node_id) = ready.first().cloned() {
        ready.remove(0);
        if !emitted.insert(node_id.clone()) {
            continue;
        }
        result.push(node_id.clone());
        if let Some(consumers) = dependents.get(&node_id).cloned() {
            for consumer in consumers {
                if let Some(deps) = dependencies.get_mut(&consumer) {
                    deps.remove(&node_id);
                    if deps.is_empty() && !emitted.contains(&consumer) {
                        let position = ready
                            .binary_search_by_key(
                                &original_index.get(&consumer).copied().unwrap_or(usize::MAX),
                                |id| original_index.get(id).copied().unwrap_or(usize::MAX),
                            )
                            .unwrap_or_else(|index| index);
                        ready.insert(position, consumer);
                    }
                }
            }
        }
    }
    // Append any nodes left over (cycle or unreachable) in original order so
    // we never drop nodes; the cycle is preserved as a deterministic fallback.
    for node_id in source_ids {
        if !emitted.contains(&node_id) {
            result.push(node_id);
        }
    }
    result
}
