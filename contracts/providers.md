# Provider conversations

## Configuration

`ProtocolRegistry` registers implementations by adapter ID and contract version.
`ProviderConfiguration` version 1 declares service identities with independent chat and media
bindings. Backends select a service, model, explicit capabilities and adapter options.
Validation checks every declaration before credential resolution or adapter construction.
Unknown IDs, duplicate registrations, incompatible operations, unsupported parameters and
invalid options fail with `ProviderCode` and a declaration path.

Binding resolves each used service operation's credential once and constructs immutable adapters.
The validated configuration retains its adapter registrations. The host retains the configured
registry for the operation's lifetime; rebinding creates a separate generation. Construction
performs no provider I/O.

An HTTP binding's `baseUrl` is an API directory, including any proxy prefix and API version.
Protocol routes append beneath that directory. Authentication explicitly selects none, bearer,
header or query delivery. Secrets come from environment or host references. Literal headers hold
non-secret protocol settings. Snapshots, debug formatting, logs and errors exclude resolved
credentials. Account changes require a new host-supplied credential scope revision.

The safe snapshot and configuration revision belong with the accepted run. Hosts verify the
revision before resume and retain the accepted registry while remote jobs remain active.
Parsing and inspection preserve stored values. `upgrade_legacy_providers` requires explicit
backend-to-service assignments; `upgrade_provider_content` requires the host's retained origin
evidence. Hosts own persistence of the converted copy.

## Dispatch

Each HTTP attempt sends one immutable prepared request. Only a connection failure known to precede
HTTP dispatch or an HTTP 429 rejection permits another generation attempt. A timeout, connection
loss after dispatch, server failure, malformed response or interrupted ordinary stream is
ambiguous and cannot replay generation. Configuration, authentication and protocol errors stop.
Read-only response retrieval may retry transport failures without creating another generation.

Backoff uses explicit initial and maximum delays and a finite retry count. Retry-After is a lower
bound; a delay beyond the configured maximum stops instead of retrying early. One operation
deadline covers attempts, backoff and tool turns. Dropping the request cancels pending work.
Background-response cancellation retains ownership of its cleanup request when its caller is
dropped, with one cancellation attempt per response guard.
Retries do not invoke the tool runner. A tool result is appended once; a rejected continuation
resends that exact body. Background recovery retrieves the same response ID after the last
accepted sequence, discarding duplicates and never moving the cursor backwards.

Errors retain dispatch certainty, HTTP status, request identity and retry delay. A local tool fails
after the provider turn that called it, so its failure never reports the request as not sent. Retry observations
identify the attempt and delay. Unknown remote outcome remains unknown for spend accounting.
Nested tool errors retain separate `ToolCallEvidence`; changing provider dispatch certainty
does not change the tool's recorded outcome. Binding error redaction covers nested tool
request IDs, including encoded credentials.

## Content

Messages retain ordered content, role and tool identity. System messages precede the conversation
and contain text. Media input uses data URLs or explicit provider-readable references, never
implicit local file reads. Provider-native assistant blocks belong to one provider and retain
opaque fields and signatures. Versioned native and reasoning blocks identify the service, adapter
and binding revision. Cross-binding replay fails before I/O. Tool results match earlier calls.

Transport capabilities are checked against each configured model's explicit capabilities:

| Transport | Input | Output | Native output formats |
|---|---|---|---|
| OpenAI Responses | Text, image, document | Text | JSON object, JSON Schema |
| OpenAI Chat Completions | Text, image | Text | JSON object, JSON Schema |
| Anthropic Messages | Text, image, PDF | Text | JSON Schema |
| Gemini GenerateContent | Text, image, document, audio, video | Text, image, audio | JSON object, JSON Schema |
| Mock | Text | Text | JSON object, JSON Schema |

Adapters declare native, estimated or unavailable input counting. Chat Completions declares
counting unavailable and supports ordinary and streaming completions. A service can disable
counting or restrict its adapter's content and parameter capabilities.

Counting and generation encode the same conversation where counting is supported. Unsupported modalities, content roles,
MIME types, provider references and output modes fail before provider I/O. Backend capability
summaries expose provider-neutral declarations. Model support is configuration, not inferred
from a provider name. Pricing remains the separately versioned price catalogue.

Structured output is requested through the native API. Its JSON Schema is validated before
sending, without remote schema fetches. A completed final turn must parse and satisfy that schema
before publishing a structured result. Refusal, incomplete output and invalid JSON fail; tool-turn
text cannot contaminate the final result. Buffers obey the shared payload bound. Prompted JSON
requires an application-authored prompt and is not a fallback for unsupported native output.

[Media jobs](media-jobs.md) cover asynchronous generation. Document format upgrades require explicit commands.


## Durable tool conversations

`DurableConversation` checkpoints provider turns through a host-owned `ConversationStore`.
Chat Completions supports ordinary and streaming responses. OpenAI Responses supports foreground
streaming and background polling with `contextOverflow=error`. Background streaming returns
`unsupported_tool_continuation`: recovery requires a retained event cursor.
Other adapters return `unsupported_tool_continuation` from the default interface.

