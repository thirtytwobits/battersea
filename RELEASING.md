# Releasing

A release has one version across Cargo, npm and `release.json`. Rust consumers use its Git tag;
npm consumers install tarballs attached to that release. Registry publication is a separate decision.

`release.json` records the repository, MIT licence and private security-report route.
`cargo xtask release-check` checks these against the package metadata before publication.
Keep GitHub private vulnerability reporting enabled.

Every release runs CI, both API gates and package-install checks before creating assets. The tag
must match the checked version. The workflow creates a draft release containing npm tarballs,
Cargo package archives and SHA-256 checksums for review, with the version's `CHANGELOG.md` entry
as its notes. Publishing that draft is the final step.

CI also verifies a versioned documentation site. The draft includes its documentation archive
in the SHA-256 manifest. Publishing the release deploys the checked archive to GitHub Pages,
preserving earlier documented versions. Prereleases do not replace the stable landing page.
See [documentation delivery](docs/README.md) for build commands and publication retries.

API snapshots accompany intentional contract changes. Document the release scope in CHANGELOG.md;
package versions do not authorise upgrading a stored flow or journal format.
