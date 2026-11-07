# Build a kit

A kit is a domain node set that applications share: one crate holding node definitions,
handlers, a port, a reference host and a conformance suite. The [kit contract](contracts/kits.md)
defines its parts, names and release rules.

## Write a kit

1. Choose the kit's name. It prefixes every handler id and token type and, in PascalCase,
   every class name. Declare it in `KIT`, and test the manifest with `Kit::check`.
2. Write `MANIFEST` as [the generic nodes do](custom-nodes.md), and `register_token_types`,
   which embeds the application's extension schema at each record type's `extension` key.
3. Define `Port` in the kit's own terms: the records it stores, the services it calls and the
   run state it keeps. Its only supertrait is `ExecutionHost`. Handlers are generic over
   `H: Port` and never name a concrete host.
4. Implement `Port` for `ReferenceHost<reference::State>` in memory, built `with_registry`
   when the kit registers parameter datatypes. Write the kit's contract
   with a distinguishing-proof table and test each clause against the reference host, advancing
   its `ManualClock` rather than waiting.
5. Export `conformance::run` behind the `conformance` feature. Each case is a flow in text
   source, its activation values, the clock steps during the run and the port operations
   before it. Implement `PortAccess<H: Port>` once: it applies those operations and observes
   the port's state for the comparison.
6. Add an example host under `examples/kit-NAME` that implements the port over something other
   than the reference host's maps and runs the suite.

A kit may depend on `battersea-flow`, `battersea-runtime`, `battersea-kit` and
`battersea-model`. A provider, tool runner or store reaches a handler through the port.

## Use a kit

1. Implement the kit's `Port` on your `ExecutionHost`, projecting the kit's `RunState` out of
   your run state and including it in your versioned host state.
2. Register the kit's token types with your extension schema, list its `MANIFEST` in
   `Catalog::from_manifests` beside the generic nodes and your own, and call its
   `register_handlers` on your `RegistryBuilder`.
3. Run `conformance::run` against your host in a test, with the kit's `conformance` feature
   enabled as a dev-dependency. The generic kit's suite is `battersea_nodes::conformance::run`;
   register its probe handlers beside the generic handlers in the test host, as
   `examples/custom-node` does.
4. Pin `Kit::model_facing_text` in a snapshot test. To change a formatter block, list a manifest
   holding only `formatter_blocks` after the kit's.

Kits with parameter renderers or controller widgets ship them as `@battersea/kit-NAME`; pass
them to `FlowEditor` as [editor integration](editor-integration.md) describes.
