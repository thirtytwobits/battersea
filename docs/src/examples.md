# Examples

Run examples from a checkout of the documentation's release tag:

```sh
cargo run --locked -p catalogue-example
cargo run --locked -p custom-node-example
cargo run --locked -p backend-example
cargo run --locked -p declarative-providers-example
cargo run --locked -p mcp-example
cargo run --locked -p editor-server-example -- --help
```

The catalogue example validates and round-trips application-owned graphs. The custom-node
example implements a complete host. The backend example registers mock chat, tools, and
media independently of a product session.
The declarative-provider example binds two service identities to the same mock adapter and
checks their diagnostic identities. Its contract tests exercise HTTP protocols with local fixtures.

The [MCP consumer](mcp.md) launches a controlled stdio child and local
HTTP JSON/SSE servers, then delivers remote tool results through a provider conversation.
Its tests inject lost replies, disconnects, cancellation and provider continuation retries.

The [editor server](editor-server.md) joins installed npm packages to an independent Rust
application. To try its API in an empty application directory:

```sh
cargo run --locked -p editor-server-example -- init /tmp/battersea-example
cargo run --locked -p editor-server-example -- serve /tmp/battersea-example
```

Use a new directory for each trial. The example's application owns its saved documents and
activation records. Starting the server does not redispatch accepted work.
