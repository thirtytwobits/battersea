//! Codes naming the refusals Battersea's runtime and generic nodes report through
//! [`ExecutionError`](crate::ExecutionError). Each code names one failure and is stable across
//! releases; the message carries the detail. Hosts choose their own codes for their refusals.

macro_rules! error_codes {
    ($($(#[doc = $doc:literal])+ $name:ident = $value:literal;)+) => {
        $($(#[doc = $doc])+ pub const $name: &str = $value;)+
        /// Every code Battersea's runtime and generic nodes report.
        pub const ALL: &[&str] = &[$($name),+];
    };
}

error_codes! {
    /// A classified node failure violates the bounded failure-payload contract.
    INVALID_NODE_FAILURE = "invalid_node_failure";
    /// A handler does not implement the entry point the runtime called, for example sink
    /// execution or logic evaluation.
    UNSUPPORTED_HANDLER_OPERATION = "unsupported_handler_operation";
    /// The host does not implement durable checkpoint capture or restoration.
    UNSUPPORTED_HOST_OPERATION = "unsupported_host_operation";
    /// No handler is registered for a definition's `handler_id`.
    UNKNOWN_HANDLER = "unknown_handler";
    /// The flow has no node with the requested id.
    UNKNOWN_NODE = "unknown_node";
    /// The scheduler holds no definition for a node's `definition_name`.
    UNKNOWN_DEFINITION = "unknown_definition";
    /// A node's dynamic port groups cannot be expanded from its parameter values.
    INVALID_DYNAMIC_PORTS = "invalid_dynamic_ports";
    /// The flow breaks its execution contract, including its source order.
    INVALID_EXECUTION_CONTRACT = "invalid_execution_contract";
    /// An action was invoked on a port the node does not offer.
    UNSUPPORTED_ACTION_PORT = "unsupported_action_port";
    /// A token was emitted on a port that is not an output of its node.
    UNKNOWN_OUTPUT_PORT = "unknown_output_port";
    /// A token was emitted on an output that is closed or has published its final value.
    CLOSED_OUTPUT = "closed_output";
    /// A token was emitted on an output of the phase the node is not running.
    MISMATCHED_OUTPUT_PHASE = "mismatched_output_phase";
    /// An output cannot carry the emitted token's type.
    INCOMPATIBLE_TOKEN_TYPE = "incompatible_token_type";
    /// A token edge's target node or port does not exist.
    UNKNOWN_EDGE_TARGET = "unknown_edge_target";
    /// A token edge's target does not accept the emission's mode or token type.
    INCOMPATIBLE_EDGE_TARGET = "incompatible_edge_target";
    /// A payload exceeds the byte limit of its activation, edge or provider mailbox.
    PAYLOAD_TOO_LARGE = "payload_too_large";
    /// A payload cannot be serialised to measure its size.
    UNSERIALISABLE_PAYLOAD = "unserialisable_payload";
    /// A reservation is blocked and no queued consumer can run to release capacity.
    CAPACITY_DEADLOCK = "capacity_deadlock";
    /// A retention budget refused a reservation outright.
    CAPACITY_EXHAUSTED = "capacity_exhausted";
    /// A handler took a final input that holds no value.
    MISSING_INPUT_VALUE = "missing_input_value";
    /// A final-value input received more than one value.
    DUPLICATE_FINAL_VALUE = "duplicate_final_value";
    /// The flow cannot complete because nodes still wait for inputs.
    UNSATISFIED_INPUTS = "unsatisfied_inputs";
    /// A signal reached an action port already on its causal path.
    SIGNAL_CYCLE = "signal_cycle";
    /// A node started a provider while its previous provider was still active.
    DUPLICATE_PROVIDER = "duplicate_provider";
    /// The provider mailbox limits cannot bound an event pump.
    INVALID_PUMP_LIMITS = "invalid_pump_limits";
    /// A provider's event pump panicked or stopped before its stream ended.
    PROVIDER_STOPPED = "provider_stopped";
    /// The activation task panicked.
    ACTIVATION_PANICKED = "activation_panicked";
    /// The activation was cancelled, by its token, a handler or a cancelled checkpoint.
    ACTIVATION_CANCELLED = "activation_cancelled";
    /// A required activation value is missing or has the wrong type.
    INVALID_ACTIVATION_VALUE = "invalid_activation_value";
    /// Resuming an activation requires the host to supply durable storage.
    DURABLE_STORAGE_REQUIRED = "durable_storage_required";
    /// A checkpoint no longer matches the activation entry or the registered handler versions.
    INCOMPATIBLE_CHECKPOINT = "incompatible_checkpoint";
    /// A handler declares checkpoint version zero.
    INVALID_CHECKPOINT_VERSION = "invalid_checkpoint_version";
    /// Execution stopped where its journal must be recovered. The message names the cause.
    EXECUTION_INTERRUPTED = "execution_interrupted";
    /// The execution journal is unavailable.
    EXECUTION_JOURNAL_MISSING = "execution_journal_missing";
    /// The execution journal's revision changed.
    STALE_EXECUTION_REVISION = "stale_execution_revision";
    /// The execution journal or a checkpoint in it is incompatible with this run.
    EXECUTION_JOURNAL_INCOMPATIBLE = "execution_journal_incompatible";
    /// A recovery operation is invalid for the journal's state.
    INVALID_RECOVERY_OPERATION = "invalid_recovery_operation";
    /// The journal store failed to read or write.
    EXECUTION_JOURNAL_STORAGE_FAILED = "execution_journal_storage_failed";
    /// Effects require explicit resolution before resume.
    UNRESOLVED_EFFECTS = "unresolved_effects";
    /// The host commit requires reconciliation.
    HOST_COMMIT_PENDING = "host_commit_pending";
    /// The execution already has a terminal outcome.
    EXECUTION_TERMINAL = "execution_terminal";
    /// A failed journal write may have committed; reopen the journal.
    EXECUTION_JOURNAL_POISONED = "execution_journal_poisoned";
}

/// The declared code equal to `code`, so a refusal can be raised again under another class.
pub(crate) fn declared(code: Option<&str>) -> Option<&'static str> {
    ALL.iter().copied().find(|declared| Some(*declared) == code)
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
