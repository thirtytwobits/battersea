# Provider conversations

Status: Implemented in v0.8.0.

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

Errors retain dispatch certainty, HTTP status, request identity and retry delay. Retry observations
identify the attempt and delay. Unknown remote outcome remains unknown for spend accounting.

## Content

Messages retain ordered content, role and tool identity. System messages precede the conversation
and contain text. Media input uses data URLs or explicit provider-readable references, never
implicit local file reads. Provider-native assistant blocks belong to one provider and retain
opaque fields and signatures. Cross-provider replay fails. Tool results match earlier calls.

Transport capabilities are checked against each configured model's explicit capabilities:

| Transport | Input | Output | Native output formats |
|---|---|---|---|
| OpenAI Responses | Text, image, document | Text | JSON object, JSON Schema |
| Anthropic Messages | Text, image, PDF | Text | JSON Schema |
| Gemini GenerateContent | Text, image, document, audio, video | Text, image, audio | JSON object, JSON Schema |
| Mock | Text | Text | JSON object, JSON Schema |

Counting and generation encode the same conversation. Unsupported modalities, content roles,
MIME types, provider references and output modes fail before provider I/O. Backend capability
summaries expose provider-neutral declarations. Model support is configuration, not inferred
from a provider name. Pricing remains the separately versioned price catalogue.

Structured output is requested through the native API. Its JSON Schema is validated before
sending, without remote schema fetches. A completed final turn must parse and satisfy that schema
before publishing a structured result. Refusal, incomplete output and invalid JSON fail; tool-turn
text cannot contaminate the final result. Buffers obey the shared payload bound. Prompted JSON
requires an application-authored prompt and is not a fallback for unsupported native output.

[Media jobs](media-jobs.md) cover asynchronous generation. Document format upgrades require explicit commands.
