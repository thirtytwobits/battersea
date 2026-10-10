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
    /// Durable hosts encode all application-owned node state and resource references.
    /// This is called only when a checkpoint journal is enabled by the activation host.
    fn checkpoint_host_state(
        &self,
        _runtime: &Self::State,
    ) -> Result<crate::recovery::VersionedState, Self::Error> {
        Err(Self::Error::invalid_request(
            "Host does not implement durable state checkpoints.",
        ))
    }
    fn controller_activation_effects(
        &self,
        runtime: &mut Self::State,
        node: &str,
        values: &HashMap<String, Value>,
    );
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
            attempt: runtime.event_attempt,
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
        token: &Token,
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
            .flat_map(|(_, edges)| edges.iter().map(|edge| edge.target_node_id.clone()))
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
        let source_ids = order_source_phase_materialization(runtime, source_ids)
            .map_err(Self::Error::invalid_request)?;

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
            self.check_cancelled(runtime, token)?;
            crate::durable::begin::<Self>(runtime, &node.id, "materialize")?;
            runtime
                .node_phases
                .insert(node.id.clone(), FlowPortPhase::Snapshot);
            materialize_registered_flow_node(self, runtime, &node, &definition, token).await?;
            if !runtime.producers.contains_key(&node.id) {
                self.finish_node_phase(runtime, &node, FlowPortPhase::Snapshot)
                    .await?;
            } else {
                self.await_snapshot_phase(runtime, &node.id, token).await?;
            }
            while self.progress_delivery(runtime, token).await? {}
            while self.evaluate_ready_logic_nodes(runtime).await? {
                while self.progress_delivery(runtime, token).await? {}
            }
            crate::durable::finish(self, runtime)?;
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
        crate::durable::begin::<Self>(runtime, node_id, "execute_node")?;
        let step: Result<(), Self::Error> = async {
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

            self.check_cancelled(runtime, token)?;
            runtime.started_nodes.insert(node.id.clone());
            let phase = if definition.kind == FlowNodeClass::Source
                && !expanded_output_ports_for_node(&runtime.definitions, &node)
                    .map_err(Self::Error::internal)?
                    .iter()
                    .any(|p| p.phase == FlowPortPhase::Execution)
            {
                FlowPortPhase::Snapshot
            } else {
                FlowPortPhase::Execution
            };
            runtime.node_phases.insert(node.id.clone(), phase);
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
                    if runtime.producers.contains_key(node_id) {
                        runtime.executed_nodes.remove(node_id);
                    } else {
                        self.finish_node_phase(runtime, &node, phase).await?;
                    }
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
        .await;
        step?;
        crate::durable::finish(self, runtime)
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
    ) -> Result<Retained<Token>, Self::Error> {
        let value = runtime
            .input_tokens
            .remove(&(node_id.into(), port_name.into()))
            .ok_or_else(|| {
                Self::Error::invalid_request(format!("No final value on {node_id:?}.{port_name}."))
            })?;
        runtime.consumed_tokens += 1;
        Ok(value)
    }

    fn inline_node_ready(&self, runtime: &Self::State, node_id: &str) -> bool {
        let Some(node) = runtime.nodes_by_id.get(node_id) else {
            return false;
        };
        let Some(definition) = node_definition(&runtime.definitions, node) else {
            return false;
        };
        if definition.kind == FlowNodeClass::Hybrid
            && !runtime.materialized_sources.contains(node_id)
        {
            return false;
        }
        let Ok(ports) = expanded_input_ports_for_node(&runtime.definitions, node) else {
            return false;
        };
        self.connected_input_ports(runtime, node_id)
            .iter()
            .all(|name| {
                let key = (node_id.to_string(), name.clone());
                let Some(port) = ports.iter().find(|p| p.name == *name) else {
                    return true;
                }; // automation is applied separately
                if port.phase == FlowPortPhase::Snapshot && definition.kind == FlowNodeClass::Hybrid
                {
                    return true;
                }
                if port.mode == FlowPortMode::Stream {
                    runtime.closed_inputs.contains(&key)
                } else {
                    runtime.input_tokens.contains_key(&key)
                }
            })
    }

    async fn emit_flow_token(
        &self,
        runtime: &mut Self::State,
        source_node_id: &str,
        source_port: &str,
        value: Token,
        token: &CancellationToken,
    ) -> Result<(), Self::Error> {
        self.check_cancelled(runtime, token)?;
        let source = runtime
            .nodes_by_id
            .get(source_node_id)
            .ok_or_else(|| Self::Error::internal("Missing token source."))?;
        let port = expanded_output_ports_for_node(&runtime.definitions, source)
            .map_err(Self::Error::internal)?
            .into_iter()
            .find(|p| p.name == source_port)
            .ok_or_else(|| Self::Error::invalid_request("Unknown output port."))?;
        let key = (source_node_id.to_string(), source_port.to_string());
        if runtime.closed_outputs.contains(&key)
            || (port.mode == FlowPortMode::FinalValue && runtime.final_outputs.contains(&key))
        {
            return Err(Self::Error::invalid_request(format!("Output {source_node_id:?}.{source_port} is closed or has already published its final value.")));
        }
        if runtime
            .node_phases
            .get(source_node_id)
            .is_some_and(|phase| *phase != port.phase)
        {
            return Err(Self::Error::invalid_request(format!(
                "Output {source_node_id:?}.{source_port} belongs to a different node phase."
            )));
        }
        if port.token_type != "auto" && !port_accepts_token_type(&port, &value.token_type) {
            return Err(Self::Error::invalid_request(format!(
                "Output {source_node_id:?}.{source_port} cannot emit token type {:?}.",
                value.token_type
            )));
        }
        let edges = runtime
            .outgoing_edges
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let bytes = crate::pump::measure(
            &(&value, &runtime.signal_cause),
            runtime.flow.execution.limits.retained_bytes as usize,
        )
        .map_err(|e| Self::Error::invalid_request(e.to_string()))?;
        // Check the complete fan-out before reserving or exposing any destination.
        for edge in &edges {
            let target = runtime
                .nodes_by_id
                .get(&edge.target_node_id)
                .ok_or_else(|| Self::Error::invalid_request("Unknown fan-out target."))?;
            let automation = expanded_automation_ports_for_node(&runtime.definitions, target)
                .map_err(Self::Error::invalid_request)?
                .into_iter()
                .find(|p| p.name == edge.target_port);
            if let Some(target_port) = automation {
                if port.mode != FlowPortMode::FinalValue
                    || !automation_port_accepts_token_type(&target_port, &value.token_type)
                {
                    return Err(Self::Error::invalid_request(format!(
                        "Incompatible automation destination on edge {:?}.",
                        edge.id
                    )));
                }
            } else {
                let target_port = expanded_input_ports_for_node(&runtime.definitions, target)
                    .map_err(Self::Error::invalid_request)?
                    .into_iter()
                    .find(|p| p.name == edge.target_port)
                    .ok_or_else(|| Self::Error::invalid_request("Unknown fan-out input."))?;
                if port.mode != target_port.mode
                    || !port_accepts_token_type(&target_port, &value.token_type)
                {
                    return Err(Self::Error::invalid_request(format!(
                        "Incompatible token destination on edge {:?}.",
                        edge.id
                    )));
                }
            }
            if edge
                .queue
                .as_ref()
                .is_some_and(|q| bytes > q.max_event_bytes as usize)
            {
                return Err(Self::Error::invalid_request(format!(
                    "Oversize token on edge {:?}: {bytes} bytes.",
                    edge.id
                )));
            }
        }
        let charges = loop {
            self.check_cancelled(runtime, token)?;
            let mut charges = HashMap::new();
            let mut blocked = None;
            // All lossless reservations precede lossy discards and branch visibility.
            let mut admission = edges.iter().collect::<Vec<_>>();
            admission.sort_by_key(|edge| {
                edge.queue
                    .as_ref()
                    .is_some_and(|q| q.policy == FlowQueuePolicy::DropOldest)
            });
            for edge in admission {
                loop {
                    let reserve = || -> Result<Vec<crate::retention::Reservation>, String> {
                        let mut reserved = vec![runtime.retention.reserve(
                            1,
                            bytes,
                            Some(&edge.target_node_id),
                        )?];
                        if let Some(budget) = runtime.edge_budgets.get(&edge.id) {
                            reserved.push(budget.reserve(1, bytes, None)?);
                        }
                        Ok(reserved)
                    };
                    match reserve() {
                        Ok(reserved) => {
                            charges.insert(edge.id.clone(), reserved);
                            break;
                        }
                        Err(reason) => {
                            let oldest = if edge
                                .queue
                                .as_ref()
                                .is_some_and(|q| q.policy == FlowQueuePolicy::DropOldest)
                            {
                                runtime
                                    .data_queue
                                    .iter()
                                    .position(|item| item.edge.id == edge.id)
                            } else {
                                None
                            };
                            if let Some(index) = oldest {
                                let old = runtime
                                    .data_queue
                                    .remove(index)
                                    .expect("queued lossy delivery");
                                let discarded_bytes = crate::pump::measure(
                                    &(old.value.as_ref(), &old.cause),
                                    usize::MAX,
                                )
                                .map_err(|e| Self::Error::internal(e.to_string()))?;
                                let sequence = old.sequence;
                                drop(old);
                                self.publish_execution_record(runtime, source_node_id, EventKind::TokenDrop, "Discarded oldest streaming delivery.".into(), Some(json!({"edgeId":edge.id,"sourcePort":source_port,"tokenType":value.token_type,"sequence":sequence,"encodedBytes":discarded_bytes}))).await;
                            } else {
                                blocked = Some(format!("edge {:?}: {reason}", edge.id));
                                break;
                            }
                        }
                    }
                }
                if blocked.is_some() {
                    break;
                }
            }
            if let Some(reason) = blocked {
                drop(charges);
                if !self.progress_delivery(runtime, token).await?
                    && !self.execute_ready_inline(runtime, token).await?
                {
                    return Err(Self::Error::invalid_request(format!(
                        "Capacity deadlock at {source_node_id:?}.{source_port}, {reason}"
                    )));
                }
            } else {
                break charges;
            }
        };
        if port.mode == FlowPortMode::FinalValue {
            runtime.final_outputs.insert(key);
        }
        runtime.emitted_tokens += 1;
        self.observe_token(runtime, source_node_id, source_port, &value);
        self.publish_execution_record(runtime, source_node_id, EventKind::TokenEmit, format!("Emitted {} on {source_port}.", value.token_type), Some(json!({"sourcePort":source_port,"tokenType":value.token_type,"targetCount":edges.len(),"value":value.value}))).await;
        let mut charges = charges;
        let emission = runtime.delivery_sequence;
        for edge in edges {
            let value = Retained::charged(
                value.clone(),
                charges.remove(&edge.id).expect("admitted edge"),
            );
            let sequence = runtime.delivery_sequence;
            runtime.delivery_sequence += 1;
            let cause = runtime.signal_cause.clone();
            runtime.data_queue.push_back(crate::delivery::DataDelivery {
                emission,
                sequence,
                edge,
                value,
                cause,
            });
        }
        Ok(())
    }

    async fn execute_sink_port(
        &self,
        runtime: &mut Self::State,
        node: &FlowNode,
        input_port: &str,
        value: Retained<Token>,
    ) -> Result<(), Self::Error> {
        let definition = node_definition(&runtime.definitions, node)
            .ok_or_else(|| Self::Error::internal("Missing sink definition."))?
            .clone();
        execute_registered_sink(self, runtime, node, &definition, input_port, value).await
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
                runtime.producers.remove(&node.id);
                runtime.executed_nodes.insert(node.id.clone());
                runtime.materialized_sources.insert(node.id.clone());
                self.discard_queued_input_tokens_for_node(runtime, &node.id);
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
        self.enqueue_ready_nodes(runtime);
        loop {
            self.check_cancelled(runtime, token)?;
            if self.progress_delivery(runtime, token).await? {
                continue;
            }
            if self.evaluate_ready_logic_nodes(runtime).await? {
                continue;
            }
            if self.execute_ready_inline(runtime, token).await? {
                continue;
            }
            if !runtime.producers.is_empty() {
                self.next_provider_event(runtime, token).await?;
                continue;
            }
            let blocked = runtime
                .flow
                .nodes
                .iter()
                .filter(|node| {
                    node_definition(&runtime.definitions, node).is_some_and(|d| {
                        matches!(
                            d.kind,
                            FlowNodeClass::Inline
                                | FlowNodeClass::Hybrid
                                | FlowNodeClass::Instrument
                        )
                    }) && !runtime.executed_nodes.contains(&node.id)
                })
                .map(|node| node.id.clone())
                .collect::<Vec<_>>();
            if !blocked.is_empty() {
                return Err(Self::Error::invalid_request(format!(
                    "Flow cannot complete: unsatisfied inputs on nodes {blocked:?}."
                )));
            }
            break;
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
            break;
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
        crate::durable::begin::<Self>(runtime, &node.id, "logic")?;
        let step: Result<(), Self::Error> = async {
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
        .await;
        step?;
        crate::durable::finish(self, runtime)
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
            .any(|p| p.name == signal_port)
        {
            return Ok(());
        }
        let edge = runtime
            .outgoing_signal_edges
            .get(&(node.id.clone(), signal_port.into()))
            .cloned();
        let token = runtime.activation_token.clone();
        if let Some(edge) = &edge {
            let bytes = crate::pump::measure(
                &(edge, &runtime.signal_cause),
                runtime.flow.execution.limits.retained_bytes as usize,
            )
            .map_err(|e| Self::Error::invalid_request(e.to_string()))?;
            let charge = loop {
                self.check_cancelled(runtime, &token)?;
                match runtime.retention.reserve(1, bytes, None) {
                    Ok(charge) => break charge,
                    Err(reason) => {
                        if !self.progress_delivery(runtime, &token).await? {
                            return Err(Self::Error::invalid_request(format!(
                                "Capacity deadlock on signal {}.{signal_port}: {reason}",
                                node.id
                            )));
                        }
                    }
                }
            };
            let sequence = runtime.delivery_sequence;
            runtime.delivery_sequence += 1;
            let cause = runtime.signal_cause.clone();
            runtime
                .control_queue
                .push_back(crate::delivery::ControlDelivery {
                    sequence,
                    control: crate::delivery::Control::Signal(edge.clone()),
                    cause,
                    _charge: charge,
                });
        }
        runtime
            .signal_sources_settled
            .insert((node.id.clone(), signal_port.into()));
        self.publish_execution_record(
            runtime,
            &node.id,
            EventKind::SignalEmit,
            format!("Emitted signal {signal_port}."),
            Some(json!({"signalPort":signal_port,"targetCount":usize::from(edge.is_some())})),
        )
        .await;
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
    fn check_cancelled(
        &self,
        runtime: &Self::State,
        token: &CancellationToken,
    ) -> Result<(), Self::Error> {
        if token.is_cancelled() || runtime.activation_token.is_cancelled() {
            Err(Self::Error::cancelled("Flow activation cancelled."))
        } else {
            Ok(())
        }
    }

    /// Register an owned stream. The handler returns; the driver applies its events.
    /// Reserving the complete mailbox bounds queued and borrowed provider payloads.
    async fn start_provider(
        &self,
        runtime: &mut Self::State,
        node_id: &str,
        source: impl futures_util::Stream<Item = crate::ProviderEvent> + Send + 'static,
    ) -> Result<(), Self::Error> {
        if runtime.producers.contains_key(node_id) {
            return Err(Self::Error::invalid_request(format!(
                "Node {node_id:?} already has an active provider."
            )));
        }
        let phase = *runtime
            .node_phases
            .get(node_id)
            .ok_or_else(|| Self::Error::internal("A provider must start in a node turn."))?;
        let q = runtime.flow.execution.limits.provider_queue.clone();
        let token = runtime.activation_token.clone();
        let capacity = loop {
            self.check_cancelled(runtime, &token)?;
            match runtime
                .retention
                .reserve(q.items as usize, q.bytes as usize, None)
            {
                Ok(reservation) => break reservation,
                Err(reason) => {
                    if !self.progress_delivery(runtime, &token).await?
                        && !self.execute_ready_inline(runtime, &token).await?
                    {
                        return Err(Self::Error::invalid_request(format!(
                            "Capacity deadlock starting provider on {node_id:?}: {reason}"
                        )));
                    }
                }
            }
        };
        let limits = crate::pump::PumpLimits::new(
            q.items as usize,
            q.bytes as usize,
            q.max_event_bytes as usize,
        )
        .map_err(|e| Self::Error::invalid_request(e.to_string()))?;
        runtime.producers.insert(
            node_id.into(),
            crate::delivery::Producer {
                pump: crate::pump::EventPump::spawn(source, limits, token),
                phase,
                _capacity: capacity,
            },
        );
        Ok(())
    }

    async fn close_flow_output(
        &self,
        runtime: &mut Self::State,
        node_id: &str,
        port: &str,
    ) -> Result<(), Self::Error> {
        let key = (node_id.to_string(), port.to_string());
        if runtime.closed_outputs.contains(&key) {
            return Ok(());
        }
        let bytes = crate::pump::measure(
            &(node_id, port, &runtime.signal_cause),
            runtime.flow.execution.limits.retained_bytes as usize,
        )
        .map_err(|e| Self::Error::invalid_request(e.to_string()))?;
        let token = runtime.activation_token.clone();
        let charge = loop {
            self.check_cancelled(runtime, &token)?;
            match runtime.retention.reserve(1, bytes, None) {
                Ok(charge) => break charge,
                Err(reason) => {
                    if !self.progress_delivery(runtime, &token).await? {
                        return Err(Self::Error::invalid_request(format!(
                            "Capacity deadlock closing {node_id:?}.{port}: {reason}"
                        )));
                    }
                }
            }
        };
        runtime.closed_outputs.insert(key);
        self.publish_execution_record(
            runtime,
            node_id,
            EventKind::TokenClose,
            "Output closed.".into(),
            Some(json!({"sourcePort":port,"direction":"output"})),
        )
        .await;
        let sequence = runtime.delivery_sequence;
        runtime.delivery_sequence += 1;
        let cause = runtime.signal_cause.clone();
        runtime
            .control_queue
            .push_back(crate::delivery::ControlDelivery {
                sequence,
                control: crate::delivery::Control::Close {
                    node: node_id.into(),
                    port: port.into(),
                },
                cause,
                _charge: charge,
            });
        Ok(())
    }

    async fn finish_node_phase(
        &self,
        runtime: &mut Self::State,
        node: &FlowNode,
        phase: FlowPortPhase,
    ) -> Result<(), Self::Error> {
        for port in expanded_output_ports_for_node(&runtime.definitions, node)
            .map_err(Self::Error::internal)?
        {
            if port.phase == phase {
                self.close_flow_output(runtime, &node.id, &port.name)
                    .await?;
            }
        }
        let definition = node_definition(&runtime.definitions, node)
            .ok_or_else(|| Self::Error::internal("Missing node definition."))?
            .clone();
        if phase == FlowPortPhase::Snapshot {
            runtime.materialized_sources.insert(node.id.clone());
        }
        if definition.kind != FlowNodeClass::Hybrid || phase == FlowPortPhase::Execution {
            runtime.executed_nodes.insert(node.id.clone());
            self.emit_post_activate_signal(runtime, node, &definition)
                .await?;
            self.mark_node_signal_ports_settled_except(runtime, node, &[])?;
        }
        if runtime.activated_node_id.as_deref() == Some(&node.id) {
            let bytes = crate::pump::measure(
                &(&node.id, &runtime.signal_cause),
                runtime.flow.execution.limits.retained_bytes as usize,
            )
            .map_err(|e| Self::Error::invalid_request(e.to_string()))?;
            let token = runtime.activation_token.clone();
            let charge = loop {
                match runtime.retention.reserve(1, bytes, None) {
                    Ok(charge) => break charge,
                    Err(reason) => {
                        if !self.progress_delivery(runtime, &token).await? {
                            return Err(Self::Error::invalid_request(format!(
                                "Capacity deadlock applying controller effects: {reason}"
                            )));
                        }
                    }
                }
            };
            let sequence = runtime.delivery_sequence;
            runtime.delivery_sequence += 1;
            let cause = runtime.signal_cause.clone();
            runtime
                .control_queue
                .push_back(crate::delivery::ControlDelivery {
                    sequence,
                    control: crate::delivery::Control::Controller(node.id.clone()),
                    cause,
                    _charge: charge,
                });
        }
        let inputs = expanded_input_ports_for_node(&runtime.definitions, node)
            .map_err(Self::Error::internal)?;
        for port in inputs {
            if port.phase == phase {
                runtime.input_tokens.remove(&(node.id.clone(), port.name));
            }
        }
        self.publish_execution_record(
            runtime,
            &node.id,
            EventKind::NodeComplete,
            format!("Completed {:?} phase.", phase),
            Some(json!({"definitionName":node.definition_name,"handlerId":definition.handler_id,"complete":runtime.executed_nodes.contains(&node.id),"phase":phase})),
        )
        .await;
        self.enqueue_ready_nodes(runtime);
        Ok(())
    }

    fn enqueue_ready_nodes(&self, runtime: &mut Self::State) {
        let mut ids = runtime
            .flow
            .nodes
            .iter()
            .filter(|node| {
                node_definition(&runtime.definitions, node).is_some_and(|definition| {
                    matches!(
                        definition.kind,
                        FlowNodeClass::Inline | FlowNodeClass::Hybrid | FlowNodeClass::Instrument
                    )
                }) && !runtime.started_nodes.contains(&node.id)
                    && !runtime.executed_nodes.contains(&node.id)
                    && !runtime.ready_inline_nodes.contains(&node.id)
                    && self.inline_node_ready(runtime, &node.id)
            })
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        runtime.ready_inline_nodes.extend(ids);
    }

    async fn execute_ready_inline(
        &self,
        runtime: &mut Self::State,
        token: &CancellationToken,
    ) -> Result<bool, Self::Error> {
        while let Some(id) = runtime.ready_inline_nodes.pop_front() {
            if runtime.started_nodes.contains(&id)
                || runtime.executed_nodes.contains(&id)
                || !self.inline_node_ready(runtime, &id)
            {
                continue;
            }
            self.check_cancelled(runtime, token)?;
            self.execute_flow_node(runtime, &id, None, token).await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn finish_sink_if_ready(
        &self,
        runtime: &mut Self::State,
        node: &FlowNode,
    ) -> Result<(), Self::Error> {
        if runtime.executed_nodes.contains(&node.id) {
            return Ok(());
        }
        let definition = node_definition(&runtime.definitions, node)
            .ok_or_else(|| Self::Error::internal("Missing sink definition."))?;
        if definition.kind != FlowNodeClass::Sink {
            return Ok(());
        }
        let ports = expanded_input_ports_for_node(&runtime.definitions, node)
            .map_err(Self::Error::internal)?;
        let ready = self
            .connected_input_ports(runtime, &node.id)
            .iter()
            .all(|port| {
                let key = (node.id.clone(), port.clone());
                if ports
                    .iter()
                    .find(|p| p.name == *port)
                    .is_some_and(|p| p.mode == FlowPortMode::Stream)
                {
                    runtime.closed_inputs.contains(&key)
                } else {
                    runtime.received_final_inputs.contains(&key)
                }
            });
        if ready {
            self.finish_node_phase(runtime, node, FlowPortPhase::Execution)
                .await?;
        }
        Ok(())
    }

    async fn progress_delivery(
        &self,
        runtime: &mut Self::State,
        token: &CancellationToken,
    ) -> Result<bool, Self::Error> {
        if runtime.data_queue.is_empty() && runtime.control_queue.is_empty() {
            return Ok(false);
        }
        crate::durable::begin::<Self>(runtime, "$scheduler", "deliver")?;
        let progress: Result<bool, Self::Error> = async {
        self.check_cancelled(runtime, token)?;
        let data_first = match (runtime.data_queue.front(), runtime.control_queue.front()) {
            (Some(data), Some(control)) => data.sequence < control.sequence,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => return Ok(false),
        };
        if data_first {
            if runtime
                .data_queue
                .front()
                .is_some_and(|delivery| runtime.active_delivery_groups.contains(&delivery.emission))
            {
                return Ok(false);
            }
            let delivery = runtime.data_queue.pop_front().expect("data front");
            runtime.active_delivery_groups.insert(delivery.emission);
            let previous = std::mem::replace(&mut runtime.signal_cause, delivery.cause.clone());
            let result = self
                .deliver_flow_token(runtime, &delivery.edge, delivery.value.clone(), token)
                .await;
            runtime.signal_cause = previous;
            runtime.active_delivery_groups.remove(&delivery.emission);
            if let Err(error) = &result {
                self.publish_execution_record(
                    runtime,
                    &delivery.edge.target_node_id,
                    EventKind::NodeError,
                    error.message().into(),
                    Some(json!({"edgeId":delivery.edge.id,"code":error.code()})),
                )
                .await;
            }
            result?;
        } else {
            let delivery = runtime.control_queue.pop_front().expect("control front");
            let previous = std::mem::replace(&mut runtime.signal_cause, delivery.cause.clone());
            let result = async {
                match &delivery.control {
                    crate::delivery::Control::Controller(node) => {
                        let values = runtime.activation_values.clone();
                        self.controller_activation_effects(runtime, node, &values);
                    }
                    crate::delivery::Control::Close { node, port } => {
                        let edges = runtime.outgoing_edges.get(&(node.clone(), port.clone())).cloned().unwrap_or_default();
                        for edge in edges {
                            runtime.closed_inputs.insert((edge.target_node_id.clone(), edge.target_port.clone()));
                            self.publish_execution_record(runtime,&edge.target_node_id,EventKind::TokenClose,"Input closed.".into(),Some(json!({"targetPort":edge.target_port,"sourceNode":node,"sourcePort":port,"direction":"input"}))).await;
                            let target = runtime.nodes_by_id.get(&edge.target_node_id).cloned().ok_or_else(|| Self::Error::internal("Missing closure target."))?;
                            self.finish_sink_if_ready(runtime, &target).await?;
                        }
                        self.enqueue_ready_nodes(runtime);
                    }
                    crate::delivery::Control::Signal(edge) => {
                        let target = runtime.nodes_by_id.get(&edge.target_node_id).cloned().ok_or_else(|| Self::Error::internal("Missing signal target."))?;
                        let key = (target.id.clone(), edge.target_port.clone());
                        if runtime.signal_cause.contains(&key) {
                            return Err(Self::Error::invalid_request(format!("Signal cycle: {:?} -> {:?} (edge {:?}).", runtime.signal_cause, key, edge.id)));
                        }
                        runtime.signal_cause.push(key.clone());
                        runtime.signal_action_latches.insert(key);
                        self.publish_execution_record(runtime, &target.id, EventKind::SignalReceive, format!("Received signal on {}.", edge.target_port), Some(json!({"edgeId":edge.id,"sourceNodeId":edge.source_node_id,"sourcePort":edge.source_port,"targetPort":edge.target_port}))).await;
                        self.execute_flow_action(runtime, &target, &edge.target_port).await?;
                    }
                }
                Ok(())
            }.await;
            runtime.signal_cause = previous;
            result?;
        }
        self.check_cancelled(runtime, token)?;
        Ok(true)
        }.await;
        let progressed = progress?;
        crate::durable::finish(self, runtime)?;
        Ok(progressed)
    }

    async fn deliver_flow_token(
        &self,
        runtime: &mut Self::State,
        edge: &FlowEdge,
        value: Retained<Token>,
        token: &CancellationToken,
    ) -> Result<(), Self::Error> {
        let target = runtime
            .nodes_by_id
            .get(&edge.target_node_id)
            .cloned()
            .ok_or_else(|| Self::Error::internal("Missing token target."))?;
        let definition = node_definition(&runtime.definitions, &target)
            .ok_or_else(|| Self::Error::internal("Missing target definition."))?
            .clone();
        if self.edge_source_is_disabled(runtime, edge)
            || self.node_is_disabled_output_capable(runtime, &target, &definition)
        {
            self.publish_execution_record(
                runtime,
                &target.id,
                EventKind::TokenSkip,
                "Disabled token target.".into(),
                Some(json!({"edgeId":edge.id,"targetPort":edge.target_port,"tokenType":value.token_type})),
            )
            .await;
            return Ok(());
        }
        if let Some(port) = expanded_automation_ports_for_node(&runtime.definitions, &target)
            .map_err(Self::Error::internal)?
            .into_iter()
            .find(|p| p.name == edge.target_port)
        {
            return self
                .apply_automation_write(
                    runtime,
                    edge,
                    &edge.source_node_id,
                    &edge.source_port,
                    &target,
                    &port,
                    &value,
                )
                .await;
        }
        let port = expanded_input_ports_for_node(&runtime.definitions, &target)
            .map_err(Self::Error::internal)?
            .into_iter()
            .find(|p| p.name == edge.target_port)
            .ok_or_else(|| Self::Error::internal("Missing input port."))?;
        self.publish_execution_record(runtime, &target.id, EventKind::TokenReceive, format!("Received {} on {}.", value.token_type, port.name), Some(json!({"edgeId":edge.id,"sourceNodeId":edge.source_node_id,"sourcePort":edge.source_port,"targetPort":edge.target_port,"tokenType":value.token_type,"value":value.value}))).await;
        runtime.node_phases.insert(target.id.clone(), port.phase);
        if port.mode == FlowPortMode::FinalValue
            && !runtime
                .received_final_inputs
                .insert((target.id.clone(), port.name.clone()))
        {
            return Err(Self::Error::invalid_request(format!(
                "Final input {:?}.{} received more than one value.",
                target.id, port.name
            )));
        }
        if definition.kind == FlowNodeClass::Sink {
            runtime.consumed_tokens += 1;
            self.execute_sink_port(runtime, &target, &port.name, value)
                .await?;
            self.finish_sink_if_ready(runtime, &target).await?;
        } else if port.mode == FlowPortMode::Stream {
            receive_registered_input_token(
                self,
                runtime,
                &target,
                &definition,
                &port.name,
                &value,
                token,
            )
            .await?;
            runtime.consumed_tokens += 1;
        } else {
            runtime
                .input_tokens
                .insert((target.id.clone(), port.name), value);
        }
        self.enqueue_ready_nodes(runtime);
        Ok(())
    }

    async fn await_snapshot_phase(
        &self,
        runtime: &mut Self::State,
        node: &str,
        token: &CancellationToken,
    ) -> Result<(), Self::Error> {
        while !runtime.materialized_sources.contains(node) {
            self.check_cancelled(runtime, token)?;
            if self.progress_delivery(runtime, token).await? {
                continue;
            }
            if self.evaluate_ready_logic_nodes(runtime).await? {
                continue;
            }
            if !runtime.producers.contains_key(node) {
                return Err(Self::Error::internal("Snapshot phase did not complete."));
            }
            self.next_provider_event(runtime, token).await?;
        }
        Ok(())
    }

    async fn next_provider_event(
        &self,
        runtime: &mut Self::State,
        token: &CancellationToken,
    ) -> Result<(), Self::Error> {
        crate::durable::begin::<Self>(runtime, "$providers", "provider_event")?;
        let step: Result<(), Self::Error> = async {
        use futures_util::future::select_all;
        // Each future borrows only its mailbox. No producer has access to graph state.
        let last = runtime.last_provider.clone();
        let mut candidates = runtime.producers.iter_mut().collect::<Vec<_>>();
        candidates.sort_by_key(|(node, _)| *node);
        if let Some(last) = last {
            let offset = candidates
                .iter()
                .position(|(node, _)| **node > last)
                .unwrap_or(0);
            candidates.rotate_left(offset);
        }
        let futures = candidates
            .into_iter()
            .map(|(node, producer)| {
                let node = node.clone();
                Box::pin(async move { (node, producer.pump.next().await) })
            })
            .collect::<Vec<_>>();
        let ((node_id, delivery), _, _) = tokio::select! {
            biased;
            _ = token.cancelled() => return Err(Self::Error::cancelled("Flow activation cancelled.")),
            value = select_all(futures) => value,
        };
        self.check_cancelled(runtime, token)?;
        runtime.last_provider = Some(node_id.clone());
        let phase = runtime
            .producers
            .get(&node_id)
            .expect("selected producer")
            .phase;
        runtime.node_phases.insert(node_id.clone(), phase);
        let node = runtime
            .nodes_by_id
            .get(&node_id)
            .cloned()
            .ok_or_else(|| Self::Error::internal("Missing provider node."))?;
        let definition = node_definition(&runtime.definitions, &node)
            .ok_or_else(|| Self::Error::internal("Missing provider definition."))?
            .clone();
        match delivery.map_err(|error| match error {
            crate::pump::PumpError::Cancelled => Self::Error::cancelled(error.to_string()),
            crate::pump::PumpError::ProducerPanicked | crate::pump::PumpError::ProducerStopped => {
                Self::Error::interrupted(error.to_string())
            }
            _ => Self::Error::invalid_request(error.to_string()),
        })? {
            Some(delivery) => {
                self.publish_execution_record(
                    runtime,
                    &node_id,
                    EventKind::ProviderEvent,
                    "Selected provider event.".into(),
                    Some(json!({"encodedBytes":delivery.encoded_bytes()})),
                )
                .await;
                match delivery.value() {
                    crate::ProviderEvent::Token { port, value } => {
                        self.emit_flow_token(runtime, &node_id, port, value.clone(), token)
                            .await?
                    }
                    crate::ProviderEvent::Signal { port } => {
                        self.emit_flow_signal(runtime, &node, port).await?
                    }
                    crate::ProviderEvent::Data { value } => {
                        self.handlers()
                            .get(&definition.handler_id)
                            .map_err(Self::Error::invalid_request)?
                            .receive_provider_event(self, runtime, &node, &definition, value, token)
                            .await?
                    }
                    crate::ProviderEvent::Failure { message } => {
                        return Err(Self::Error::internal(message.clone()))
                    }
                }
            }
            None => {
                runtime.producers.remove(&node_id);
                self.handlers()
                    .get(&definition.handler_id)
                    .map_err(Self::Error::invalid_request)?
                    .provider_complete(self, runtime, &node, &definition, token)
                    .await?;
                // Completion callbacks may start the next tool/provider turn.
                if !runtime.producers.contains_key(&node_id) {
                    self.finish_node_phase(runtime, &node, phase).await?;
                }
            }
        }
        Ok(())
        }.await;
        step?;
        crate::durable::finish(self, runtime)
    }
}

pub fn order_source_phase_materialization<S: std::ops::Deref<Target = SchedulerState>>(
    runtime: &S,
    source_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let included: HashSet<_> = source_ids.into_iter().collect();
    Ok(
        battersea_flow::execution::ordered_sources(&runtime.flow, &runtime.definitions)?
            .into_iter()
            .filter(|id| included.contains(id))
            .collect(),
    )
}