Acceptance pins the original request, tool definitions, provider binding, safe backend
configuration digest, tool binding revision and absolute expiry. The host includes credential
and authorisation context in the tool binding revision. Expiry must fit the configured backend
operation timeout. Reopening requires the same request, scope and binding; remaining time and
provider-attempt capacity survive restart. Expiry before dispatch returns `ErrorKind::Expired`
with `DispatchState::NotSent`. The host supplies a trusted wall clock across restarts.

The host store bounds reads before decoding and atomically compares journal revisions across
processes. Successful writes are durable. Failed or cancelled writes may have committed, so the
next command inspects storage. Reads validate version, integrity, payload limits and transcript
compatibility without upgrading stored values. The checksum detects accidental modification;
the host owns storage integrity and access control. Shared checkpoint version 2 includes pending
remote identities and cancellation receipts. Version 1 records remain readable and retain their
version when resumed.

Each `resume` command performs one generation or one retrieval of a pending response. A dispatch
intent is persisted before
provider I/O. A completed turn, its adapter-owned transcript, complete validated call batch and
buffered events are then published together. The adapter verifies that the ordered call batch
and output events match its retained provider response before publication, including reconciliation,
and when reopening retained turns. Each transcript must extend the preceding durable transcript
with the exact ordered provider projections of its recorded tool results and one completed provider response.
Chat Completions checkpoint codec version 2 retains native token usage with that response.
Pending calls become visible after publication.
Provider diagnostics can precede publication; checkpoint events carry the durable observations.
Identify checkpoint events by run, provider attempt and event index when consuming them after reopen.

The host submits pending calls to its approval journal, explicitly resumes approved calls, then
copies each completed outcome with `record_tool_result`. Call identity, name and arguments must
match the saved batch. Repeating an identical result at the current revision is idempotent;
a changed result is refused. Results may arrive separately and are supplied to the provider in
its original call order. Host metadata remains in the checkpoint and is excluded from provider
projections. `resume` waits for the complete batch before sending its continuation.

Chat Completions retains its native assistant envelopes and appends each tool result once. An
HTTP 429 retry sends the same prepared request. Reopening after a tool decision or result copy
uses the retained assistant turn; it never regenerates the request that selected the tools.
Retained assistant responses must have an absent or null refusal field.
Final output receives the same modality and structured-output validation as an ordinary response.
A final turn with a requested response format retains exactly one matching `StructuredOutput` event.
Completed conversations return their retained checkpoint, including after expiry.

Responses checkpoint codec version 1 retains the exact input and native response.
Continuation appends every output item, including opaque reasoning and function-call fields,
then the ordered `function_call_output` projections. Requests include encrypted reasoning for
host-owned history. Durable text, reasoning, native content and usage events are derived from the
completed response; partial stream observations remain diagnostics. Refusals, incomplete responses
and unsupported output items are rejected before publication and on recovery.

An interrupted provider task, ambiguous response or failed turn publication can leave an intent.
`resume` refuses that intent as `tool_continuation_unresolved`. After fencing the original task,
the host can reconcile a retained completed turn or pending remote identity, confirm that dispatch
did not occur, or abandon
the conversation. The host coordinates cancellation of approval work when abandoning a conversation.
Reconciliation records an actor and evidence and preserves expiry and consumed
attempts. A timeout or missing reply does not establish that dispatch did not occur.

### Background polling

`ConversationCommand` distinguishes generation, retrieval and cancellation. `ConversationStep`
returns pending state, a completed turn or terminal cancellation evidence. Adapters validate each
transition against the original request, preceding transcript and ordered tool results. The default
adapter implementation supports completed generation through `conversation_turn`.

A queued or in-progress Responses create publishes `ProviderPending` with an immutable native
response identity. Reopening retains that identity and the exact submitted input. Each subsequent
`resume` retrieves the same response after the configured poll interval. A pending retrieval
preserves the original handle; a completed retrieval publishes the validated turn and retains its
handle alongside the transcript. Failed or interrupted retrieval can be retried at the stored
revision. Retrieval never authorises a new generation. Failure before the create result is published
leaves `ProviderIntent` and requires host reconciliation.

`cancel` publishes `ProviderCancelling` before requesting remote cancellation. Generation and
retrieval remain blocked until cancellation returns a matching terminal response. The terminal
receipt is published with `Abandoned`; repeating that command returns the saved receipt. Interrupted
cancellation can be retried against the same remote identity. Cancellation is bounded by the
configured timeout and remains available after conversation expiry. The host owns remote cleanup:
dropping a conversation leaves the remote job running, and host abandonment preserves its handle.
The host also coordinates outstanding tool approval work.

Background expiry must fit both the backend operation timeout and background wait limit.
Retrieval preserves absolute expiry and consumes no provider-attempt capacity. Remote retention
and `storeResponse` remain provider and host policy; an unavailable response causes a retrieval
error. Recovery does not replace an unavailable response with a new generation.

Provider generation intents, including explicitly reconciled attempts, consume the request's finite turn
budget. The complete checkpoint, including native transcripts, events, results, metadata and
reconciliation evidence, counts towards its byte limit. The shared batch limit is 256 calls per
durable turn. Stale revisions, changed bindings, incompatible codecs and corrupt state stop
before provider I/O. A durable provider turn never invokes a tool executor.
