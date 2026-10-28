# Documentation delivery

The published site is <https://thirtytwobits.github.io/battersea/>. Its landing page selects the
latest documented stable release. Each release has independent handbook, Rust, TypeScript,
and schema paths under `/battersea/vX.Y.Z/`. Older releases that predate documentation archives
are not listed. Prereleases are listed without becoming the stable landing page.

## Build locally

Install the exact Rust and mdBook versions from `docs/toolchain.json`. mdBook is a repository
tool, not a runtime dependency:

```sh
rustup toolchain install 1.99.0 --profile minimal
cargo install mdbook --version 0.4.52 --locked
cargo xtask docs
```

That command checks generated bindings, stages the handbook, and builds public Rust crates
from `release.json` with all features and rustdoc warnings denied. It does not invoke Node.
Cargo-backed commands must run sequentially. Compilation intermediates are retained, but old
HTML is removed before generation.

In `typescript/`, install the lockfile and generate the reference:

```sh
npm ci --ignore-scripts
npm run docs
```

After installing these prerequisites, select **Documentation: Build** through VS Code's
**Tasks: Run Task** command. It builds the handbook, Rust reference, and TypeScript reference
in sequence, then checks the complete site. Output is written to `target/documentation/site/`.
The default build shortcut runs **Build: All**, which builds the Rust workspace and npm packages.

Then, from the repository root:

```sh
cargo xtask docs-check
cargo xtask docs-archive
mkdir -p target/documentation/preview
ln -sfn ../site target/documentation/preview/battersea
python3 -m http.server 8080 --bind 127.0.0.1 --directory target/documentation/preview
```

Open `http://localhost:8080/battersea/`. The preview mount reproduces the Pages project prefix,
including API navigation and version links. The link checker resolves every HTML link, asset,
and anchor under `/battersea/vX.Y.Z/`, and refuses links that escape that prefix.

The build writes only ignored output under `target/` and npm package build output.
`cargo xtask docs-archive` creates `target/documentation/battersea-docs-vX.Y.Z.tar.gz`.
Its `documentation.json` records the version, Git commit, dirty-source status, and generator
versions. Local dirty builds are useful for review; the publisher refuses to deploy them.

## Sources and coverage

Edit the handbook in `docs/src/`, with navigation in `SUMMARY.md`. The build copies contracts
from `contracts/` without editing them. Repository and package guides, licence, notices,
release notes, and provenance are also staged from their original sources. The first-flow
walkthrough includes the independently compiled catalogue example. Provider feature lists
are generated from the package manifest. Installation examples use `release.json`'s version.

Rust API documentation comes from the public crates named in `release.json`, without examples,
repository tooling, or private items. TypeDoc 0.28.20 is pinned in the TypeScript lockfile;
package configurations cover the public TypeScript entry points and editor subpaths.
When changing package exports, update the documentation entry points as well. API snapshots
in `api/` remain compatibility records and are not documentation inputs.

## CI, releases, and publication

CI builds and checks a complete site and uploads a downloadable `documentation-preview`
artifact for review. Pull requests cannot deploy Pages. Rust and handbook generation run
in a lane that refuses Node invocations; TypeScript documentation is built separately.

The push to `main` archives the checked site and keeps it with the Cargo and npm packages as
the build's release files. The Draft release workflow attaches them to the draft, and
`SHA256SUMS` covers every archive. Publishing the draft starts `docs-pages.yml` through
`release: published`.
Documentation is not published merely because a tag or a draft exists.

The Pages workflow reconstructs the entire site from published release assets. It verifies
each archive's checksum, safe paths, version identity, and clean source commit against the tag.
It never needs a previous Actions artifact or a previous Pages deployment to retain versions.
The GitHub-designated latest release controls the landing page when it has documentation;
otherwise the highest documented stable semantic version is used. Release creation time does
not determine the stable landing page.

Deployment runs are serialized. The build has read permissions; only deployment receives
`pages: write` and `id-token: write`. The `github-pages` environment must allow release tags
matching `v*` as well as the default branch. Pages must use **GitHub Actions** as its publishing source.

After merging the workflows, the next release delivers documentation automatically. Existing
releases without a documentation archive remain unlisted; do not attach a build from a different
source commit to an old tag.

## Retry and recovery

Run the **Publish documentation** workflow manually to retry assembly and deployment from
published archives. The same workflow is safe to rerun after an older release is published:
it queries the current set of releases and latest designation before assembly. Every deployment
contains all documented published versions.

If publication fails after a release is published, correct the workflow or restore the matching
checksummed release asset, then retry. A deployment failure does not require retagging or
republishing packages. The deployment job checks the landing page, version index, handbook,
a Rust symbol, TypeScript reference, and flow schema over HTTPS after Pages reports success.
