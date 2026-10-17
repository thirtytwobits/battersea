# Configure services

Register protocol implementations once, then bind application-authored services and backends.
The service ID identifies an account or endpoint in errors, logs and continuations. The adapter
ID selects the wire protocol. Multiple services can use the same adapter.

```rust,ignore
let mut protocols = battersea_model::ProtocolRegistry::default();
battersea_providers::register_builtin_adapters(&mut protocols)?;
let configuration = battersea_config::parse_provider_configuration(source)?;
let validated = configuration.validate(&protocols)?;
let providers = validated.bind(&credentials, logger)?;
let chat = providers.chat("office-chat").ok_or("unknown backend")?;
```

`examples/declarative-providers` is a complete consumer with a YAML configuration.
`compose_backend_schema` exposes the `providerConfiguration` definition for application schemas.

## Bindings

A service has a chat binding, a media binding, or both. This provider block selects different
protocols for chat and image generation:

```yaml
version: 1
providers:
  office:
    chat:
      adapter: openai-chat-completions
      baseUrl: https://gateway.example/api/v1/
      auth:
        type: bearer
        credential: {env: OFFICE_API_KEY}
    media:
      adapter: openai-images
      baseUrl: https://images.example/v1/
      auth:
        type: header
        header: x-api-key
        credential: {host: image-account}
```

Backends refer to `provider: office` and declare their model, capabilities, properties and options.
`baseUrl` is an API directory: Chat Completions appends `chat/completions`, Responses appends
`responses`, and image generation appends `images/generations`. Keep proxy prefixes and API
versions in the base URL. Credentials use the selected authentication method. Use `type: none`
for services without authentication. Query authentication names its `parameter` explicitly.
`headers` contains non-secret protocol settings; Anthropic requires `anthropic-version` and
Runway requires `x-runway-version`.

| Adapter ID | Operations | Input counting |
|---|---|---|
| `openai-responses` | Streaming chat, background poll/stream | Native |
| `openai-chat-completions` | Chat and streaming chat | Unavailable |
| `anthropic-messages` | Streaming chat | Native |
| `google-generate-content` | Streaming chat | Native |
| `openai-images` | Image generation | — |
| `google-interactions` | Image generation | — |
| `runway-tasks` | Image/video jobs | — |
| `replicate-predictions` | Image/video/audio jobs | — |
| `mock` | Chat and media fixtures | Estimate |

Cargo [features](features.md) select the available registrations. Applications register additional
`ProtocolAdapter` descriptors in the same registry, supplying factories, capability ceilings and
an options schema. Constructors must avoid provider I/O. Change `contract_version` when a custom
adapter changes continuation or configuration semantics.

Bindings can restrict `content`, `parameters` and `counting`. Backend declarations must fit
within both the binding and adapter capabilities. `counting: unavailable` disables preflight
counting; callers must handle that result explicitly. Backend options are checked against the
registered adapter schema before construction.

## Lifetime and recovery

Validation retains the exact adapter registrations. Binding resolves credentials once per used
service operation and returns an immutable `ConfiguredProviders`. Hosts supply a
`CredentialResolver`, or `EnvironmentCredentials` with a non-secret scope revision. Change the
scope revision when a reference moves to another remote account. Rebind after changing secrets;
retain the previous registry for its active requests and jobs.

Persist `snapshot()` and `revision()` with accepted runs. Before resume, compare the reconstructed
registry using `verify_revision`. Endpoint, protocol, options and account-scope changes produce
different configuration revisions. Resolved credentials are excluded from snapshots and debug
formatting. Logs and errors mask credential bytes and their URL-encoded representations.

Native continuation blocks carry a service ID, adapter ID and binding revision. Replay requires
the same binding. Remote media jobs retain their binding for polling, cancellation and retrieval.
For authenticated output downloads on another origin, list that HTTPS origin in
`deliveryOrigins`; each redirect checks the destination again.

## Legacy conversion

The legacy `ProviderRegistry`, `register_builtin_providers` and backend readers remain available.
The version 1 provider block rejects backend-level endpoint and authentication overrides.

`upgrade_legacy_providers` copies existing backends using a complete map of
`chat/<backend>` and `media/<backend>` assignments. Each assignment names the intended service
and supplies its protocol, API-directory base URL and credential reference. This makes endpoint
interpretation and account ownership explicit, including proxies. Conflicting assignments fail.

`upgrade_provider_content` copies legacy native/reasoning blocks using the host's retained provider
assignment and accepted binding origin. Supply that origin only when retained records establish
ownership. Hosts persist converted documents and preserve accepted-run records. Reading a document
does not invoke either conversion.
