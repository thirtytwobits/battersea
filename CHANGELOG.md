# Changelog

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
