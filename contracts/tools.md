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

Evidence about a tool survives classification of the enclosing provider request. Completion
does not prove durable publication or authorise replay; journal intents and host reconciliation
govern recovery. Dropping a stream cancels pending execution and may leave a started call
without a completion event.

The host-controlled approval boundary belongs between argument validation and executor
dispatch. Run identity, definition and binding revisions, approval decisions and durable
continuation belong to that shared boundary. Tool outcome evidence describes execution;
it does not grant authority to execute.
