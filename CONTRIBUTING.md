# Contributing

Run `cargo xtask check` for Rust formatting, dependency boundaries, manifest consistency, tests
and documentation. Run `cargo xtask package` to compile packaged crates from an independent
consumer. These commands need only Rust and the operating system tools used by Cargo.

For the handbook, API references, and documentation checks, see [documentation delivery](docs/README.md).

VS Code tasks are convenience shortcuts to these Cargo, xtask, and npm commands. **Build: All**
is the default build task (`Cmd+Shift+B` on macOS, `Ctrl+Shift+B` on Windows/Linux). Use
**Tasks: Run Task** for Rust and TypeScript builds, tests and validation; package and API checks;
bindings checks or generation; documentation; and example applications. The Rust package tasks
prompt for a workspace package. Composite tasks run their steps sequentially.

**Example: Initialize Editor** and **Example: Serve Editor** prompt for an application data
directory. Initialization is explicit; serving requires an initialized directory. The standalone
editor web build/dev tasks use the release packages pinned by that sample's manifest. Use
**Example: Verify Editor Integration** to exercise freshly packed workspace packages instead.
Install the relevant dependencies and pinned tools before running their shortcuts. Build policy
continues to live in Cargo, xtask, and npm scripts; CI does not use the VS Code task configuration.

In `typescript/`, install the lockfile with `npm ci`, then run `npm run check` and
`npm run package:check`. The latter installs the packed artefacts in an isolated consumer.

Build `editor-server-example` with Cargo, then run `npm run test:example` in `typescript/` for the
installed-package browser and CLI contract checks. They use an isolated application directory.

Public Rust APIs are recorded by `cargo-public-api` 0.52.0 using `nightly-2026-10-01`; the pinned
toolchain is only needed for the API gate. `cargo xtask api` checks the snapshots. TypeScript's
API record is the declaration output checked by its own gate. Follow tool help when deliberately
updating a snapshot. See the [upstream API tool](https://github.com/cargo-public-api/cargo-public-api)
for its toolchain contract.

For co-development, use uncommitted Cargo path patches and locally packed npm tarballs in the
consumer. Committed Primrose dependencies must name one published Battersea release.

## Debugging in VS Code

Install the recommended rust-analyzer and CodeLLDB extensions for Rust debugging. The first
launch, **Rust: Custom Node**, runs the independent handler example. **Rust: Catalogue** and
**Rust: Mock Backend** cover validation and mock provider/tool/media execution. Cargo resolves
the debug executable for each launch. Use rust-analyzer's **Debug** actions for individual Rust tests.

For **Rust: Editor Server**, run **Example: Initialize Editor** once with a new application
directory, then supply that directory to the launch prompt. Launching the server preserves its
saved documents and activation records. **Rust: Editor CLI Activation** prompts for the running
server URL, flow key, entry node, and input text. **Rust: Attach to Process** lets you select an
existing process; stop/disconnect the attach session without terminating a server you want to keep.

Open a `.test.ts`, `.test.tsx`, or `.test.mjs` file and select **TypeScript: Current Test** to build
the workspace packages and debug that file with Node and the existing `tsx` loader. The launch
attaches to Node's test worker processes. Source breakpoints in TypeScript tests work through
`tsx`; the package build currently emits no source maps, so stepping into package implementations
uses their generated JavaScript in `dist/`.

**Browser: Editor Sample** uses VS Code's built-in Chrome debugger. Install dependencies in
`examples/editor-server/web/`, start the editor API separately if needed, then launch the browser.
Its pre-launch task starts Vite and waits for port 18181 to be ready; a busy port fails explicitly.
After debugging, terminate **Example: Dev Editor Web** through **Tasks: Terminate Task** when
you no longer need it. The browser launch debugs the sample and its manifest-pinned release
packages. It does not substitute current workspace sources. A full-stack compound launch can
follow when the sample explicitly uses current packages and the library build supplies source maps.
