use crate::Token;
use battersea_flow::{FlowDocument, FlowEdge, FlowEdgeKind, FlowNode, FlowNodeDefinition};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct SchedulerState {
    pub event_sequence: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub run_id: String,
    pub flow: FlowDocument,
    pub definitions: HashMap<String, FlowNodeDefinition>,
    pub nodes_by_id: HashMap<String, FlowNode>,
    pub incoming_edges: HashMap<(String, String), FlowEdge>,
    pub outgoing_edges: HashMap<(String, String), FlowEdge>,
    pub incoming_signal_edges: HashMap<(String, String), FlowEdge>,
    pub outgoing_signal_edges: HashMap<(String, String), FlowEdge>,
    pub input_tokens: HashMap<(String, String), Vec<Token>>,
    pub ready_inline_nodes: VecDeque<String>,
    pub executed_nodes: HashSet<String>,
    pub materialized_sources: HashSet<String>,
    pub disabled_output_nodes: HashSet<String>,
    pub signal_action_latches: HashSet<(String, String)>,
    /// `(node_id, action_port)` pairs currently on the signal-emission call
    /// stack. Used to detect back-edges (signal cycles) at runtime, since the
    /// validator does not reject signal-edge cycles. Inserted before recursing
    /// into a signal-triggered action and removed afterwards, so legitimate
    /// fan-in re-firing along distinct paths is preserved.
    pub active_signal_path: HashSet<(String, String)>,
    pub signal_sources_settled: HashSet<(String, String)>,
    pub logic_nodes_evaluated: HashSet<String>,
    pub emitted_tokens: u32,
    pub consumed_tokens: u32,
    pub activation_values: HashMap<String, Value>,
    pub activation_token: CancellationToken,
    pub cancellation_requested: bool,
}

fn str_error(s: String) -> String {
    s
}
impl SchedulerState {
    pub fn new(
        run_id: String,
        flow: &FlowDocument,
        definitions: HashMap<String, FlowNodeDefinition>,
    ) -> Result<Self, String> {
        battersea_flow::execution::validate_execution_contract(flow, &definitions)?;
        let mut incoming_edges = HashMap::<(String, String), battersea_flow::FlowEdge>::new();
        let mut outgoing_edges = HashMap::<(String, String), battersea_flow::FlowEdge>::new();
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
                    if outgoing_edges.insert(key, edge.clone()).is_some() {
                        return Err(str_error(format!(
                            "Output port \"{}\" on node \"{}\" already has an outgoing edge.",
                            edge.source_port, edge.source_node_id
                        )));
                    }
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
        Ok(Self {
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
            ready_inline_nodes: VecDeque::new(),
            executed_nodes: HashSet::new(),
            materialized_sources: HashSet::new(),
            disabled_output_nodes: HashSet::new(),
            signal_action_latches: HashSet::new(),
            active_signal_path: HashSet::new(),
            signal_sources_settled: HashSet::new(),
            logic_nodes_evaluated: HashSet::new(),
            emitted_tokens: 0,
            consumed_tokens: 0,
            activation_values: HashMap::new(),
            activation_token: CancellationToken::new(),
            cancellation_requested: false,
        })
    }
}
