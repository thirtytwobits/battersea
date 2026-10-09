# Battersea

Battersea is being extracted from Primrose Hill as an embeddable dataflow framework.

`battersea-flow` provides flow documents, ports, templates, extensible catalogues and pure
graph validation. Applications register their handlers, nominal token schemas, parameter
schemas and product validators. `examples/catalogue` is an independent host application.

`battersea-runtime` supplies node handlers, token and signal scheduling, typed execution events
and the activation lifecycle. Applications supply their state, effects and durable acceptance
through host interfaces. `examples/custom-node` runs a separately compiled node without a
product session.

`@battersea/flow` carries the generated TypeScript contract, JSON Schema, shared fixtures
and token-connection compatibility. Rust types own the contract; `cargo xtask bindings`
generates its distributable forms.

Document inspection reports unsupported versions before interpreting graph fields. Loading
and canonical saving preserve opaque editor namespaces. Format upgrades are explicitly
registered transformations; the host owns their invocation and persistence.

Battersea is licensed under the [MIT License](LICENSE).

The backend is a Cargo workspace. TypeScript packages have a separate workspace in `typescript/`.
See [CONTRIBUTING.md](CONTRIBUTING.md) for validation and [RELEASING.md](RELEASING.md) for delivery.
Source attribution is recorded in [provenance.json](provenance.json).
