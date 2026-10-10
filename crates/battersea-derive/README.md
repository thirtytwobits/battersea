# Battersea derive

Derive `battersea_flow::catalog::NodeDefinition` on a Rust node type and attach one inline
manifest with `#[node_definition(manifest = r#"..."#)]`. The manifest has the same shape as
`Catalog::from_manifest`; formatter blocks and custom datatypes use the host's registry.
Use `#[node_definition(crate = renamed_flow, manifest = ...)]` for a renamed flow dependency.

Call `RegistryBuilder::register_defined` to register its handler and obtain the definition for
catalogue assembly. Execution is implemented through `NodeHandler`. See the repository's
`examples/custom-node` for a complete application.
