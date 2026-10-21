# Application tools

## Records

`battersea_tools::Tool` is the Model Context Protocol 2026-07-28 tool record, named by
`MCP_PROTOCOL_REVISION`. Its fields keep the protocol's spelling, `_meta` included.
`tools.schema.json` publishes it, and `@battersea/flow` carries its TypeScript types.

A tool's annotations are its side-effect class. Each hint is a claim of the tool's author.
`Tool::effect` reads every hint at its protocol default: an unannotated tool may change and
destroy, is not idempotent, and reaches outside its own domain.

## Profiles

A tool lists the agent profiles permitted to call it under
`_meta["io.github.thirtytwobits.battersea/profiles"]`, `PROFILES_META_KEY`. A tool that lists
none is permitted to none. The application names its profiles and chooses each run's profile.

## Registry

`ToolRegistry::register` admits a record Battersea can offer to a provider. It refuses with a
`ToolCode`:

| Record | Code |
|---|---|
| Name outside 1 to 128 ASCII letters, digits, `_`, `-` and `.` | `invalid_tool_definition` |
| No description | `invalid_tool_definition` |
| Input schema whose root is not an object schema | `invalid_tool_definition` |
| Profiles that are not distinct, non-blank names | `invalid_tool_definition` |
| Input or output schema that does not compile | `invalid_tool_schema` |
| Provider-facing name already registered | `duplicate_tool` |

A provider is offered each tool under its provider-facing name, `local_tool_name` of its own,
with its description exactly as written.
`ToolRegistry::for_profile` keeps the tools permitted to one profile. A call names its tool by
either name. The registry refuses a call to a tool it does not hold as `unregistered_tool`, and
arguments outside the tool's input schema as `invalid_tool_arguments`, before any executor runs.
The executor receives the call under the tool's own name.

## Results

`EngineLocalToolResult::json` retains ordinary application JSON. A content result carries
ordered blocks, optional structured output, an execution-error flag and host metadata.
Where a tool declares an output schema, the registry validates successful application JSON
or the content result's structured output. Missing or invalid structured output is refused
as `invalid_tool_result`. Tool-reported execution errors bypass the success schema and
return to the provider as results.

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

`ApprovedToolExecutor` pins a profile-filtered registry, host policy, run identity, binding
revision and absolute deadline. Validation precedes policy evaluation. The policy receives
the complete tool record and canonical call, including validated arguments. The request
revision binds the run, host binding revision, record and call. Retained host decisions must
match that revision. Tool effect annotations remain author claims; the host determines trust.

The policy returns an available `Allow`, `Deny` or `Pending` decision without waiting for user
interaction. Denied, pending and expired calls stop the provider conversation with typed tool
codes and `not_dispatched` evidence. Allowed calls enter the registered executor and retain
the registry's result validation. A deadline during execution reports uncertainty.

Every validated call ID reserves capacity before policy evaluation. Clones share reservations;
duplicate IDs, including altered calls, are refused. Denial, pending, cancellation and completion
retain the reservation. Capacity is finite and reservations are never evicted. Approval request
payloads use the shared 64 MiB limit. Provider continuation retries reuse the recorded result
without consulting approval or invoking the executor again.

The host supplies a distinct scope for each tool conversation and includes all selected tool
bindings and credential scopes in its binding revision. It retains the executor
for the conversation's lifetime. Reconstructing an executor starts another reservation set;
interrupted runs require host reconciliation before replacement. Pending approval stops before
dispatch.


## Durable approval

`DurableToolExecutor` records pending calls in a host-owned `ToolApprovalStore`. Each journal
belongs to one conversation: its scope pins the run, binding revision and absolute Unix expiry.
The binding revision includes selected tool bindings, credentials and authorisation context.
Each request digest binds that scope, the complete tool definition and the canonical call.
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
