//! Copyright (c) Scott A Dixon
//!
//! The conversation kit `conversation`: the node set of a chat loop, reaching the
//! application's transcript, model, tools and spend through [`Port`].
//! `contracts/kit-conversation.md` defines its parts.
use battersea_flow::registry::Registry;
use serde_json::Value;

pub mod compaction;
pub mod port;
pub mod reference;
pub mod types;
pub use compaction::{Decision, Measured, Policy, PolicyEvaluation, Trigger};
pub use port::Port;
pub use types::*;
#[cfg(feature = "conformance")]
pub mod conformance;

/// The kit's name and namespace.
pub const NAME: &str = "conversation";
/// The kit's manifest: its node definitions and formatter blocks.
pub const MANIFEST: &str = include_str!("nodes.yaml");
/// The token types the kit registers.
pub const TOKEN_TYPES: &[&str] = &[
    "conversation.turn",
    "conversation.turns",
    "conversation.summary",
    "conversation.revision",
    "conversation.call_status",
    "conversation.tool_rounds",
];
/// The kit's descriptor.
pub const KIT: battersea_kit::Kit = battersea_kit::Kit {
    name: NAME,
    manifest: MANIFEST,
    token_types: TOKEN_TYPES,
};

/// The kit's recoverable node failure codes and the port's refusal codes.
pub mod codes {
    /// A chat request whose input tokens exceed the node's budget; nothing was dispatched.
    pub const OVER_BUDGET: &str = "conversation.over_budget";
    /// The port refused a spend reservation; nothing was dispatched.
    pub const SPEND_REFUSED: &str = "conversation.spend_refused";
    /// The model crate classified the provider's error as a request error.
    pub const PROVIDER_FAILED: &str = "conversation.provider_failed";
    /// The model asked for more tool rounds than the plan allows.
    pub const TOOL_ROUNDS_EXHAUSTED: &str = "conversation.tool_rounds_exhausted";
    /// A compaction or refinement response the parser cannot read.
    pub const UNPARSABLE_RESPONSE: &str = "conversation.unparsable_response";
    /// A transcript the port does not hold.
    pub const UNKNOWN_TRANSCRIPT: &str = "conversation.unknown_transcript";
    /// A turn the transcript does not hold.
    pub const UNKNOWN_TURN: &str = "conversation.unknown_turn";
    /// A job the port no longer knows.
    pub const UNKNOWN_JOB: &str = "conversation.unknown_job";
}

/// What the application supplies when registering the kit's token types.
#[derive(Debug, Clone)]
pub struct Options {
    /// The schema of the `extension` every record carries. The reference host uses `true`.
    pub extension_schema: Value,
}

/// Registers the kit's token types, each embedding `options.extension_schema` at its
/// record's `extension` key.
pub fn register_token_types(registry: &mut Registry, options: &Options) -> Result<(), String> {
    for (name, schema) in types::token_schemas(&options.extension_schema) {
        registry
            .register_token_type(name, schema)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
