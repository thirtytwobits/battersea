//! Copyright (c) Scott A Dixon
//! The refusals every Battersea operation declares, each under its Clapham category.
use battersea_model::ToolCode;
use clapham::{Category, Refusal};
use std::collections::BTreeMap;

macro_rules! codes {
    ($($(#[doc = $doc:literal])+ $name:ident = $value:literal, $category:ident;)+) => {
        $($(#[doc = $doc])+ pub const $name: &str = $value;)+
        /// Every code `battersea-ops` and its hosts' adapters refuse with, beyond Battersea's
        /// runtime, validation and tool-runner codes.
        pub const ALL: &[&str] = &[$($name),+];
        fn own(code: &str) -> Option<Category> {
            match code {
                $($value => Some(Category::$category),)+
                _ => None,
            }
        }
    };
}

codes! {
    /// The host holds no flow with the requested key.
    UNKNOWN_FLOW = "unknown_flow", NotFound;
    /// The host holds no run with the requested identity.
    UNKNOWN_RUN = "unknown_run", NotFound;
    /// The host holds no MCP connection with the requested server identity.
    UNKNOWN_MCP_SERVER = "unknown_mcp_server", NotFound;
    /// A stored flow's document cannot be read or upgraded.
    INVALID_FLOW = "invalid_flow", InvalidInput;
    /// An upgrade names a revision other than the one the host would upgrade from.
    STALE_UPGRADE_REVISION = "stale_upgrade_revision", Conflict;
    /// An upgrade that changes the flow names no expected revision.
    UPGRADE_REVISION_REQUIRED = "upgrade_revision_required", Usage;
    /// A reconciliation's completed result is not a tool result.
    INVALID_RECONCILIATION = "invalid_reconciliation", InvalidInput;
    /// The decision comes from the grant the run's own agent holds.
    OWN_GRANT_DECISION = "own_grant_decision", Refused;
    /// The host cannot serve the operation now; the caller may retry.
    HOST_UNAVAILABLE = "host_unavailable", Unavailable;
    /// The host failed the operation.
    HOST_FAILURE = "host_failure", Internal;
}

/// The category of a runtime error code.
fn runtime(code: &str) -> Category {
    use battersea_runtime::error_codes::*;
    match code {
        ACTIVATION_CANCELLED | EXECUTION_INTERRUPTED => Category::Interrupted,
        CAPACITY_EXHAUSTED
        | CAPACITY_DEADLOCK
        | PROVIDER_STOPPED
        | EXECUTION_JOURNAL_STORAGE_FAILED => Category::Unavailable,
        UNSUPPORTED_HOST_OPERATION
        | DURABLE_STORAGE_REQUIRED
        | DUPLICATE_PROVIDER
        | INVALID_PUMP_LIMITS => Category::Configuration,
        EXECUTION_JOURNAL_MISSING => Category::NotFound,
        STALE_EXECUTION_REVISION
        | EXECUTION_JOURNAL_INCOMPATIBLE
        | INCOMPATIBLE_CHECKPOINT
        | INVALID_CHECKPOINT_VERSION
        | INVALID_RECOVERY_OPERATION
        | UNRESOLVED_EFFECTS
        | HOST_COMMIT_PENDING
        | EXECUTION_TERMINAL
        | EXECUTION_JOURNAL_POISONED => Category::Conflict,
        ACTIVATION_PANICKED => Category::Internal,
        _ => Category::InvalidInput,
    }
}

/// The category of a tool-runner code.
fn tool(code: ToolCode) -> Category {
    match code {
        ToolCode::ToolApprovalDenied
        | ToolCode::ToolApprovalPending
        | ToolCode::ToolApprovalExpired => Category::Refused,
        ToolCode::ToolApprovalConflict
        | ToolCode::ToolApprovalUnresolved
        | ToolCode::ToolContinuationConflict
        | ToolCode::ToolContinuationUnresolved
        | ToolCode::ToolContinuationExpired => Category::Conflict,
        ToolCode::ToolApprovalStorage
        | ToolCode::ToolContinuationStorage
        | ToolCode::ToolApprovalCapacity => Category::Unavailable,
        _ => Category::InvalidInput,
    }
}

/// Every code a Battersea operation declares: its own, the runtime's error codes, flow
/// validation's issue codes and the tool runner's codes, each with its category.
pub fn declared() -> BTreeMap<String, Category> {
    let mut refusals = BTreeMap::new();
    for code in battersea_flow::issue_codes::ALL {
        refusals.insert(code.to_string(), Category::InvalidInput);
    }
    for code in battersea_runtime::error_codes::ALL {
        refusals.insert(code.to_string(), runtime(code));
    }
    for code in ToolCode::ALL {
        refusals.insert(code.as_str().to_string(), tool(*code));
    }
    for code in ALL {
        refusals.insert(code.to_string(), own(code).expect("an own code"));
    }
    refusals
}

/// The category every Battersea operation declares for `code`, if it declares it.
pub fn category(code: &str) -> Option<Category> {
    own(code)
        .or_else(|| {
            ToolCode::ALL
                .iter()
                .find(|tool_code| tool_code.as_str() == code)
                .map(|tool_code| tool(*tool_code))
        })
        .or_else(|| {
            battersea_runtime::error_codes::ALL
                .contains(&code)
                .then(|| runtime(code))
        })
        .or_else(|| {
            battersea_flow::issue_codes::ALL
                .contains(&code)
                .then_some(Category::InvalidInput)
        })
}

/// A completed refusal with `code` under its declared category. A code Battersea does not
/// declare is reported as [`HOST_FAILURE`], carrying the original code in `details`.
pub fn refusal(code: &str, message: impl Into<String>) -> Refusal {
    match category(code) {
        Some(category) => Refusal::new(code, category, message),
        None => {
            let mut refusal = Refusal::new(HOST_FAILURE, Category::Internal, message);
            refusal.details = Some(serde_json::json!({ "code": code }));
            refusal
        }
    }
}
