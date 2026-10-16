# Battersea

Battersea is an embeddable dataflow framework. It supplies graph contracts, execution,
model and provider interfaces, and an unstyled React editor. Your application supplies
sessions, storage, media ownership, presentation, and durable effects.

Start with [installation](installation.md) and [your first flow](first-flow.md).
Then choose [host integration](host-integration.md) or [editor integration](editor-integration.md).

The [Rust reference](../rust/index.html) and [TypeScript reference](../typescript/index.html)
describe the public APIs for this documentation version. Download the matching
[flow schema](../schemas/flow.schema.json), [runtime schema](../schemas/runtime.schema.json),
and [pianola roll schema](../schemas/pianola-roll.schema.json).

The [contracts](contracts.md) define behaviour. Reading a document never upgrades its stored
format. Your application explicitly invokes registered upgrades and decides when to persist them.

Use the [version index](../../versions/index.html) to choose another release.
