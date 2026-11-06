//! Copyright (c) Scott A Dixon
//!
//! The generic kit `battersea`: node manifests and handlers that run on any
//! `ExecutionHost`. Its class names and token types are the shared vocabulary every kit
//! may reference; see `contracts/kits.md`.
pub mod concatenate;
pub mod logic;
pub mod multiplexer;
pub use battersea_kit::text;
pub use battersea_kit::{NodeError, NodeResult};

/// Generic node catalogue; pass it explicitly when composing an application catalogue.
pub const MANIFEST: &str = include_str!("nodes.yaml");
/// The generic kit's descriptor.
pub const KIT: battersea_kit::Kit = battersea_kit::Kit {
    name: battersea_kit::GENERIC_KIT,
    manifest: MANIFEST,
    token_types: battersea_kit::SHARED_TOKEN_TYPES,
};

/// Register the nominal token contracts used by this node library.
pub fn register_token_types(
    registry: &mut battersea_flow::registry::Registry,
) -> Result<(), String> {
    for name in KIT.token_types {
        registry
            .register_token_type(name, serde_json::Value::Bool(true))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
/// Install the generic execution handlers in the application's registry.
pub fn register_handlers<H: battersea_runtime::ExecutionHost>(
    builder: &mut battersea_runtime::RegistryBuilder<H>,
) -> Result<(), String> {
    builder.register(concatenate::ConcatenateHandler)?;
    builder.register(multiplexer::MultiplexerHandler)?;
    builder.register(logic::LogicMultiplexerHandler)?;
    builder.register(logic::LogicDemultiplexerHandler)?;
    builder.register(logic::AndGateHandler)?;
    builder.register(logic::OrGateHandler)?;
    builder.register(logic::InvertGateHandler)?;
    Ok(())
}
#[cfg(feature = "conformance")]
pub mod conformance;
#[cfg(test)]
fn test_catalog() -> battersea_flow::catalog::Catalog {
    let mut registry = battersea_flow::registry::Registry::default();
    register_token_types(&mut registry).unwrap();
    for id in [
        "battersea.concatenate",
        "battersea.multiplexer",
        "battersea.logic.multiplexer",
        "battersea.logic.demultiplexer",
        "battersea.logic.and",
        "battersea.logic.or",
        "battersea.logic.invert",
    ] {
        registry.register_handler(id).unwrap();
    }
    battersea_flow::catalog::Catalog::from_manifest(MANIFEST, registry).unwrap()
}
#[cfg(test)]
mod tests {
    #[test]
    fn generic_manifest_needs_only_library_contracts() {
        super::test_catalog();
    }
    #[test]
    fn manifest_composition_rejects_duplicate_definitions() {
        let mut registry = battersea_flow::registry::Registry::default();
        super::register_token_types(&mut registry).unwrap();
        for definition in super::test_catalog().entries() {
            registry.register_handler(&definition.handler_id).unwrap();
        }
        assert!(battersea_flow::catalog::Catalog::from_manifests(
            &[("first", super::MANIFEST), ("second", super::MANIFEST)],
            registry
        )
        .is_err());
    }
}
