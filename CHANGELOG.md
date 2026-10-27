# Changelog

## 0.20.0 — 2026-10-27

- Take [Clapham](https://github.com/thirtytwobits/clapham) v0.3.2. A host that depends on Clapham
  pins the `v0.3.2` tag; Cargo treats each Git tag as a separate `clapham` crate, so the types
  `battersea-tools` and `battersea-ops` accept come from that tag alone.
- The MCP daemons of `examples/editor-server` and `examples/ops` answer every `resources/read`
  with JSON-RPC `-32602` and the URI under `data.uri`, where they answered `-32002`. Neither
  registers a resource template.
- Public Rust APIs, `operation.schema.json` and stored formats are unchanged.

## 0.19.1 — 2026-10-26

- `runtime.schema.json` no longer defines `FlowFailurePolicy`. The node state's `failure_policy`
  writes the flow contract's failure policy in place, so a host composes the published schema
  bundles into one document again. `@battersea/flow` exports `FlowFailurePolicy` from the flow
  contract alone.
- The bindings generator refuses published schema bundles that define the same name.
- Public Rust APIs and stored formats are unchanged. See PR #44.

## 0.19.0 — 2026-10-26

- `@battersea/editor` accepts `react` and `react-dom` 18.3 or 19, and `react-amnesia` 0.2 or
  0.3. Its published declarations are the same under React 18 and React 19 types.
- `@dnd-kit/core` 6.3.1 declares its components with the global `JSX` namespace that React 19's
  types removed. A React 19 application type-checks those declarations only with
  `skipLibCheck`.
- Rust crates, public Rust APIs and stored formats are unchanged. See PR #42.

## 0.18.0 — 2026-10-25

- Take [Clapham](https://github.com/thirtytwobits/clapham) v0.3.0, the operations contract of
  the London set. Every crate requires Rust 1.88.
- Breaking Rust API: `ToolRegistry::new` offers a selection of a Clapham composed set under the
  run's verified OAuth grant, and dispatches each call through `ComposedSet::call`. `register` and
  `Default` are removed; hosts contribute their tools as Clapham operations. Profiles are named
  selections, set with `with_profile` and narrowed with `for_profile`. `ToolRegistry::revision` is
  the selection's revision, which an approval scope's binding revision covers.
  `Tool::for_operation` presents a descriptor as Clapham's MCP server does.
- A refusal reaches the tool loop with its message and evidence. Clapham's `invalid_input`,
  `invalid_result` and `unknown_operation`, and any code spelled as a `ToolCode`, set
  `tool_code`; any other code, such as a remote `mcp_timeout`, is the classification unchanged.
  A registry result carries no host metadata, so a remote tool result's `_meta` no longer reaches
  the host.
- Remove `battersea-mcp`. Remote tools arrive as operations through Clapham's
  `clapham-mcp-client`, and `examples/mcp` runs on it. The MCP adapter contract is Clapham's.
- Add `battersea-ops`, which contributes Battersea's flow, run, approval and MCP-connection
  operations under the `battersea` namespace over the host capabilities the application
  supplies. See `contracts/operations.md`. `examples/ops` gives the same result for every
  operation through all seven of Clapham's front ends.
- `@battersea/flow` carries Clapham's `operation.schema.json` and its `Operations` types.
- `examples/editor-server` serves its flow and run operations at `/mcp` under local tokens and
  writes an endpoint record. Its CLI is Clapham's, built from the daemon's catalogue; the
  `activate`, `inspect`, `cancel`, `inspect-upgrade` and `upgrade` subcommands are removed.
- Stored formats are unchanged. Tool approval journals keep version 1, and a 0.17.0 journal reads
  back unchanged; its calls refuse to dispatch, because the registry presents a different tool
  record. See issues #31, #32 and #33, and PRs #38, #39 and #40.

## 0.17.0 — 2026-10-22

- Add per-node `fail`, `emit` and `skip` policies for explicitly classified recoverable
  failures. Handled failures preserve admitted outputs and settled effects while independent
  work and later ordered fan-out destinations continue. Omission preserves `fail` behaviour.
- Add catalogue-declared, runtime-owned `node_failure` outputs with bounded payloads and
  durable identities. Cancellation, capacity refusal, journal failure and unresolved effects
  retain their control outcomes.
- Retain failure settlement and dependency skip reasons in checkpoints. Recovery validates
  each stopped node's record and preserves error delivery without repeating settled effects.
- Expose failure policies in the editor with validation, undo/redo and save/reload support,
  including port-layout edits. Attribute handled failures separately from activation outcomes.
- Breaking stored-format change: flows use document version 3 through an explicit upgrade;
  execution and scheduler checkpoints use version 2. Older flows remain inspectable; older
  checkpoints refuse resume. Reads preserve stored bytes.
- Breaking Rust API: flow nodes carry `failure_policy`, node definitions declare
  `failure_ports`, and provider events include `RecoverableFailure`. Schemas, TypeScript
  bindings and API snapshots include these contracts. See issue #6 and PR #34.

## 0.16.0 — 2026-10-22

- Add `battersea-mcp`, a Model Context Protocol 2026-07-28 client over rmcp 3.6.0 with `stdio`
  and `http` transports. It discovers a server's tools within finite limits, binds the selected
  tools under aliases and runs them through the tool registry. It requires Rust 1.88; the core
  crates keep Rust 1.85. `examples/mcp` composes it in a host.
- Breaking Rust API: `EngineLocalToolResult` is an enum of `Json`, an application JSON result
  (`EngineLocalToolResult::json`), and `Content`, a `ToolContentResult` of ordered content
  blocks, structured content, an error flag and host `_meta`. `EngineAdapterRequestError` holds
  `request_id` as `Box<str>` and carries `tool_call`, the `ToolCallEvidence` of a nested tool
  call's outcome.
- Add tool approval. `ToolApprovalPolicy` and `ApprovedToolExecutor` ask the host before each
  call. `DurableToolExecutor` persists pending calls, decisions, dispatch intents and results
  through a host `ToolApprovalStore`, and resumes a call only when the host asks.
- Add durable provider conversations. `DurableConversation` checkpoints provider turns and tool
  results through a host `ConversationStore`, and resumes without regenerating a turn. Chat
  Completions supports it, and OpenAI Responses supports it with foreground streaming and with
  background polling and cancellation.
- State when each journal is no longer read: a recovery journal once
  `JournalRecord::is_terminal`; a tool-approval journal once `ToolApprovalJournal::is_settled`,
  and a conversation once `ConversationCheckpoint::is_settled`, each after its run has ended.
  See `contracts/recovery.md`, Retention.

## 0.15.1 — 2026-10-21

- The scheduler runs an instrument only when a signal reaches one of its action ports. It ran
  instruments as inline nodes, which failed any activation that kept one in its flow.

## 0.15.0 — 2026-10-19

- Add `battersea-tools`. `Tool` is the Model Context Protocol 2026-07-28 tool record, and a tool
  lists the agent profiles permitted to call it under `_meta`. `tools.schema.json` and the `Tools`
  types in `@battersea/flow` publish the record, and `toolProfiles` reads a tool's profiles.
- Breaking Rust API: `ToolRegistry` moves from `battersea-model` to `battersea-tools` and registers
  `Tool` records. It refuses a record it cannot offer as `ToolCode::InvalidToolDefinition`, keeps
  one profile's tools with `for_profile`, and passes each call to its executor under the tool's
  own name. A result outside the tool's declared output schema is refused as
  `ToolCode::InvalidToolResult`.
- A local tool's failure after a provider turn reports its dispatch as unknown, never as not sent.
- `NoLocalTools` refuses every call of a request offered no tools. The Chat Completions and mock
  adapters run with it when a request carries no executor.

## 0.14.0 — 2026-10-16

- Bind versioned provider configurations to registered protocol adapters with explicit service identities, endpoints, authentication and capability restrictions.
- Add a separate OpenAI Chat Completions adapter with streaming, structured output and local tool execution.
- Retain accepted adapter registrations and resolved credentials; expose safe snapshots and revisions for host-owned recovery.
- Bind native continuations and media jobs to their accepted service, protocol and account scope.
- Add explicit legacy configuration and continuation conversion, plus an independent declarative-provider example.
- Mask raw, URI/form-encoded and JSON-escaped credentials in provider errors and logs.
- Deliver versioned Rust and TypeScript API documentation with release archives and GitHub Pages publication.
- Breaking Rust API: backend structs carry an optional `ProviderBinding`, API keys use `SecretString`, prepared media carries a binding revision, and content has versioned native/reasoning variants. Legacy backend parsing remains explicit.

## 0.13.0 — 2026-10-16

- Breaking contract change: `ExecutionError`'s `invalid_request`, `cancelled`, `interrupted` and
  `recovery_required` take a reason code, which `reason_code` reads back. `internal` reports a
  fault and carries none.
- Declare the runtime's and generic nodes' codes in `battersea_runtime::error_codes`, listed by
  `error_codes::ALL`. `RecoveryError::code` and `PumpError::code` name journal and pump failures.
  A run that requires recovery keeps the Battersea code that named its cause.
- Breaking: `NodeError::InvalidRequest` carries a reason, read by `NodeError::reason_code`.
- Breaking: `SchedulerState::new` refuses a flow with the `FlowValidationIssue`s validation reports
  for it, with the same codes and node and edge ids. `validate_execution_contract` returns a
  `FlowValidationIssue`; another document version is `UNSUPPORTED_DOCUMENT_VERSION`.
- Breaking validity change: validation expands every node's dynamic action and signal ports and
  reports `INVALID_DYNAMIC_PORTS` for the node. A count parameter with no value or default now
  fails although no edge or alias touches the node.
- Tool admission, the tool registry and the tool loop name their refusals with a
  `battersea_model::ToolCode` in the new `EngineAdapterRequestError::tool_code` field; for
  example `ToolCode::UnofferedTool` for a call to a tool the model was not offered.

## 0.12.0 — 2026-10-15

- Breaking contract change: `FlowValidationIssue` requires a `code` naming the failure.
  Consumers branch on the code; the message is for people.
- Declare Battersea's codes in `battersea_flow::issue_codes`, listed by `issue_codes::ALL`.
  Parameter-value and dynamic-port failures keep their detail in the message.
- Graph rules registered with `Registry::register_graph_validator` supply the host's own codes.
- Add `FlowValidationIssue::new`, `with_node_id` and `with_edge_id`.

## 0.11.3 — 2026-10-15

- Carry the provider's own message into the failure of a refused HTTP request, for example
  `Provider HTTP request failed with status 401: invalid x-api-key`. Only the `error.message`
  field of the JSON error body is read.

## 0.11.2 — 2026-10-11

- Deliver provider events larger than the transport mailbox through bounded frames.
  Tool results arrive intact and in order; partial assembly and complete borrowed
  payloads remain charged to the node retention budget. Cancellation releases both.
- Apply backpressure until the handler releases each complete provider event.

## 0.11.1 — 2026-10-10

- Settle branches whose final-value input closes without a value, preserving independent completed outputs.
- Propagate successful absence through joins and sinks without invoking handlers or emitting activation signals.
- Preserve unresolved-dependency errors and checkpoint recovery across absence propagation.

## 0.11.0 — 2026-10-10

- Parse and export versioned flow source with lossless graph, execution policy and editor layout round trips.
- Reject unsupported versions, ambiguous identities and malformed source with location diagnostics.
- Derive node metadata from inline manifests using the shared catalogue parser and host registry.
- Validate definition/handler identity during code-defined registration; execute parsed flows in the independent custom-node application.
- Record the trust boundary and requirements for a separately designed sandbox ABI.

## 0.10.0 — 2026-10-10

- Capture versioned scheduler and host state at resumable driver boundaries, with pinned graph, catalogue, configuration and handler versions.
- Publish effect intents before driver segments and commit their results with checkpoints; retain interrupted segments for explicit resolution.
- Fence resume by revision and host ownership, preserving run identity and completed work.
- Separate host commit reconciliation from graph execution, including lost write acknowledgements.
- Preserve bounded queue accounting, closure state, temporary host resources and serialisable text buffers.
- Attribute execution observations by run, resume attempt and sequence.

## 0.9.0 — 2026-10-10

- Expose remote media snapshots, single polling operations, bounded retries, cancellation and repeatable output retrieval.
- Add Replicate image/video/audio predictions with explicit model input mapping and signed, idempotent callbacks.
- Move Runway onto the shared job lifecycle with strict identity/status validation and expiry-aware, bounded output downloads.
- Preserve synchronous provider completion and application-owned asset storage.
## 0.8.0 — 2026-10-10

- Preserve ordered multimodal conversations, native continuation signatures and provider capability declarations.
- Request native JSON output and validate completed results before delivery.
- Retry immutable HTTP requests only after known pre-dispatch failure or rate-limit rejection; keep ambiguous generation outcomes terminal.
- Preserve tool effects across continuation retries, bound backoff by the operation deadline, and report retry observations.
- Decode Gemini image and audio output and retain OpenAI and Anthropic continuation blocks.

## 0.7.1 — 2026-10-10

- Publish typed runtime snapshots, contiguous deltas and bounded retention independently of tracing.
- Export bounded OTLP GenAI spans and metrics, with opt-in masked content capture.
- Calculate attributed costs from complete usage and versioned price catalogues; atomically reserve concurrent budgets and reconcile ambiguous charges explicitly.
- Carry provider usage, per-turn billing and watchdog metadata through the shared model.
- Bound Pianola execution capture and report lossless-capture failures.
- Drive the independent editor from the runtime view with cursor resynchronisation.

## 0.6.0 — 2026-10-09

- Require an explicit upgrade to version 2 with port modes, source order and finite limits.
- Queue ordered token fan-out and control delivery with lossless admission and explicit lossy drops.
- Consume streaming deltas once and wait for successful closure before final execution.
- Drive owned provider pumps with cancellation, fair selection and retained-payload accounting.
- Support token fan-out editing and exercise durable save/reload in the independent editor.
- Author source priority, activation limits and connection capacity through shared editor controls.
- Bound provider HTTP parsing and tool payloads; backpressure asynchronous media reporters.

## 0.5.0 — 2026-10-09

- Publish an unstyled React editor with document, catalogue, validation and activation ports.
- Extract graph primitives, parameter renderers, undo, diagnostics and lazy layout engines.
- Preserve application metadata and other editor namespaces when saving flows.
- Publish generic Pianola grammar, lossless capture, registered assertions and grading hooks.
- Ship an independent editor/server/CLI example and Chromium/WebKit package-consumer checks.

## 0.4.0 — 2026-10-09

- Extract message contracts, open provider/tool registries and the shared tool runner.
- Add explicit provider features, typed admission errors, request deadlines and stream watchdogs.
- Preserve native tool continuation payloads, including signed reasoning.
- Add synchronous media results and cancellable remote jobs.
- Extract backend configuration/schema composition, generic node handlers and declared-root prompt scanning.
- Exercise mock chat, a registered tool and media from an independent packaged application.

## 0.3.0 — 2026-10-08

- Add the public node SDK, immutable executable catalogues and host execution contract.
- Extract token queues, source ordering, signal settlement and activation scheduling.
- Add typed execution events, accepted identities and terminal outcomes.
- Convert automation values through registered parameter contracts.
- Exercise an independent custom node and lifecycle failure cases against packaged crates.

## 0.2.0 — 2026-10-08

- Extract flow documents, node catalogues, ports, template contracts and graph validation.
- Register host handlers, nominal token and parameter schemas, and product validators.
- Inspect versions before graph fields; expose explicitly registered upgrade transformations.
- Preserve rendering policy and opaque editor state in canonical document round trips.
- Generate JSON Schema, TypeScript contracts and shared fixtures from Rust; export port
  compatibility through `@battersea/flow`.
- Validate an independent catalogue application against the packaged crate.

## 0.1.0 — 2026-10-08

- Adopt the MIT licence for the project and distributed packages.
- Establish the Cargo and TypeScript workspaces, dependency guards and package validation.
- Establish API snapshots and the draft GitHub release workflow.
