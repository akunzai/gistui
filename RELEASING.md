# Releasing

The maintainer/owner runbook for cutting a gistui release. Contributors don't need any of
this — see [CONTRIBUTING.md](CONTRIBUTING.md).

## How a release works

A release is a `vX.Y.Z` git tag that matches `Cargo.toml`'s `version`. Pushing the tag
triggers the full pipeline:

- `.github/workflows/release.yml` — builds and attaches the platform binaries.
- `.github/workflows/publish.yml` — publishes the crate to
  [crates.io](https://crates.io/crates/gistui).

The crate is published and the `CARGO_REGISTRY_TOKEN` secret is configured, so the publish
step runs automatically on tag. `release.yml` also pushes the downstream package definitions
directly — no manual bump, no waiting on a schedule:

- [Homebrew tap](https://github.com/akunzai/homebrew-tap) — `Formula/gistui.rb` regenerated
  from the new release's per-platform checksums and pushed straight to `main`.
- [Scoop bucket](https://github.com/akunzai/scoop-bucket) — `bucket/gistui.json` patched with
  the new version/URL/hash and pushed straight to `main`.

Both pushes require the `HOMEBREW_BUMP_TOKEN` repository secret (a PAT scoped to those two
repos); if it's unset, the corresponding step skips itself and logs a message instead of
failing the release.

Packaging stays lean via `Cargo.toml` `exclude` (the demo harness, site assets and CI config
are kept out of the published tarball); `cargo publish --dry-run` validates the tarball.

## Cutting a release

1. Bump `version` in `Cargo.toml` (and refresh `Cargo.lock`); confirm `cargo publish --dry-run`
   is clean.
2. In `CHANGELOG.md`, rename the `## [Unreleased]` section to a dated `## [X.Y.Z] — YYYY-MM-DD`
   heading and add its release link reference at the bottom.
3. Merge to `main` (CI gate green).
4. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`.
5. Verify: the GitHub release has the binaries, [crates.io](https://crates.io/crates/gistui)
   shows the new version (and docs.rs built), and `Formula/gistui.rb` / `bucket/gistui.json`
   show a new `chore: bump gistui to vX.Y.Z` commit on the tap's / bucket's `main` (pushed by
   `release.yml` within the same run — if either is missing, check that run's "Update Homebrew
   formula" / "Update Scoop manifest" step and confirm `HOMEBREW_BUMP_TOKEN` is still valid).
