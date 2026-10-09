# Battersea

Battersea is being extracted from Primrose Hill as an embeddable dataflow framework.

`battersea-flow` provides flow documents, ports, templates, extensible catalogues and pure
graph validation. Applications register their handlers, nominal token schemas, parameter
schemas and product validators. `examples/catalogue` is an independent host application.

`battersea-runtime` supplies node handlers, token and signal scheduling, typed execution events
and the activation lifecycle. Applications supply their state, effects and durable acceptance
through host interfaces. `examples/custom-node` runs a separately compiled node without a
product session.

`battersea-model` supplies content-block messages, provider and tool registration, typed errors,
request admission and the shared tool loop. `battersea-providers` supplies OpenAI, Anthropic,
Google, Runway and mock transports through individual Cargo features. Text admission accepts
text blocks; provider-native tool continuation retains opaque signed fields.

`battersea-config` composes backend configuration schemas with application schemas.
`battersea-nodes` supplies explicit node manifests and generic handlers. `battersea-guard`
scans source against the application's declared prompt roots. Applications retain their
prompts, product tools, storage and asset ownership.

`examples/backend` runs mock chat, a registered local tool and media generation independently.
Media submission returns either a completed result or a remote job with wait/cancel operations.

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
