//! Codes carried by the [`FlowValidationIssue`](crate::FlowValidationIssue)s that Battersea's
//! generic validation reports. Each code names one failure and is stable across releases.
//! Host graph rules choose their own codes.

macro_rules! issue_codes {
    ($($(#[doc = $doc:literal])+ $name:ident = $value:literal;)+) => {
        $($(#[doc = $doc])+ pub const $name: &str = $value;)+
        /// Every code Battersea's generic validation reports.
        pub const ALL: &[&str] = &[$($name),+];
    };
}

issue_codes! {
    /// The document's version is not the one this release validates.
    UNSUPPORTED_DOCUMENT_VERSION = "unsupported_document_version";
    /// The execution policy, port phases, port modes or edge queues break the execution contract.
    INVALID_EXECUTION_CONTRACT = "invalid_execution_contract";
    /// The token graph contains a cycle.
    GRAPH_CYCLE = "graph_cycle";
    /// A node's id is blank.
    BLANK_NODE_ID = "blank_node_id";
    /// Two nodes share an id.
    DUPLICATE_NODE_ID = "duplicate_node_id";
    /// A node's `definition_name` is blank.
    BLANK_DEFINITION_NAME = "blank_definition_name";
    /// A node's `instance_name` is blank.
    BLANK_INSTANCE_NAME = "blank_instance_name";
    /// A node names a definition the catalogue does not hold.
    UNKNOWN_DEFINITION = "unknown_definition";
    /// A node sets a parameter its definition does not declare.
    UNKNOWN_PARAMETER = "unknown_parameter";
    /// A node parameter's value breaks its datatype or editor constraints.
    INVALID_PARAMETER_VALUE = "invalid_parameter_value";
    /// `port_parameter_values` names a port the node does not have.
    PORT_PARAMETERS_FOR_UNKNOWN_PORT = "port_parameters_for_unknown_port";
    /// `port_parameter_values` sets a parameter the port does not declare.
    UNKNOWN_PORT_PARAMETER = "unknown_port_parameter";
    /// A port parameter's value breaks its datatype or editor constraints.
    INVALID_PORT_PARAMETER_VALUE = "invalid_port_parameter_value";
    /// A node's dynamic port groups cannot be expanded from its parameter values.
    INVALID_DYNAMIC_PORTS = "invalid_dynamic_ports";
    /// A node exposes one canonical port id on more than one side.
    DUPLICATE_PORT_ID = "duplicate_port_id";
    /// `port_names` aliases a port the node does not have.
    ALIAS_FOR_UNKNOWN_PORT = "alias_for_unknown_port";
    /// A port alias is blank.
    BLANK_PORT_ALIAS = "blank_port_alias";
    /// Two ports on one side of a node share an alias.
    DUPLICATE_PORT_ALIAS = "duplicate_port_alias";
    /// An edge's id is blank.
    BLANK_EDGE_ID = "blank_edge_id";
    /// Two edges share an id.
    DUPLICATE_EDGE_ID = "duplicate_edge_id";
    /// An edge's source node is not in the document.
    UNKNOWN_SOURCE_NODE = "unknown_source_node";
    /// An edge's target node is not in the document.
    UNKNOWN_TARGET_NODE = "unknown_target_node";
    /// A token edge's source port is not an output port of its node.
    UNKNOWN_SOURCE_PORT = "unknown_source_port";
    /// A token edge's target port is not an input or automation port of its node.
    UNKNOWN_TARGET_PORT = "unknown_target_port";
    /// A token edge does not run from an output port to an input or automation port.
    MISDIRECTED_TOKEN_EDGE = "misdirected_token_edge";
    /// A token edge's source type can never satisfy its target port.
    INCOMPATIBLE_TOKEN_TYPES = "incompatible_token_types";
    /// An input or automation port has more than one incoming token edge.
    DUPLICATE_INPUT_EDGE = "duplicate_input_edge";
    /// A signal edge's source port is not a signal port of its node.
    UNKNOWN_SIGNAL_PORT = "unknown_signal_port";
    /// A signal edge's target port is not an action port of its node.
    UNKNOWN_ACTION_PORT = "unknown_action_port";
    /// A signal port has more than one outgoing edge.
    DUPLICATE_SIGNAL_EDGE = "duplicate_signal_edge";
    /// An action port has more than one incoming edge.
    DUPLICATE_ACTION_EDGE = "duplicate_action_edge";
}

#[cfg(test)]
mod tests {
    use super::ALL;
    use std::collections::HashSet;

    #[test]
    fn codes_are_distinct_snake_case_names() {
        let mut seen = HashSet::new();
        for code in ALL {
            assert!(seen.insert(*code), "{code} is declared twice");
            assert!(
                code.split('_')
                    .all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase())),
                "{code} is not snake_case"
            );
        }
    }
}
