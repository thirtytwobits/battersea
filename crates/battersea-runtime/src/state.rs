use crate::delivery::{Cause, ControlDelivery, DataDelivery, Producer};
use crate::{Retained, RetentionBudget, Token};
use battersea_flow::{FlowDocument, FlowEdge, FlowEdgeKind, FlowNode, FlowNodeDefinition};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct SchedulerState {
    pub(crate) recovery: Option<crate::durable::DurableExecution>,
    pub(crate) continuation: crate::durable::Continuation,
    pub event_attempt: u64,
    pub event_sequence: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub run_id: String,
    pub flow: FlowDocument,
    pub definitions: HashMap<String, FlowNodeDefinition>,
    pub nodes_by_id: HashMap<String, FlowNode>,
    pub incoming_edges: HashMap<(String, String), FlowEdge>,
    pub outgoing_edges: HashMap<(String, String), Vec<FlowEdge>>,
    pub incoming_signal_edges: HashMap<(String, String), FlowEdge>,
    pub outgoing_signal_edges: HashMap<(String, String), FlowEdge>,
    pub input_tokens: HashMap<(String, String), Retained<Token>>,
    pub retention: RetentionBudget,
    pub(crate) edge_budgets: HashMap<String, RetentionBudget>,
    pub(crate) active_delivery_groups: HashSet<u64>,
    pub(crate) data_queue: VecDeque<DataDelivery>,
    pub(crate) control_queue: VecDeque<ControlDelivery>,
    pub(crate) delivery_sequence: u64,
    pub(crate) signal_cause: Cause,
    pub(crate) final_outputs: HashSet<(String, String)>,
    pub(crate) closed_outputs: HashSet<(String, String)>,
    pub(crate) received_final_inputs: HashSet<(String, String)>,
    pub(crate) closed_inputs: HashSet<(String, String)>,
    pub(crate) last_provider: Option<String>,
    pub(crate) producers: HashMap<String, Producer>,
    pub(crate) node_phases: HashMap<String, battersea_flow::FlowPortPhase>,
    pub(crate) started_nodes: HashSet<String>,
    pub ready_inline_nodes: VecDeque<String>,
    pub executed_nodes: HashSet<String>,
    pub materialized_sources: HashSet<String>,
    pub disabled_output_nodes: HashSet<String>,
    pub signal_action_latches: HashSet<(String, String)>,
    pub signal_sources_settled: HashSet<(String, String)>,
    pub logic_nodes_evaluated: HashSet<String>,
    pub emitted_tokens: u32,
    pub consumed_tokens: u32,
    pub(crate) activation_values_charge: Option<crate::retention::Reservation>,
    pub(crate) activated_node_id: Option<String>,
    pub activation_values: HashMap<String, Value>,
    pub activation_token: CancellationToken,
    pub cancellation_requested: bool,
}

fn str_error(s: String) -> String {
    s
}
impl SchedulerState {
    pub fn retained_values(&self) -> impl Iterator<Item = &serde_json::Value> {
        self.input_tokens
            .values()
            .map(|token| &token.value)
            .chain(self.data_queue.iter().map(|delivery| &delivery.value.value))
    }
    pub fn recovery_record(&self) -> Option<&crate::recovery::JournalRecord> {
        self.recovery
            .as_ref()
            .map(|recovery| recovery.journal.record())
    }
    pub fn new(
        run_id: String,
        flow: &FlowDocument,
        definitions: HashMap<String, FlowNodeDefinition>,
    ) -> Result<Self, String> {
        battersea_flow::execution::validate_execution_contract(flow, &definitions)?;
        let mut incoming_edges = HashMap::<(String, String), battersea_flow::FlowEdge>::new();
        let mut outgoing_edges = HashMap::<(String, String), Vec<battersea_flow::FlowEdge>>::new();
        let mut incoming_signal_edges =
            HashMap::<(String, String), battersea_flow::FlowEdge>::new();
        let mut outgoing_signal_edges =
            HashMap::<(String, String), battersea_flow::FlowEdge>::new();
        for edge in &flow.edges {
            match edge.kind {
                FlowEdgeKind::Token => {
                    let target_key = (edge.target_node_id.clone(), edge.target_port.clone());
                    if incoming_edges.insert(target_key, edge.clone()).is_some() {
                        return Err(str_error(format!(
                            "Input port \"{}\" on node \"{}\" already has an incoming edge.",
                            edge.target_port, edge.target_node_id
                        )));
                    }
                    let key = (edge.source_node_id.clone(), edge.source_port.clone());
                    outgoing_edges.entry(key).or_default().push(edge.clone());
                }
                FlowEdgeKind::Signal => {
                    let target_key = (edge.target_node_id.clone(), edge.target_port.clone());
                    if incoming_signal_edges
                        .insert(target_key, edge.clone())
                        .is_some()
                    {
                        return Err(str_error(format!(
                            "Action port \"{}\" on node \"{}\" already has an incoming edge.",
                            edge.target_port, edge.target_node_id
                        )));
                    }
                    let key = (edge.source_node_id.clone(), edge.source_port.clone());
                    if outgoing_signal_edges.insert(key, edge.clone()).is_some() {
                        return Err(str_error(format!(
                            "Output port \"{}\" on node \"{}\" already has an outgoing edge.",
                            edge.source_port, edge.source_node_id
                        )));
                    }
                }
            }
        }
        for edges in outgoing_edges.values_mut() {
            edges.sort_by(|a, b| (a.order, &a.id).cmp(&(b.order, &b.id)));
        }
        let limits = &flow.execution.limits;
        let retention = RetentionBudget::new(
            limits.pending_events as usize,
            limits.retained_bytes as usize,
            limits.node_retained_bytes as usize,
        );
        let edge_budgets = flow
            .edges
            .iter()
            .filter_map(|edge| {
                edge.queue.as_ref().map(|q| {
                    (
                        edge.id.clone(),
                        RetentionBudget::new(q.items as usize, q.bytes as usize, q.bytes as usize),
                    )
                })
            })
            .collect();
        Ok(Self {
            recovery: None,
            continuation: crate::durable::Continuation::Preflight,
            event_attempt: 0,
            event_sequence: Default::default(),
            run_id,
            flow: flow.clone(),
            definitions,
            nodes_by_id: flow
                .nodes
                .iter()
                .cloned()
                .map(|n| (n.id.clone(), n))
                .collect(),
            incoming_edges,
            outgoing_edges,
            incoming_signal_edges,
            outgoing_signal_edges,
            input_tokens: HashMap::new(),
            retention,
            edge_budgets,
            active_delivery_groups: HashSet::new(),
            data_queue: VecDeque::new(),
            control_queue: VecDeque::new(),
            delivery_sequence: 0,
            signal_cause: Vec::new(),
            final_outputs: HashSet::new(),
            closed_outputs: HashSet::new(),
            closed_inputs: HashSet::new(),
            received_final_inputs: HashSet::new(),
            last_provider: None,
            producers: HashMap::new(),
            node_phases: HashMap::new(),
            started_nodes: HashSet::new(),
            ready_inline_nodes: VecDeque::new(),
            executed_nodes: HashSet::new(),
            materialized_sources: HashSet::new(),
            disabled_output_nodes: HashSet::new(),
            signal_action_latches: HashSet::new(),
            signal_sources_settled: HashSet::new(),
            logic_nodes_evaluated: HashSet::new(),
            emitted_tokens: 0,
            consumed_tokens: 0,
            activation_values_charge: None,
            activated_node_id: None,
            activation_values: HashMap::new(),
            activation_token: CancellationToken::new(),
            cancellation_requested: false,
        })
    }
}
