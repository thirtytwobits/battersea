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
