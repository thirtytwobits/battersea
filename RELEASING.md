# Releasing

A release has one version across Cargo, npm and `release.json`. Rust consumers use its Git tag;
npm consumers install tarballs attached to that release. Registry publication is a separate decision.

`release.json` records the repository, MIT licence and private security-report route.
`cargo xtask release-check` checks these against the package metadata before publication.
Keep GitHub private vulnerability reporting enabled.

Every release runs CI, both API gates and package-install checks before creating assets. The tag
must match the checked version. The workflow creates a draft release containing npm tarballs,
Cargo package archives and SHA-256 checksums for review. Publishing that draft is the final step.

API snapshots accompany intentional contract changes. Document the release scope in CHANGELOG.md;
package versions do not authorise upgrading a stored flow or journal format.
