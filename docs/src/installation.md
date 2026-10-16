# Install a release

Each release uses one version for Rust crates and npm packages. Registry publication is
disabled. Use a Git tag for Cargo and release tarballs for npm; do not install these
packages from crates.io or the npm registry.

The documentation build substitutes this release's version into the examples below.

```toml
[dependencies]
battersea-flow = { git = "https://github.com/thirtytwobits/battersea", tag = "v{{version}}" }
battersea-runtime = { git = "https://github.com/thirtytwobits/battersea", tag = "v{{version}}" }
```

Rust requires at least the `rust-version` declared in that release's Cargo manifest.
Node is not required for Rust builds or tests.

For a React application, install the matching release tarballs and the editor's peer dependencies:

```sh
npm install \
  https://github.com/thirtytwobits/battersea/releases/download/v{{version}}/battersea-flow-{{version}}.tgz \
  https://github.com/thirtytwobits/battersea/releases/download/v{{version}}/battersea-editor-{{version}}.tgz \
  'react@^18.3.1' 'react-dom@^18.3.1' '@xyflow/react@^12.8.5' \
  '@dnd-kit/core@^6.3.1' 'react-amnesia@^0.2.0'
```

Download `SHA256SUMS` from the same release to verify package and documentation archives.
The npm packages' manifests record their supported peer versions. Repository development
uses Node 22 and the lockfile in `typescript/`.

Upgrading the installed package does not upgrade saved flows or execution journals.
See the [execution](contracts/execution.md) and [recovery](contracts/recovery.md) contracts.
