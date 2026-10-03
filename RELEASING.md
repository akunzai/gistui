# Releasing

The maintainer/owner runbook for cutting a gistui release. Contributors don't need any of
this — see [CONTRIBUTING.md](CONTRIBUTING.md).

## How a release works

[GitHub Releases](https://github.com/akunzai/gistui/releases) are the version change record.
The workflow generates release notes from merged PR titles, grouped by the labels in
`.github/release.yml`. The release-note review procedure is below.

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

1. On a `release/X.Y.Z` branch, bump `version` in `Cargo.toml` and refresh `Cargo.lock`
   with `cargo build` (`--locked` refuses the version change). Validate the tarball with
   `cargo publish --dry-run --allow-dirty` before committing, or without `--allow-dirty` after.
2. Review merged PR titles and category labels, and identify user-visible direct commits
   that need manual release-note entries using the procedure below.
3. Check the committed demo and stills against the recorded flows in `docs/demo.md`.
   Re-record with `mise run demo` when a user-visible change appears in those flows;
   skip it for changes the recordings do not show. Keep only assets whose picture changed,
   comparing against the committed version before restoring unchanged generated files.
   Per-change TUI verification still follows `docs/agents/verification.md`.
4. Open a PR from `release/X.Y.Z` and merge it to `main` once the CI gate is green.
   On the merged `main`, preview notes again before tagging.
5. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`.
6. Approve: open the tag's Release run in the Actions tab. Once the gate, builds, and
   attestation are green, approve the `release` deployment under **Review deployments**.
   That is the only approval; nothing leaves the repository before it. `release` (with a
   required reviewer) holds `HOMEBREW_BUMP_TOKEN`; `crates-io` (no reviewer) holds
   `CARGO_REGISTRY_TOKEN`. Both admit only tags matching `v*`.
7. Verify: the GitHub release has the binaries and expected notes; add the missing entries
   identified during review or a highlights summary when useful. Confirm [crates.io](https://crates.io/crates/gistui)
   shows the new version (and docs.rs built), and `Formula/gistui.rb` / `bucket/gistui.json`
   show a new `chore: bump gistui to vX.Y.Z` commit on the tap's / bucket's `main` (pushed by
   `release.yml` within the same run — if either is missing, check that run's "Update Homebrew
   formula" / "Update Scoop manifest" step and confirm `HOMEBREW_BUMP_TOKEN` is still valid).
8. Create the next version's milestone if it does not exist, with a one-line description
   (`gh api repos/{owner}/{repo}/milestones -f title=<x.y.z> -f description=...`). Move
   still-open issues from the released milestone to it
   (`gh issue list --milestone X.Y.Z --state open`). Write a concise summary of shipped
   highlights derived from the release notes into the released milestone's description,
   then close it. A PR merged after the tag belongs to the next milestone.

## Reviewing release notes

After release preparation is merged, preview notes for the proposed tag without creating
a release. Replace `vX.Y.Z` and `vPREVIOUS` with the proposed and previous release tags:

```sh
gh api --method POST repos/{owner}/{repo}/releases/generate-notes \
  -f tag_name=vX.Y.Z \
  -f target_commitish=main \
  -f previous_tag_name=vPREVIOUS \
  --jq .body
```

The [generate-notes API](https://docs.github.com/en/rest/releases/releases#generate-release-notes-content-for-a-release)
returns a preview without saving a release or draft; the workflow generates the final
notes at the tag. Check that the comparison starts at the intended previous release,
PR titles describe the user-visible changes, and labels place them in the right sections.
`skip-changelog` excludes a PR from the notes. Correct misleading titles or labels and
regenerate the preview before tagging.

After fetching `origin`, compare the preview with
`git log --oneline vPREVIOUS..origin/main`. Changes committed directly to `main` have no
merged PR entry: identify missing user-visible changes before tagging, then add their
entries to the published release notes during verification. Add a highlights summary
when it helps readers. [Immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases)
lock the tag and assets, but still allow editing the title and release notes.
