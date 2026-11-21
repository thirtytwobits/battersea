# Failures

Status: contract.

Battersea does not delete diagnostic data. What a provider, a tool or a Battersea component
reports about a failed or incomplete operation reaches the host intact. Battersea adds its own
context to a failure and neither replaces nor drops what it received. Resolved credentials are
masked.

## Failure

Every operation that fails or ends incomplete reports one `Failure`.

| Part | Content |
|---|---|
| Kind | Battersea's classification |
| Message | Battersea's description, in fixed wording |
| Context | The values Battersea names in its description, each under a fixed name |
| Reporter | The provider, tool or Battersea component that reported the failure |
| Origin | The operation, provider, backend and model |
| Exchange | What is known of the remote exchange: dispatch certainty, attempt count, HTTP status, retry delay and the identifiers the provider issued |
| Watchdog | The limit an idle watchdog enforced |
| Tool code | The tool runner's name for a refusal |
| Tool call | `ToolCallEvidence` about a nested tool call |
| Native detail | The reporter's own account, whole |
| Partial output | Output received before the failure and not delivered |
| Cause | The failure or foreign error that this failure wraps |
| Cleanup | The failure of the cleanup Battersea attempted after the operation failed |

Native detail is a provider's error body, stream error event or terminal event, or a tool's error.
Battersea carries native detail as the structured value it received and selects no fields from
it; a body that is not JSON is carried as text. Kind is derived from native detail and status.
Refusal and truncation have the kinds `refusal` and `incomplete`. A stream error event takes the
kind of its provider error type, as an HTTP rejection takes the kind of its status.

Every failure of an operation carries that operation's origin, whichever component raises the
failure. A failure raised after a provider's response headers arrive carries the identifiers the
provider issued; this includes stream errors, decoding failures and completion validation.

Battersea's message is the same text for every failure raised at one site. A value that
distinguishes one such failure from another, such as a node, an edge or a limit, is in the
context. The message and the context contain no text written by the reporter. Reporter text is in
native detail or in the cause.

A failure that wraps another failure carries it as its cause. A wrapped error of another type is
a foreign error, carried as its text and the text of each of its sources in order. No failure
site discards the error it handles.

A failed operation whose cleanup also fails, such as the cancellation of a remote job, reports
the operation's failure and carries the cleanup's failure whole.

Partial output is output Battersea held when the operation failed: buffered structured text and
incomplete content blocks. Events already delivered remain with their consumer.

A failure is bounded by the limits of what it carries: the provider payload limit and the output
limits of its operation.

## Termination

A turn's terminal event carries a class and the provider's terminal payload. The class is
`complete`, `tool_use`, `length`, `refusal` or `other` and decides control flow. A native reason
that Battersea does not recognise has the class `other`. A final turn of any class but `complete`
fails with the terminal payload as its native detail.

## Credentials

Resolved credentials are masked in every part of every failure, in their raw, URI-encoded,
form-encoded and JSON-escaped forms. Masking replaces the credential and keeps the content around
it. Nothing else is removed from a failure the host receives.

## Disclosure

The host receives every failure complete. An operation the host calls returns its failure. The
driver passes every node failure to the host's failure observer, with its `node_failure`
identity, before applying the node's failure policy. A failed observer call fails the activation.

The host chooses what the flow receives. `FailureDisclosure` is `full` or `withheld`; omission
means `full`. Under `withheld`, native detail, partial output and foreign error text are marked
withheld at every depth of the cause chain, and in a cleanup failure, in the `node_failure` token, the checkpoint and the
runtime records. The context is not withheld. A withheld part is distinct from an absent part. The `node_failure` identity
joins a withheld failure to the complete failure the observer received.

Under `full`, the token and the checkpoint carry the complete failure. Runtime records treat
native detail, partial output and foreign error text as content under
[the observation contract](observability.md).

## Required proof

| Area | Proof |
|---|---|
| Adapters | For each registered adapter and each recorded failure of that adapter (HTTP rejection, stream error event, refusal, truncation, block, malformed event), every leaf of the provider's payload is present in the failure |
| Identity | A decoding failure, a stream error and a completion-validation failure after the response headers each carry the issued identifiers and the origin |
| Termination | Distinct native reasons of one class remain distinct in the failure; an unrecognised reason has the class `other`; a truncated or refused conversation turn has the kind `incomplete` or `refusal` and carries the provider's response |
| Partial output | A refusal under a response format carries the buffered text; a stream that fails, ends, stalls or passes its deadline with content blocks open carries those blocks |
| Cause | A wrapped foreign error's text and its sources' text are present in order |
| Cleanup | A failed remote cancellation's rejection body is present in the failure the host receives, and withheld with it |
| Credentials | A credential in each encoded form inside a provider payload is masked and the rest of the payload is unchanged |
| Disclosure | Under `withheld`, the token, the checkpoint and the runtime records hold no native detail, partial output or foreign error text, and the observer received the complete failure under the same identity; under `full` the token holds them |
| Observer | The observer is called before the policy applies; its failure fails the activation |
| Kits | A kit's provider failure carries the adapter's failure unchanged |
| Message | A failure constructor accepts no message built at run time |
| Discarded errors | `cargo xtask check` refuses an error discarded without a stated reason |
