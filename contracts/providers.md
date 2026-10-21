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

`DurableConversation` checkpoints completed provider turns through a host-owned
`ConversationStore`. Chat Completions supports this interface for ordinary and streaming
responses. Other adapters return `unsupported_tool_continuation` from the default interface.

Acceptance pins the original request, tool definitions, provider binding, safe backend
configuration digest, tool binding revision and absolute expiry. The host includes credential
and authorisation context in the tool binding revision. Expiry must fit the configured backend
operation timeout. Reopening requires the same request, scope and binding; remaining time and
provider-attempt capacity survive restart. The host supplies a trusted wall clock across restarts.

The host store bounds reads before decoding and atomically compares journal revisions across
processes. Successful writes are durable. Failed or cancelled writes may have committed, so the
next command inspects storage. Reads validate version, integrity, payload limits and transcript
compatibility without upgrading stored values. The checksum detects accidental modification;
the host owns storage integrity and access control.

Each `resume` command performs at most one provider turn. A dispatch intent is persisted before
provider I/O. A completed turn, its adapter-owned transcript, complete validated call batch and
buffered events are then published together. Pending calls become visible after that publication.
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
Final output receives the same modality and structured-output validation as an ordinary response.
Completed conversations return their retained checkpoint, including after expiry.

An interrupted provider task, ambiguous response or failed turn publication can leave an intent.
`resume` refuses that intent as `tool_continuation_unresolved`. After fencing the original task,
the host can reconcile a retained completed turn, confirm that dispatch did not occur, or abandon
the conversation. The host coordinates cancellation of approval work when abandoning a conversation.
Reconciliation records an actor and evidence and preserves expiry and consumed
attempts. A timeout or missing reply does not establish that dispatch did not occur.

Provider intents, including explicitly reconciled attempts, consume the request's finite turn
budget. The complete checkpoint, including native transcripts, events, results, metadata and
reconciliation evidence, counts towards its byte limit. The shared batch limit is 256 calls per
durable turn. Stale revisions, changed bindings, incompatible codecs and corrupt state stop
before provider I/O. A durable provider turn never invokes a tool executor.
