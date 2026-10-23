//! Authored failure policies and runtime-owned failure outputs.
use crate::{
    FlowNodeClass, FlowNodeDefinition, FlowPort, FlowPortKind, FlowPortMode, FlowPortPhase,
};
use serde::{Deserialize, Serialize};

pub const NODE_FAILURE_TOKEN_TYPE: &str = "node_failure";
pub const FAILURE_CODE_BYTES: usize = 128;
pub const FAILURE_MESSAGE_BYTES: usize = 4096;

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FlowFailurePolicy {
    #[default]
    Fail,
    Emit,
    Skip,
}
impl FlowFailurePolicy {
    pub fn is_fail(&self) -> bool {
        *self == Self::Fail
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowFailurePort {
    pub name: String,
    pub phase: FlowPortPhase,
}
impl FlowFailurePort {
    pub fn as_port(&self) -> FlowPort {
        FlowPort {
            name: self.name.clone(),
            kind: FlowPortKind::Output,
            token_type: NODE_FAILURE_TOKEN_TYPE.into(),
            mode: FlowPortMode::FinalValue,
            phase: self.phase,
            display_class: None,
            accepted_token_types: vec![],
            short_description: Some("Handled node failure.".into()),
            long_description: None,
            formatter: None,
            parameters: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowNodeFailure {
    pub id: String,
    pub node_id: String,
    pub phase: FlowPortPhase,
    pub operation: String,
    pub code: String,
    pub message: String,
}

/// Every phase that can invoke a node callback, including authored actions.
pub fn failure_phases(definition: &FlowNodeDefinition) -> Vec<FlowPortPhase> {
    let mut phases = vec![];
    if matches!(
        definition.kind,
        FlowNodeClass::Source | FlowNodeClass::Hybrid
    ) {
        phases.push(FlowPortPhase::Snapshot);
    }
    if definition.kind != FlowNodeClass::Source
        || definition
            .output_ports
            .iter()
            .any(|p| p.phase == FlowPortPhase::Execution)
        || definition
            .dynamic_output_ports
            .iter()
            .any(|p| p.phase == FlowPortPhase::Execution)
        || definition
            .action_ports
            .iter()
            .any(|p| p.name != crate::catalog::FLOW_ACTION_DISABLE)
        || !definition.dynamic_action_ports.is_empty()
        || definition
            .controller_actions
            .as_ref()
            .is_some_and(|ports| !ports.is_empty())
    {
        phases.push(FlowPortPhase::Execution);
    }
    phases
}

pub fn validate_failure_ports(definition: &FlowNodeDefinition) -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    let mut phases = vec![];
    for port in &definition.failure_ports {
        if port.name.trim().is_empty()
            || !names.insert(&port.name)
            || phases.contains(&port.phase)
            || !failure_phases(definition).contains(&port.phase)
            || definition.output_ports.iter().any(|p| p.name == port.name)
        {
            return Err(format!(
                "Invalid failure port {:?} on {:?}.",
                port.name, definition.class_name
            ));
        }
        phases.push(port.phase);
    }
    Ok(())
}
