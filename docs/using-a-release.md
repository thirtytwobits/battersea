# Using a Battersea release

npm consumers install the `@battersea/flow` and `@battersea/editor` tarballs attached to the
release; [installation](src/installation.md) gives the commands.

<!-- newsletter:begin consuming · maintained in thirtytwobits/london -->

## Taking a release

Battersea publishes nothing to crates.io. Take its crates from a release tag:

```toml
[dependencies]
CRATE = { git = "https://github.com/thirtytwobits/battersea", tag = "vX.Y.Z" }
```

Cargo treats each tag as a separate source, so take every Battersea crate from the
same tag, and commit `Cargo.lock`. The libraries need Rust 1.95 or later.

`SHA256SUMS` lists the files attached to a release. Check a download with
`sha256sum --check --ignore-missing SHA256SUMS`.

[`london.toml`](https://github.com/thirtytwobits/london/blob/main/london.toml) names the release
of each piece that the London Project's examples build against.

<!-- newsletter:end consuming -->
