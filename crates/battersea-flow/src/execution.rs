//! Versioned execution policy, source ordering and the explicitly invoked v1 upgrade.
use crate::{
    catalog::node_definition,
    document::FLOW_DOCUMENT_VERSION,
    ports::{
        expanded_automation_ports_for_node, expanded_input_ports_for_node,
        expanded_output_ports_for_node,
    },
    FlowDocument, FlowEdgeKind, FlowNodeClass, FlowNodeDefinition,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlowPortMode {
    FinalValue,
    Stream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlowPortPhase {
    Snapshot,
    Execution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlowQueuePolicy {
    Backpressure,
    DropOldest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowQueueLimits {
    pub items: u32,
    pub bytes: u32,
    pub max_event_bytes: u32,
    pub policy: FlowQueuePolicy,
}
impl Default for FlowQueueLimits {
    fn default() -> Self {
        Self {
            items: 32,
            bytes: 1024 * 1024,
            max_event_bytes: 256 * 1024,
            policy: FlowQueuePolicy::Backpressure,
        }
    }
}
impl FlowQueueLimits {
    pub fn validate(&self) -> Result<(), String> {
        if self.items == 0
            || self.bytes == 0
            || self.max_event_bytes == 0
            || self.max_event_bytes > self.bytes
        {
            return Err("Queue limits must be positive and allow one maximum-sized event.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowExecutionLimits {
    pub pending_events: u32,
    pub retained_bytes: u32,
    pub node_retained_bytes: u32,
    pub provider_queue: FlowQueueLimits,
}
impl Default for FlowExecutionLimits {
    fn default() -> Self {
        Self {
            pending_events: 4096,
            retained_bytes: 64 * 1024 * 1024,
            node_retained_bytes: 16 * 1024 * 1024,
            provider_queue: FlowQueueLimits::default(),
        }
    }
}
impl FlowExecutionLimits {
    pub fn validate(&self) -> Result<(), String> {
        self.provider_queue.validate()?;
        if self.pending_events == 0
            || self.retained_bytes == 0
            || self.node_retained_bytes == 0
            || self.node_retained_bytes > self.retained_bytes
            || self.provider_queue.bytes > self.retained_bytes
            || self.provider_queue.items > self.pending_events
            || self.provider_queue.policy != FlowQueuePolicy::Backpressure
        {
            return Err("Execution limits must be positive, fit the activation budget and keep provider events lossless.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowExecutionPolicy {
    pub source_order: Vec<String>,
    pub limits: FlowExecutionLimits,
}

fn source_ids(
    flow: &FlowDocument,
    definitions: &HashMap<String, FlowNodeDefinition>,
) -> Result<Vec<String>, String> {
    flow.nodes
        .iter()
        .filter_map(|node| match node_definition(definitions, node) {
            Some(definition)
                if matches!(
                    definition.kind,
                    FlowNodeClass::Source | FlowNodeClass::Hybrid
                ) =>
            {
                Some(Ok(node.id.clone()))
            }
            Some(_) => None,
            None => Some(Err(format!(
                "Unknown node definition {:?}; resolve its execution contract before upgrading.",
                node.definition_name
            ))),
        })
        .collect()
}

/// Stable topological source order, using authored priority among ready sources.
pub fn ordered_sources(
    flow: &FlowDocument,
    definitions: &HashMap<String, FlowNodeDefinition>,
) -> Result<Vec<String>, String> {
    let sources = source_ids(flow, definitions)?;
    let expected: HashSet<_> = sources.iter().collect();
    let declared: HashSet<_> = flow.execution.source_order.iter().collect();
    if expected.len() != sources.len()
        || declared.len() != flow.execution.source_order.len()
        || expected != declared
    {
        return Err("Source order must contain every snapshot source exactly once.".into());
    }
    let mut dependencies: HashMap<String, HashSet<String>> = HashMap::new();
    for node in &flow.nodes {
        if !expected.contains(&node.id) {
            continue;
        }
        let snapshot_inputs: HashSet<_> = expanded_input_ports_for_node(definitions, node)?
            .into_iter()
            .filter(|port| port.phase == FlowPortPhase::Snapshot)
            .map(|port| port.name)
            .collect();
        for edge in &flow.edges {
            if edge.kind == FlowEdgeKind::Token
                && edge.target_node_id == node.id
                && snapshot_inputs.contains(&edge.target_port)
            {
                if !expected.contains(&edge.source_node_id) {
                    return Err(format!(
                        "Snapshot input on {:?} depends on a non-source node.",
                        node.id
                    ));
                }
                dependencies
                    .entry(node.id.clone())
                    .or_default()
                    .insert(edge.source_node_id.clone());
            }
        }
    }
    let mut ordered = Vec::with_capacity(sources.len());
    let mut emitted = HashSet::new();
    while ordered.len() < sources.len() {
        let next = flow
            .execution
            .source_order
            .iter()
            .find(|id| {
                !emitted.contains(*id)
                    && dependencies
                        .get(*id)
                        .is_none_or(|deps| deps.iter().all(|dep| emitted.contains(dep)))
            })
            .ok_or("Snapshot source dependencies contain a cycle.")?;
        emitted.insert(next.clone());
        ordered.push(next.clone());
    }
    Ok(ordered)
}

/// Validate policy without effects or document mutation. Graph/type validation remains
/// part of catalogue validation, which calls this function as well.
pub fn validate_execution_contract(
    flow: &FlowDocument,
    definitions: &HashMap<String, FlowNodeDefinition>,
) -> Result<(), String> {
    if flow.version != FLOW_DOCUMENT_VERSION {
        return Err(format!(
            "Flow version {} requires an explicit upgrade.",
            flow.version
        ));
    }
    flow.execution.limits.validate()?;
    ordered_sources(flow, definitions)?;
    for node in &flow.nodes {
        let definition = node_definition(definitions, node).ok_or("Unknown node definition.")?;
        for port in expanded_input_ports_for_node(definitions, node)?
            .into_iter()
            .chain(expanded_output_ports_for_node(definitions, node)?)
        {
            if port.phase == FlowPortPhase::Snapshot
                && (!matches!(
                    definition.kind,
                    FlowNodeClass::Source | FlowNodeClass::Hybrid
                ) || port.mode == FlowPortMode::Stream)
            {
                return Err(format!(
                    "Snapshot port {:?} on {:?} must be a final-value source or hybrid port.",
                    port.name, node.id
                ));
            }
        }
    }
    for edge in &flow.edges {
        if edge.kind == FlowEdgeKind::Signal {
            if edge.queue.is_some() {
                return Err(format!(
                    "Signal edge {:?} cannot declare a streaming queue.",
                    edge.id
                ));
            }
            continue;
        }
        let source = flow
            .nodes
            .iter()
            .find(|node| node.id == edge.source_node_id)
            .ok_or("Edge has an unknown source.")?;
        let target = flow
            .nodes
            .iter()
            .find(|node| node.id == edge.target_node_id)
            .ok_or("Edge has an unknown target.")?;
        let output = expanded_output_ports_for_node(definitions, source)?
            .into_iter()
            .find(|port| port.name == edge.source_port)
            .ok_or("Edge has an unknown output.")?;
        let input = expanded_input_ports_for_node(definitions, target)?
            .into_iter()
            .find(|port| port.name == edge.target_port);
        let input_mode = match input {
            Some(input) => {
                if input.phase == FlowPortPhase::Snapshot && output.phase != FlowPortPhase::Snapshot
                {
                    return Err("Snapshot inputs require snapshot outputs.".into());
                }
                input.mode
            }
            None if expanded_automation_ports_for_node(definitions, target)?
                .iter()
                .any(|port| port.name == edge.target_port) =>
            {
                FlowPortMode::FinalValue
            }
            None => return Err("Edge has an unknown input.".into()),
        };
        if input_mode != output.mode {
            return Err(format!(
                "Edge {:?} connects different consumption modes.",
                edge.id
            ));
        }
        match (output.mode, &edge.queue) {
            (FlowPortMode::Stream, Some(queue)) => {
                queue.validate()?;
                if queue.bytes > flow.execution.limits.retained_bytes
                    || queue.items > flow.execution.limits.pending_events
                {
                    return Err(format!(
                        "Queue on edge {:?} exceeds activation limits.",
                        edge.id
                    ));
                }
            }
            (FlowPortMode::FinalValue, None) => {}
            _ => {
                return Err(format!(
                    "Edge {:?} must declare a queue exactly when it carries a stream.",
                    edge.id
                ))
            }
        }
    }
    Ok(())
}

/// Convert a v1 document only when explicitly invoked. The caller owns revision
/// fencing, a recoverable original and publication. Unknown catalogue entries fail.
pub fn upgrade_v1_document(
    source: &str,
    definitions: &HashMap<String, FlowNodeDefinition>,
    limits: FlowExecutionLimits,
) -> Result<FlowDocument, String> {
    let mut raw: Value = serde_json::from_str(source).map_err(|error| error.to_string())?;
    if raw.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("The v1 upgrade requires document version 1.".into());
    }
    if raw.get("execution").is_some()
        || raw
            .get("edges")
            .and_then(Value::as_array)
            .is_some_and(|edges| edges.iter().any(|edge| edge.get("queue").is_some()))
    {
        return Err("A v1 document cannot declare v2 execution policy.".into());
    }
    limits.validate()?;
    raw["version"] = serde_json::json!(FLOW_DOCUMENT_VERSION);
    raw["execution"] = serde_json::to_value(FlowExecutionPolicy {
        source_order: Vec::new(),
        limits,
    })
    .map_err(|error| error.to_string())?;
    let mut flow: FlowDocument = serde_json::from_value(raw).map_err(|error| error.to_string())?;
    flow.execution.source_order = source_ids(&flow, definitions)?;
    flow.execution.source_order = ordered_sources(&flow, definitions)?;
    for edge in &mut flow.edges {
        if edge.kind != FlowEdgeKind::Token {
            continue;
        }
        let node = flow
            .nodes
            .iter()
            .find(|node| node.id == edge.source_node_id)
            .ok_or("Edge has an unknown source.")?;
        let port = expanded_output_ports_for_node(definitions, node)?
            .into_iter()
            .find(|port| port.name == edge.source_port)
            .ok_or("Edge has an unknown source port.")?;
        if port.mode == FlowPortMode::Stream {
            edge.queue = Some(flow.execution.limits.provider_queue.clone());
        }
    }
    validate_execution_contract(&flow, definitions)?;
    Ok(flow)
}
