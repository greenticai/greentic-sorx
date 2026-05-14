# Release Readiness

Release metadata:

- Crate name: `greentic-sorx`
- Binary name: `greentic-sorx`
- Versioning: semantic versioning from workspace `Cargo.toml`
- Expected install paths: `cargo install greentic-sorx` or
  `cargo binstall greentic-sorx`
- Git tags: `vX.Y.Z`, matching the Cargo package version
- Changelog: `CHANGELOG.md`

Release workflows:

- `.github/workflows/release.yml` verifies release readiness and does not
  publish.
- `.github/workflows/publish.yml` runs on pushes to `main`/`master`, verifies
  release metadata, creates or verifies the matching `vX.Y.Z` tag, dispatches
  `.github/workflows/release-binaries.yml` on that tag, waits for the six
  GitHub Release archives for `cargo-binstall`, then publishes crates.io
  packages in dependency order with `CARGO_REGISTRY_TOKEN`.
- `.github/workflows/release-binaries.yml` is dispatch-only. It runs the shared
  binary release workflow when invoked on a tag ref and can be manually
  dispatched to rebuild GitHub Release archives for an existing tag.

`cargo-binstall` expects archives named
`greentic-sorx-vX.Y.Z-<target>.tgz` with the binary under the archive directory
`greentic-sorx-vX.Y.Z-<target>/`.

Future Greentic toolchain integration should add `greentic-sorx` to the shared
toolchain manifest beside the other Greentic CLI tools after the command shape
and version policy are stable.

GHCR publishing remains disabled in this repository. Later deployment registry
work may read GHCR references and exact digests, but SORX should not publish
runtime packs unless that policy changes.
