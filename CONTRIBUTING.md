# Contributing

Run `cargo xtask check` for Rust formatting, dependency boundaries, manifest consistency, tests
and documentation. Run `cargo xtask package` to compile packaged crates from an independent
consumer. These commands need only Rust and the operating system tools used by Cargo.

In `typescript/`, install the lockfile with `npm ci`, then run `npm run check` and
`npm run package:check`. The latter installs the packed artefacts in an isolated consumer.

Public Rust APIs are recorded by `cargo-public-api` 0.50.2 using `nightly-2025-08-02`; the pinned
toolchain is only needed for the API gate. `cargo xtask api` checks the snapshots. TypeScript's
API record is the declaration output checked by its own gate. Follow tool help when deliberately
updating a snapshot. See the [upstream API tool](https://github.com/cargo-public-api/cargo-public-api)
for its toolchain contract.

For co-development, use uncommitted Cargo path patches and locally packed npm tarballs in the
consumer. Committed Primrose dependencies must name one published Battersea release.
