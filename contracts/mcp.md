# MCP adapter

`battersea-mcp` uses rmcp 3.6.0 and MCP 2026-07-28. The `stdio` and `http` features select
transports. The adapter requires Rust 1.88; core crates retain Rust 1.85.

## Ownership and discovery

The host supplies a stable server identity, a non-secret credential-scope revision, finite
limits and an operation timeout. Stdio accepts host-created streams and a cancellation token
that signals the host to reap its process. HTTP accepts a host-approved endpoint and bearer
credential. Closing or dropping the connection stops transport work. Bindings retain their
connection until released or explicitly closed.

Connections use `server/discover` and self-contained request metadata. Discovery reads fresh
pages and bounds page count, tool count, schema size and total catalogue bytes. Repeated cursors
are refused. Dropping an active discovery operation closes its connection. Invalid records receive individual diagnostics. Duplicate remote names are invalid.
The registry admits schemas without remote reference fetching.

Catalogue provenance retains the server's record. Exposed records receive profiles from the
host's selection. Remote profile metadata cannot grant access. The host applies
`ToolRegistry::for_profile` when constructing the run's permitted tools.

## Bindings

A binding selects remote names and assigns aliases. Generated aliases join the server identity
and remote name with a dot. Registry admission checks name length and collisions after
`local_tool_name` normalisation, including existing host tools. Explicit aliases resolve
collisions. Registration is atomic.

The binding revision hashes server identity, credential scope, selected definitions, aliases
and exposed records. A tool-list notification or a refresh that fails after it starts makes existing bindings stale.
A successful refresh permits an existing binding to dispatch only while its selected definitions
remain identical. Changing credentials requires another connection and binding.

## Dispatch and results

Registry validation precedes dispatch. An operation deadline covers queueing and invocation.
Active and waiting calls are bounded. Queue refusals, invalid arguments and stale bindings carry
`not_dispatched` evidence. A completed response carries `completed` evidence, including tool
errors, JSON-RPC invocation errors and failed output validation. Lost replies, disconnects and
invocation deadlines carry `uncertain` evidence.

Calls use one `tools/call` round. Dropping the call future requests cancellation; cancellation
does not prove that remote effects stopped. HTTP redirects, POST retries, SSE reconnects and
session reinitialisation are disabled. Reconnecting and discovering tools does not replay calls.
The host reconciles uncertain effects before deciding whether to issue another call.

Ordered text, image, audio, resource links and embedded resources map to shared tool results.
Structured null remains distinct from absent output. Result metadata stays in the host result;
provider projection follows the [tool contract](tools.md). JSON-RPC diagnostics retain their
numeric code, excluding remote diagnostic text and data.

Sampling, roots, elicitation and tasks are unadvertised. Server requests for unsupported
interactions are rejected. `input_required` and task results stop execution with uncertainty;
they do not initiate follow-up calls. The host controls approval, durable effect intent and
reconciliation.

## Bounds

`McpLimits` bounds each stdio frame and the complete body of each HTTP response, including SSE.
Oversized frames fail the connection explicitly. Limits also bound catalogue retention,
concurrent operations, waiting operations and incoming requests/notifications over the
connection's lifetime. Incoming request and notification payloads share a 64 MiB lifetime budget,
measured from received payload bytes including JSON whitespace.
Queued and active message capacity must fit within 64 MiB. A host can open a new connection
after a lifetime limit is reached.
Credentials and response diagnostic data are excluded from adapter error messages.
