# Connect MCP tools

`examples/mcp` is an independent Rust 1.88 consumer. The application starts a controlled stdio
child process and local Streamable HTTP servers. A configured Chat Completions provider calls
the discovered tool, receives its result, and retries a rejected continuation without repeating
the effect. All servers run locally and use application-owned temporary storage.

## Host composition

Create an `McpConfig` with a stable server identity, credential-scope revision, finite limits
and an operation timeout. The host supplies streams to `McpConnection::stdio`, or an approved
endpoint and resolved bearer credential to `McpConnection::http`. The example's `StdioHost`
launches and reaps its child; the adapter's shutdown token signals when that work can stop.

Discovery returns raw provenance, admitted tool names and diagnostics. Select admitted names
with host-authored profiles, bind them against the application's registry, and retain the binding
revision with the accepted run. `McpToolBinding::register_into` registers the immutable definitions
and executor atomically. Apply `ToolRegistry::for_profile`, then create an `ApprovedToolExecutor`
with the host policy, conversation identity, binding revision, absolute deadline and call capacity.
Pass its definitions and executor to `EngineTextStreamRequest`.

`ToolCallCompleted` retains content, host metadata and completion evidence. Provider projections
exclude host metadata and preserve text and resource references. A tool-reported error reaches
the provider as a result; protocol or delivery failures stop the conversation with separate tool
outcome evidence. See the [MCP contract](contracts/mcp.md) and [tool contract](contracts/tools.md).

## Effect ownership

The controlled server records each effect in a file owned by the example application. Fault tests
apply an effect and suppress its reply, then restart the server using the same file. Reconnecting
and discovering tools leaves the effect count at one. The host retains the uncertain outcome and
must reconcile the effect before deciding to issue another call.

Dropping a provider stream cancels local work and requests remote cancellation. A started call
may have no completion event, so the host's outstanding intent remains unresolved. The
[recovery contract](contracts/recovery.md) governs durable publication and reconciliation.

Profiles select which tools a run can use. The shared approval policy receives the tool record
and validated arguments before dispatch. It returns an available allow, deny or pending decision.
The example permits writes to its application-created fixture. Server annotations are author
claims; the host determines authority to execute.

Denied and pending calls stop with `not_dispatched` evidence. Pending requires host interaction;
explicit durable continuation is tracked in [#7](https://github.com/thirtytwobits/battersea/issues/7).
Retain the executor for the conversation: its bounded call-ID reservations prevent repeated
dispatch, including after cancellation. Recreating it does not restore those reservations. The
host must reconcile interrupted runs before replacing their execution state.
