# Releasing

The maintainer/owner runbook for cutting a gistui release. Contributors don't need any of
this — see [CONTRIBUTING.md](CONTRIBUTING.md).

## How a release works

A release is a `vX.Y.Z` git tag that matches `Cargo.toml`'s `version`. Pushing the tag
triggers `.github/workflows/release.yml`, which runs these steps in order, each only if the
one before succeeded:

1. Checks that the tag matches `Cargo.toml`'s version and runs the `mise run check` gate on
   the tagged commit.
2. Builds the platform binaries and attests their build provenance (checkable with
   `gh attestation verify <archive> --repo akunzai/gistui`).
3. Waits for approval of the `release` environment, then creates the GitHub Release with the
   binaries attached and updates the Homebrew formula and Scoop manifest.
4. Publishes the crate to [crates.io](https://crates.io/crates/gistui) from the `crates-io`
   environment, with no second approval: it starts only after the approved release job has
   succeeded. If only this step fails, re-run the failed job from the tag's workflow run; the
   release already exists.

The crate is published only while the `CARGO_REGISTRY_TOKEN` secret is configured; without
it the publish step skips itself and succeeds. The downstream package definitions are
pushed directly — no manual bump, no waiting on a schedule:

- [Homebrew tap](https://github.com/akunzai/homebrew-tap) — `Formula/gistui.rb` regenerated
  from the new release's per-platform checksums and pushed straight to `main`.
- [Scoop bucket](https://github.com/akunzai/scoop-bucket) — `bucket/gistui.json` patched with
  the new version/URL/hash and pushed straight to `main`.

Both pushes require the `HOMEBREW_BUMP_TOKEN` secret, a PAT scoped to those two repos, stored
as a repository or `release` environment secret; if it's unset, the corresponding step skips
itself and logs a message instead of failing the release.

Packaging stays lean via `Cargo.toml` `exclude` (the demo harness, site assets and CI config
are kept out of the published tarball); `cargo publish --dry-run` validates the tarball.

## Cutting a release

1. Bump `version` in `Cargo.toml` (and refresh `Cargo.lock`); confirm `cargo publish --dry-run`
   is clean.
2. In `CHANGELOG.md`, date the `## [Unreleased]` section as `## [X.Y.Z] — YYYY-MM-DD` and leave
   a fresh, empty `## [Unreleased]` heading above it — the section is permanent, so the next
   change has somewhere to land. At the bottom, add the release link reference and repoint
   `[unreleased]` at `compare/vX.Y.Z...HEAD`.
3. Merge to `main` (CI gate green).
4. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`.
5. Approve: open the tag's Release run in the Actions tab. Once the gate, builds, and
   attestation are green, approve the `release` deployment under **Review deployments**.
   That is the only approval; nothing leaves the repository before it. `release` (with a
   required reviewer) holds `HOMEBREW_BUMP_TOKEN`; `crates-io` (no reviewer) holds
   `CARGO_REGISTRY_TOKEN`. Both admit only tags matching `v*`.
6. Verify: the GitHub release has the binaries, [crates.io](https://crates.io/crates/gistui)
   shows the new version (and docs.rs built), and `Formula/gistui.rb` / `bucket/gistui.json`
   show a new `chore: bump gistui to vX.Y.Z` commit on the tap's / bucket's `main` (pushed by
   `release.yml` within the same run — if either is missing, check that run's "Update Homebrew
   formula" / "Update Scoop manifest" step and confirm `HOMEBREW_BUMP_TOKEN` is still valid).
