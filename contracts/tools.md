# Application tools

## Operations

Application tools are [Clapham](https://github.com/thirtytwobits/clapham) operations. The host
composes its contributions, including bound remote MCP tools, into a `clapham::ComposedSet`, selects
the operations an agent may call, and verifies the grant the run's calls are made under. Clapham's
composition refuses a name outside the tool-name alphabet, a missing description, an input schema
without an object root and a schema that does not compile.

## Records

`battersea_tools::Tool` is the Model Context Protocol 2026-07-28 tool record, named by
`MCP_PROTOCOL_REVISION`. Its fields keep the protocol's spelling, `_meta` included.
`tools.schema.json` publishes it, and `@battersea/flow` carries its TypeScript types.
`@battersea/flow` also carries Clapham's `operation.schema.json` and its `Operations` types.

`Tool::for_operation` presents a descriptor as Clapham's MCP server does: its name, title,
description and schemas; every hint stated from its effect; `_meta["io.github.thirtytwobits.clapham/effect"]`
for an `administrative` effect; and `_meta["io.github.thirtytwobits.clapham/records"]` for an
operation that streams records.

| Effect | `readOnlyHint` | `destructiveHint` | `idempotentHint` |
|---|---|---|---|
| `query` | true | false | true |
| `mutation` | false | false | as declared |
| `administrative` | false | false | as declared |
| `destructive` | false | true | as declared |

`openWorldHint` is the descriptor's `open_world`. Each hint is a claim of the operation's author.
`Tool::effect` reads every hint at its protocol default: an unannotated record may change and
destroy, is not idempotent, and reaches outside its own domain.

## Profiles

A profile is a named selection. `ToolRegistry::with_profile` names the operations a profile is
permitted to call. It refuses a blank profile, a profile already named, and an operation outside
the registry's selection as `invalid_tool_definition`. The presented record lists the profiles
that select it under `_meta["io.github.thirtytwobits.battersea/profiles"]`, `PROFILES_META_KEY`.
An operation no profile names is permitted to none. The application names its profiles and
chooses each run's profile.

## Registry

`ToolRegistry::new` offers one selection of a composed set under one verified `OAuthContext`. It
refuses two operations whose provider-facing names are the same as `duplicate_tool`.

A provider is offered each operation under its provider-facing name, `local_tool_name` of its own,
with its description exactly as written. `ToolRegistry::for_profile` narrows the selection to one
profile's operations. `ToolRegistry::revision` is the selection's revision: its descriptors and
the remote binding revisions it includes.

A call names its operation by either name. The registry refuses a call to an operation it does not
offer as `unregistered_tool`, and arguments outside the operation's input schema as
`invalid_tool_arguments`, before dispatch. Dispatch runs through `ComposedSet::call` under the
registry's grant: the grant's binding and scopes, the gate and the resource policies are checked
before the handler receives the call under the operation's own name.

A refusal reaches the tool loop with its message and evidence. Clapham's `invalid_input`,
`invalid_result` and `unknown_operation`, and any code spelled as a `ToolCode`, set `tool_code`
with the classification `invalid_request`. Any other code, such as `insufficient_scope` or a
remote server's `mcp_busy`, is the classification, unchanged.

## Results

`EngineLocalToolResult::json` retains ordinary application JSON. A content result carries
ordered blocks, optional structured output, an execution-error flag and host metadata.
A completed operation result without content is application JSON. Any other outcome is a content
result whose blocks are the outcome's MCP content blocks; a block the tool loop cannot represent
is refused as `unsupported_tool_content`. A registry result carries no host metadata.

Where an operation declares an output schema, Clapham validates a successful result. Missing or
invalid structured output is refused as `invalid_tool_result`. Tool-reported execution errors
bypass the success schema and return to the provider as results.

Text tool conversations preserve text, resource links and embedded text in a JSON envelope
with `content`, optional `structuredContent` and `isError`. Binary content is refused as
`unsupported_tool_content`. Resource references are not fetched. Host metadata is retained
in the completion event, excluded from provider projections and logger events, and charged
against payload limits. Debug formatting excludes metadata values.

`ToolCallCompleted.ok` reports tool success. A tool-reported error emits `ok: false` with
its result and allows the provider conversation to continue. Executor and result-projection failures
emit `ok: false` and stop the conversation.

## Outcome evidence

`ToolCallEvidence` records the call ID, tool name, optional remote request ID and one outcome:
`not_dispatched`, `completed` or `uncertain`. Tool-loop and registry refusals before execution are
`not_dispatched`. A returned result is `completed`, including tool errors and results that
fail output validation or provider delivery. An executor failure without explicit evidence
is `uncertain`. A deadline while awaiting an executor is also `uncertain`.
An oversized call batch is refused as `tool_transcript_too_large`, with `not_dispatched`
evidence identifying its first call. The whole batch is refused before execution.

Completion evidence remains available until the following provider turn succeeds. A failed
or timed-out continuation retains the latest completed call's evidence; each call's completion
event retains its own evidence.
Stream idle and request-deadline watchdogs retain the latest observed tool evidence; a started
call without a completion event remains uncertain.

Evidence about a tool survives classification of the enclosing provider request. Completion
does not prove durable publication or authorise replay; journal intents and host reconciliation
govern recovery. Dropping a stream cancels pending execution and may leave a started call
without a completion event.

## Host approval

`ApprovedToolExecutor` pins a profile's registry, host policy, run identity, binding revision
and absolute deadline. Validation precedes policy evaluation. The policy receives
the complete tool record and canonical call, including validated arguments. The request
revision binds the run, host binding revision, record and call. Retained host decisions must
match that revision. Tool effect annotations remain author claims; the host determines trust.

The policy returns an available `Allow`, `Deny` or `Pending` decision without waiting for user
interaction. Denied, pending and expired calls stop the provider conversation with typed tool
codes and `not_dispatched` evidence. Allowed calls are dispatched through the registry and retain
its result validation. A deadline during execution reports uncertainty.

Every validated call ID reserves capacity before policy evaluation. Clones share reservations;
duplicate IDs, including altered calls, are refused. Denial, pending, cancellation and completion
retain the reservation. Capacity is finite and reservations are never evicted. Approval request
payloads use the shared 64 MiB limit. Provider continuation retries reuse the recorded result
without consulting approval or invoking the executor again.

The host supplies a distinct scope for each tool conversation. Its binding revision covers the
registry's revision, the grant and the credential scopes. It retains the executor
for the conversation's lifetime. Reconstructing an executor starts another reservation set;
interrupted runs require host reconciliation before replacement. Pending approval stops before
dispatch.


## Durable approval

`DurableToolExecutor` records pending calls in a host-owned `ToolApprovalStore`. Each journal
belongs to one conversation: its scope pins the run, binding revision and absolute Unix expiry.
The binding revision covers the registry's revision, credentials and grant. Each request digest
binds that scope, the complete presented record and the canonical call. A stored record that
differs from the one the registry presents is refused before dispatch.
Opening requires the same scope and never extends expiry. In-process deadlines are monotonic;
the host supplies a trusted wall clock across restarts. Clock regression behind the journal's
last update rejects mutation.

The host store provides bounded reads and atomic, durable compare-and-exchange across processes.
Successful writes survive restart. Errors, timeouts and cancelled writes may have committed;
inspect storage before issuing another command. Loading validates the journal without repairing
or upgrading it. Unsupported versions, changed identities and invalid state are refused.
Journal revision, call count and encoded byte limits are finite. Definitions, arguments, results,
host metadata and reconciliation evidence all count towards retention.

The command sequence is explicit:

1. `call` or `submit` validates and persists a pending request. `call` returns
   `tool_approval_pending` with `not_dispatched` evidence and stops the provider loop.
2. The host authenticates an actor and calls `decide` with the request digest and expected journal
   revision. Allow, deny and cancel decisions are durable before acknowledgement. Cancellation
   also accepts an approved call that has not acquired a dispatch intent.
3. `resume` requires an approved call and the expected journal revision. It revalidates the pinned
   definition and arguments, persists an intent, checks expiry, then invokes the executor.
4. A known result or failure is persisted before returning it. Explicit resumption of a published
   outcome returns the retained outcome, including host metadata, without another dispatch.

Existing call IDs cannot be replaced. Stale commands fail with `tool_approval_conflict`.
Repeating the same decision by the same actor at the current revision is idempotent. Pending,
denied, cancelled and expired calls never dispatch. Completed results remain readable after expiry.
One journal dispatches one call at a time; an outstanding intent blocks other call mutations.
Separate journals require distinct conversation identities.

Cancellation, uncertain execution or failed result publication may leave an intent. `resume`
refuses that intent as `tool_approval_unresolved`. Before `reconcile`, the host fences the original
executor and inspects the effect at its destination. Reconciliation records an actor and evidence:
confirmed not-applied restores approval, completed retains a validated host-supplied result, and
abandon terminates the call. Expiry still forbids dispatch after not-applied reconciliation.
A journal revision fences writes; execution ownership requires host fencing.

Explicit tool resumption returns the retained result to the host. The host copies that exact call
and result into its [durable provider conversation](providers.md#durable-tool-conversations), then
explicitly resumes the provider turn. The approval and conversation stores have independent
revisions; a restart between result publication and copying retrieves the published tool result.
Resubmitting generation cannot reconstruct an interrupted conversation.

A journal is retained until it is settled and its run has ended; see
[retention](recovery.md#retention).
