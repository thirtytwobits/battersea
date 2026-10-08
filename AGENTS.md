# Battersea agent instructions

The Cargo workspace owns the backend and repository tooling. `typescript/` owns npm packages.
Rust builds and tests must work without Node. Cargo-backed commands run sequentially.

Keep product sessions, storage policy, owned media and UI composition in the consuming application.
Public crates and npm packages must not depend on Primrose or Clerkenwell. A future optional
integration belongs in an explicit adapter package.

Use `cargo xtask --help` and `npm run` for the available gates. Add behaviour tests from contracts.
Record source commits, paths and copyright notices when extracting code. Read the relevant
contract before changing behaviour. Reads cannot upgrade stored formats.

`release.json` records publication decisions. Null values block publishing. Draft licence texts
are proposals, not grants. Keep package versions synchronised; review public API snapshots when
the contract changes. Registry publication is disabled; releases deliver tagged Cargo sources
and npm tarballs.
