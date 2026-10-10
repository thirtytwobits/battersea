//! Scheduler state at a driver boundary; no provider task or borrowed stack is serialised.
use crate::{delivery::*, recovery::RecoveryError, Retained, SchedulerState, Token};
use battersea_flow::ports::*;
use battersea_flow::{FlowDocument, FlowNode, FlowNodeDefinition, FlowPortPhase};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::Ordering;

pub const SCHEDULER_CHECKPOINT_VERSION: u32 = 1;
type PortKey = (String, String);
type PortResolver = fn(
    &std::collections::HashMap<String, FlowNodeDefinition>,
    &FlowNode,
) -> Result<Vec<ResolvedFlowPort>, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedInput {
    node: String,
    port: String,
    value: Token,
    charged_bytes: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedDelivery {
    emission: u64,
    sequence: u64,
    edge: String,
    value: Token,
    cause: Cause,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedControl {
    sequence: u64,
    control: Control,
    cause: Cause,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerCheckpoint {
    version: u32,
    run_id: String,
    flow: FlowDocument,
    definitions: BTreeMap<String, FlowNodeDefinition>,
    nodes_by_id: BTreeMap<String, FlowNode>,
    event_attempt: u64,
    event_sequence: u64,
    delivery_sequence: u64,
    input_tokens: Vec<SavedInput>,
    data_queue: Vec<SavedDelivery>,
    control_queue: Vec<SavedControl>,
    final_outputs: Vec<PortKey>,
    closed_outputs: Vec<PortKey>,
    received_final_inputs: Vec<PortKey>,
    closed_inputs: Vec<PortKey>,
    last_provider: Option<String>,
    node_phases: BTreeMap<String, FlowPortPhase>,
    started_nodes: Vec<String>,
    ready_inline_nodes: Vec<String>,
    executed_nodes: Vec<String>,
    materialized_sources: Vec<String>,
    disabled_output_nodes: Vec<String>,
    signal_action_latches: Vec<PortKey>,
    signal_sources_settled: Vec<PortKey>,
    logic_nodes_evaluated: Vec<String>,
    emitted_tokens: u32,
    consumed_tokens: u32,
    activated_node_id: Option<String>,
    activation_values: BTreeMap<String, Value>,
    cancellation_requested: bool,
}

fn invalid(message: impl Into<String>) -> RecoveryError {
    RecoveryError::Invalid(message.into())
}
fn sorted<T: Clone + Ord>(values: &HashSet<T>) -> Vec<T> {
    let mut values: Vec<_> = values.iter().cloned().collect();
    values.sort();
    values
}
fn measure(value: &impl Serialize, limit: usize) -> Result<usize, RecoveryError> {
    crate::pump::measure(value, limit).map_err(|error| invalid(error.to_string()))
}

impl SchedulerCheckpoint {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn flow_key(&self) -> &str {
        &self.flow.flow_key
    }
    pub fn validate_plan(&self, accepted: &Self) -> Result<(), RecoveryError> {
        let strip_parameters = |flow: &FlowDocument| {
            let mut flow = flow.clone();
            for node in &mut flow.nodes {
                node.parameter_values.clear();
            }
            serde_json::to_value(flow).map_err(|error| invalid(error.to_string()))
        };
        if self.run_id != accepted.run_id
            || strip_parameters(&self.flow)? != strip_parameters(&accepted.flow)?
            || serde_json::to_value(&self.definitions).map_err(|e| invalid(e.to_string()))?
                != serde_json::to_value(&accepted.definitions)
                    .map_err(|e| invalid(e.to_string()))?
        {
            return Err(RecoveryError::Incompatible(
                "Pinned graph or catalogue changed.".into(),
            ));
        }
        Ok(())
    }
    pub fn capture(state: &SchedulerState) -> Result<Self, RecoveryError> {
        if !state.producers.is_empty()
            || !state.active_delivery_groups.is_empty()
            || !state.signal_cause.is_empty()
        {
            return Err(invalid(
                "Checkpoint requires a driver boundary with no live provider or borrowed delivery.",
            ));
        }
        let mut inputs = state
            .input_tokens
            .iter()
            .map(|((node, port), value)| SavedInput {
                node: node.clone(),
                port: port.clone(),
                value: (**value).clone(),
                charged_bytes: value.primary_usage().bytes,
            })
            .collect::<Vec<_>>();
        inputs.sort_by(|a, b| (&a.node, &a.port).cmp(&(&b.node, &b.port)));
        let snapshot = Self {
            version: SCHEDULER_CHECKPOINT_VERSION,
            run_id: state.run_id.clone(),
            flow: state.flow.clone(),
            definitions: state.definitions.clone().into_iter().collect(),
            nodes_by_id: state.nodes_by_id.clone().into_iter().collect(),
            event_attempt: state.event_attempt,
            event_sequence: state.event_sequence.load(Ordering::Relaxed),
            delivery_sequence: state.delivery_sequence,
            input_tokens: inputs,
            data_queue: state
                .data_queue
                .iter()
                .map(|item| SavedDelivery {
                    emission: item.emission,
                    sequence: item.sequence,
                    edge: item.edge.id.clone(),
                    value: (*item.value).clone(),
                    cause: item.cause.clone(),
                })
                .collect(),
            control_queue: state
                .control_queue
                .iter()
                .map(|item| SavedControl {
                    sequence: item.sequence,
                    control: item.control.clone(),
                    cause: item.cause.clone(),
                })
                .collect(),
            final_outputs: sorted(&state.final_outputs),
            closed_outputs: sorted(&state.closed_outputs),
            received_final_inputs: sorted(&state.received_final_inputs),
            closed_inputs: sorted(&state.closed_inputs),
            last_provider: state.last_provider.clone(),
            node_phases: state.node_phases.clone().into_iter().collect(),
            started_nodes: sorted(&state.started_nodes),
            ready_inline_nodes: state.ready_inline_nodes.iter().cloned().collect(),
            executed_nodes: sorted(&state.executed_nodes),
            materialized_sources: sorted(&state.materialized_sources),
            disabled_output_nodes: sorted(&state.disabled_output_nodes),
            signal_action_latches: sorted(&state.signal_action_latches),
            signal_sources_settled: sorted(&state.signal_sources_settled),
            logic_nodes_evaluated: sorted(&state.logic_nodes_evaluated),
            emitted_tokens: state.emitted_tokens,
            consumed_tokens: state.consumed_tokens,
            activated_node_id: state.activated_node_id.clone(),
            activation_values: state.activation_values.clone().into_iter().collect(),
            cancellation_requested: state.cancellation_requested
                || state.activation_token.is_cancelled(),
        };
        measure(&snapshot, crate::recovery::MAX_JOURNAL_BYTES)?;
        Ok(snapshot)
    }

    pub fn restore(&self) -> Result<SchedulerState, RecoveryError> {
        if self.version != SCHEDULER_CHECKPOINT_VERSION {
            return Err(RecoveryError::Incompatible(format!(
                "scheduler state version {}",
                self.version
            )));
        }
        measure(self, crate::recovery::MAX_JOURNAL_BYTES)?;
        if self.run_id.trim().is_empty() {
            return Err(invalid("Empty run identity."));
        }
        let mut state = SchedulerState::new(
            self.run_id.clone(),
            &self.flow,
            self.definitions.clone().into_iter().collect(),
        )
        .map_err(invalid)?;
        if state.nodes_by_id.len() != self.nodes_by_id.len() {
            return Err(invalid("Checkpoint node inventory changed."));
        }
        for (id, node) in &self.nodes_by_id {
            let mut authored = state
                .nodes_by_id
                .get(id)
                .cloned()
                .ok_or_else(|| invalid("Unknown checkpoint node."))?;
            authored.parameter_values = node.parameter_values.clone();
            if serde_json::to_value(authored).map_err(|e| invalid(e.to_string()))?
                != serde_json::to_value(node).map_err(|e| invalid(e.to_string()))?
            {
                return Err(invalid("Checkpoint changed node identity or definition."));
            }
        }
        state.nodes_by_id = self.nodes_by_id.clone().into_iter().collect();
        let node_set = |values: &[String]| -> Result<HashSet<String>, RecoveryError> {
            let set: HashSet<_> = values.iter().cloned().collect();
            if set.len() != values.len() || set.iter().any(|id| !state.nodes_by_id.contains_key(id))
            {
                return Err(invalid("Invalid checkpoint node set."));
            }
            Ok(set)
        };
        state.started_nodes = node_set(&self.started_nodes)?;
        state.executed_nodes = node_set(&self.executed_nodes)?;
        state.materialized_sources = node_set(&self.materialized_sources)?;
        state.disabled_output_nodes = node_set(&self.disabled_output_nodes)?;
        state.logic_nodes_evaluated = node_set(&self.logic_nodes_evaluated)?;
        node_set(&self.ready_inline_nodes)?;
        state.ready_inline_nodes = self.ready_inline_nodes.iter().cloned().collect();
        for id in self
            .node_phases
            .keys()
            .chain(self.activated_node_id.iter())
            .chain(self.last_provider.iter())
        {
            if !state.nodes_by_id.contains_key(id) {
                return Err(invalid("Unknown checkpoint phase node."));
            }
        }
        state.node_phases = self.node_phases.clone().into_iter().collect();
        state.activated_node_id = self.activated_node_id.clone();
        state.last_provider = self.last_provider.clone();
        let port_set = |values: &[PortKey],
                        resolver: PortResolver|
         -> Result<HashSet<PortKey>, RecoveryError> {
            let set: HashSet<_> = values.iter().cloned().collect();
            if set.len() != values.len() {
                return Err(invalid("Duplicate checkpoint port."));
            }
            for (id, port) in &set {
                let node = state
                    .nodes_by_id
                    .get(id)
                    .ok_or_else(|| invalid("Unknown checkpoint port node."))?;
                if !resolver(&state.definitions, node)
                    .map_err(invalid)?
                    .iter()
                    .any(|candidate| candidate.id == *port)
                {
                    return Err(invalid("Unknown checkpoint port."));
                }
            }
            Ok(set)
        };
        state.final_outputs = port_set(&self.final_outputs, resolve_output_ports_for_node)?;
        state.closed_outputs = port_set(&self.closed_outputs, resolve_output_ports_for_node)?;
        state.received_final_inputs =
            port_set(&self.received_final_inputs, resolve_input_ports_for_node)?;
        state.closed_inputs = port_set(&self.closed_inputs, resolve_checkpoint_inputs)?;
        state.signal_action_latches =
            port_set(&self.signal_action_latches, resolve_action_ports_for_node)?;
        state.signal_sources_settled =
            port_set(&self.signal_sources_settled, resolve_signal_ports_for_node)?;
        let limit = self.flow.execution.limits.retained_bytes as usize;
        let mut sequences = HashSet::new();
        let mut previous = None;
        for saved in &self.data_queue {
            validate_cause(&state, &saved.cause)?;
            if saved.sequence >= self.delivery_sequence
                || saved.emission > saved.sequence
                || previous.is_some_and(|p| p >= saved.sequence)
                || !sequences.insert(saved.sequence)
            {
                return Err(invalid("Invalid data delivery order."));
            }
            previous = Some(saved.sequence);
            let edge = self
                .flow
                .edges
                .iter()
                .find(|edge| {
                    edge.id == saved.edge && edge.kind == battersea_flow::FlowEdgeKind::Token
                })
                .cloned()
                .ok_or_else(|| invalid("Unknown delivery edge."))?;
            let bytes = measure(&(&saved.value, &saved.cause), limit)?;
            let value = restore_token(&state, &edge, saved.value.clone(), bytes)?;
            state.data_queue.push_back(DataDelivery {
                emission: saved.emission,
                sequence: saved.sequence,
                edge,
                value,
                cause: saved.cause.clone(),
            });
        }
        previous = None;
        for saved in &self.control_queue {
            validate_cause(&state, &saved.cause)?;
            if saved.sequence >= self.delivery_sequence
                || previous.is_some_and(|p| p >= saved.sequence)
                || !sequences.insert(saved.sequence)
            {
                return Err(invalid("Invalid control delivery order."));
            }
            previous = Some(saved.sequence);
            let bytes = match &saved.control {
                Control::Signal(edge) => {
                    if !self.flow.edges.iter().any(|candidate| {
                        candidate == edge && edge.kind == battersea_flow::FlowEdgeKind::Signal
                    }) {
                        return Err(invalid("Unknown signal edge."));
                    }
                    measure(&(edge, &saved.cause), limit)?
                }
                Control::Controller(node) => {
                    if !state.nodes_by_id.contains_key(node) {
                        return Err(invalid("Unknown controller node."));
                    }
                    measure(&(node, &saved.cause), limit)?
                }
                Control::Close { node, port } => {
                    if !state.closed_outputs.contains(&(node.clone(), port.clone())) {
                        return Err(invalid("Unrecorded output closure."));
                    }
                    measure(&(node, port, &saved.cause), limit)?
                }
            };
            let charge = state.retention.reserve(1, bytes, None).map_err(invalid)?;
            state.control_queue.push_back(ControlDelivery {
                sequence: saved.sequence,
                control: saved.control.clone(),
                cause: saved.cause.clone(),
                _charge: charge,
            });
        }
        for saved in &self.input_tokens {
            let key = (saved.node.clone(), saved.port.clone());
            let edge = state
                .incoming_edges
                .get(&key)
                .ok_or_else(|| invalid("Unknown retained input."))?;
            if saved.charged_bytes < measure(&saved.value, limit)? {
                return Err(invalid("Retained input is undercharged."));
            }
            let value = restore_token(&state, edge, saved.value.clone(), saved.charged_bytes)?;
            if state.input_tokens.insert(key, value).is_some() {
                return Err(invalid("Duplicate retained input."));
            }
        }
        state.activation_values = self.activation_values.clone().into_iter().collect();
        if let Some(node) = &state.activated_node_id {
            let bytes = measure(
                &state.activation_values,
                self.flow.execution.limits.node_retained_bytes as usize,
            )?;
            state.activation_values_charge = Some(
                state
                    .retention
                    .reserve(0, bytes, Some(node))
                    .map_err(invalid)?,
            );
        } else if !state.activation_values.is_empty() {
            return Err(invalid("Activation values have no owner."));
        }
        state.event_attempt = self.event_attempt;
        state
            .event_sequence
            .store(self.event_sequence, Ordering::Relaxed);
        state.delivery_sequence = self.delivery_sequence;
        state.emitted_tokens = self.emitted_tokens;
        state.consumed_tokens = self.consumed_tokens;
        state.cancellation_requested = self.cancellation_requested;
        if self.cancellation_requested {
            state.activation_token.cancel();
        }
        Ok(state)
    }
}

fn restore_token(
    state: &SchedulerState,
    edge: &battersea_flow::FlowEdge,
    value: Token,
    bytes: usize,
) -> Result<Retained<Token>, RecoveryError> {
    let source = state
        .nodes_by_id
        .get(&edge.source_node_id)
        .ok_or_else(|| invalid("Unknown token source."))?;
    let target = state
        .nodes_by_id
        .get(&edge.target_node_id)
        .ok_or_else(|| invalid("Unknown token target."))?;
    let output = expanded_output_ports_for_node(&state.definitions, source)
        .map_err(invalid)?
        .into_iter()
        .find(|port| port.name == edge.source_port)
        .ok_or_else(|| invalid("Unknown token output."))?;
    if output.token_type != "auto" && !port_accepts_token_type(&output, &value.token_type) {
        return Err(invalid("Invalid checkpoint output token type."));
    }
    if let Some(automation) = expanded_automation_ports_for_node(&state.definitions, target)
        .map_err(invalid)?
        .into_iter()
        .find(|port| port.name == edge.target_port)
    {
        if output.mode != battersea_flow::FlowPortMode::FinalValue
            || !automation_port_accepts_token_type(&automation, &value.token_type)
        {
            return Err(invalid("Invalid checkpoint automation token."));
        }
    } else {
        let input = expanded_input_ports_for_node(&state.definitions, target)
            .map_err(invalid)?
            .into_iter()
            .find(|port| port.name == edge.target_port)
            .ok_or_else(|| invalid("Unknown token input."))?;
        if input.mode != output.mode || !port_accepts_token_type(&input, &value.token_type) {
            return Err(invalid("Invalid checkpoint input token type."));
        }
    }
    if edge
        .queue
        .as_ref()
        .is_some_and(|q| bytes > q.max_event_bytes as usize)
    {
        return Err(invalid("Oversize checkpoint delivery."));
    }
    let mut reservations = vec![state
        .retention
        .reserve(1, bytes, Some(&edge.target_node_id))
        .map_err(invalid)?];
    if let Some(budget) = state.edge_budgets.get(&edge.id) {
        reservations.push(budget.reserve(1, bytes, None).map_err(invalid)?);
    }
    Ok(Retained::charged(value, reservations))
}

fn validate_cause(state: &SchedulerState, cause: &Cause) -> Result<(), RecoveryError> {
    let mut seen = HashSet::new();
    for (id, port) in cause {
        let node = state
            .nodes_by_id
            .get(id)
            .ok_or_else(|| invalid("Unknown causal node."))?;
        if !seen.insert((id, port))
            || !resolve_action_ports_for_node(&state.definitions, node)
                .map_err(invalid)?
                .iter()
                .any(|candidate| candidate.id == *port)
        {
            return Err(invalid("Invalid causal signal path."));
        }
    }
    Ok(())
}

fn resolve_checkpoint_inputs(
    definitions: &std::collections::HashMap<String, FlowNodeDefinition>,
    node: &FlowNode,
) -> Result<Vec<ResolvedFlowPort>, String> {
    let mut ports = resolve_input_ports_for_node(definitions, node)?;
    ports.extend(resolve_automation_ports_for_node(definitions, node)?);
    Ok(ports)
}
