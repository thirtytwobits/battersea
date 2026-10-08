# Changelog

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

This release is the M1 scaffold. The flow contract is M2 work.
