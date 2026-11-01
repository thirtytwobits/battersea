# Connect MCP tools

`examples/mcp` is an independent consumer of Clapham's MCP client. The application starts a controlled stdio
child process and local Streamable HTTP servers. A configured Chat Completions provider calls
the discovered tool, receives its result, and retries a rejected continuation without repeating
the effect. All servers run locally and use application-owned temporary storage.

## Host composition

Create an `McpConfig` with a stable server identity, credential-scope revision, finite bounds
and an operation timeout. The host supplies streams to `McpConnection::stdio`, or an approved
endpoint and resolved bearer credential to `McpConnection::http`. The example's `StdioHost`
launches and reaps its child; the adapter's shutdown token signals when that work can stop.

Discovery returns raw provenance, admitted tool names and diagnostics. Bind the admitted names the
host selects; `McpToolBinding::contribution` contributes them as operations under the connection's
server identity. Compose that contribution with the application's own, select the operations the
agent may call, and create a `ToolRegistry` under the run's verified grant. Name the host's profiles
with `ToolRegistry::with_profile` and apply `ToolRegistry::for_profile`. Create an
`ApprovedToolExecutor` with the host policy, conversation identity, the profile registry's
revision, an absolute deadline and a call capacity. Pass its definitions and executor to
`EngineTextStreamRequest`.

`ToolCallCompleted` retains content and completion evidence. Provider projections preserve text
and resource references. A tool-reported error reaches the provider as a result; protocol or
delivery failures stop the conversation with separate tool outcome evidence. See Clapham's
[MCP client](https://github.com/thirtytwobits/clapham/blob/v0.4.0/docs/mcp.md) and the
[tool contract](contracts/tools.md).

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

Denied and pending calls stop with `not_dispatched` evidence. Pending requires host interaction.
Retain the executor for the conversation: its bounded call-ID reservations prevent repeated
dispatch, including after cancellation. Recreating it does not restore those reservations. The
host must reconcile interrupted runs before replacing their execution state.


## Durable decisions

The example's `FileApprovalStore` persists one conversation per application-owned directory.
It bounds reads, serialises competing writes with a lock file, atomically replaces the journal,
and synchronises the file and directory before acknowledgement. The filesystem must support
durable replacement. A crash-left lock requires host inspection and executor fencing before removal.
Cancelled storage operations can finish in the background; subsequent commands inspect durable state.

Create a `DurableToolExecutor` with the profile's registry, binding revision, conversation
identity, absolute expiry and journal limits. Its provider-facing call persists the pending request
and stops before dispatch. Reopen the same scope, inspect the request, record the authenticated
host's decision, then explicitly resume the stored call. Published results remain available after
another reopen. An outstanding intent requires effect inspection and explicit reconciliation.

The consumer tests reopen file storage between pending, approval and completion over all three
transports. The effect occurs once. The provider receives
one generation request while testing the tool approval boundary.


## Provider continuation

The consumer persists a `DurableConversation` through an application-owned
`FileConversationStore`. Its completed provider turn exposes the pending tool batch. The host
records approval, resumes the tool, copies the published result into the conversation checkpoint,
and explicitly requests the next provider turn. Each boundary can be reopened using fresh store
handles and the original configured binding.

The consumer exercises ordinary and streaming Chat Completions, foreground streaming Responses
and background polling Responses over all three MCP transports. Responses retains native reasoning and function-call items across
reopen, including their opaque fields. A rejected
continuation reuses the saved transcript, the tool effect occurs once, and the remote result's
`_meta` stays out of provider requests. Failed, cancelled and invalid provider continuations retain an
unresolved intent and the preceding tool result. The host fences the original task before resolving
that intent. See the [provider contract](contracts/providers.md#durable-tool-conversations).

Background polling reopens the saved response identity before retrieving the tool batch and the
final response. Failed reads preserve the identity; explicit cancellation retains a terminal
receipt. The consumer verifies that dropping a pending conversation leaves cleanup to the host.
