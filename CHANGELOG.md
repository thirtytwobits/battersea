# Changelog

## Unreleased

- Require an explicit upgrade to version 2 with port modes, source order and finite limits.
- Queue ordered token fan-out and control delivery with lossless admission and explicit lossy drops.
- Consume streaming deltas once and wait for successful closure before final execution.
- Drive owned provider pumps with cancellation, fair selection and retained-payload accounting.
- Support token fan-out editing and exercise durable save/reload in the independent editor.

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
