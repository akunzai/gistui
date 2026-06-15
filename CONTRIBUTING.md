# Contributing to gistui

## Prerequisites

- Rust (stable toolchain)
- [GitHub CLI](https://cli.github.com/) (`gh`) installed and authenticated

## Development Setup

```bash
git clone https://github.com/akunzai/gistui.git
cd gistui
cargo build
```

Run without a TTY to verify `gh` is ready:

```bash
cargo run -- --check
```

## Verification Gate

All four must pass before every commit:

```bash
cargo fmt --check   # or: cargo fmt
cargo test
cargo check
cargo clippy --all-targets -- -D warnings
```

## Architecture

See [AGENTS.md](AGENTS.md) for the full architecture guide, non-obvious rules, and conventions.

Key points:

- **Pure modules** (`domain`, `config`, `ranking`, `local`, `diff`, `actions` planner/guard): unit-tested, no IO.
- **Thin IO boundaries** (`gh`, `actions` execute helpers, `tui::run_loop`): not unit-tested by design.
- `AppState::handle_key` is **pure** — it mutates state and returns a `KeyOutcome`; all IO runs in `run_loop`.
- Tests must never call the real `gh` or the network. Fixtures live in `tests/fixtures/gh/`.

## Submitting a PR

1. Fork and create a branch (`feat/my-feature` or `fix/issue-123`).
2. Keep commits focused; follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `chore:`).
3. Open a PR against `main`; the CI gate must be green.
4. **Label the PR** so it lands in the right release-note section (`.github/release.yml`): `enhancement` (Features), `bug` (Bug Fixes), `documentation` (Documentation), `dependencies`, or `skip-changelog` to omit it. Unlabeled PRs fall under "Other Changes".

## Releasing & publishing to crates.io

The crate is packaging-ready (`cargo publish --dry-run` passes; `Cargo.toml` `exclude`
keeps the demo harness, site assets and CI config out of the published tarball). Publishing
is automated by `.github/workflows/publish.yml`, which runs on each `vX.Y.Z` tag and no-ops
until the registry token is set.

**One-time crates.io setup** (owner):

- [ ] Create a [crates.io](https://crates.io) account (sign in with GitHub) and verify your email.
- [ ] Generate a scoped API token (Account Settings → API Tokens) with `publish-new` + `publish-update`, scoped to the `gistui` crate once it exists.
- [ ] Add it as a repository secret named `CARGO_REGISTRY_TOKEN` (Settings → Secrets and variables → Actions).

**First publish:**

- [ ] Confirm `Cargo.toml` `version` is correct and `cargo publish --dry-run` is clean.
- [ ] Either push the matching tag (`git tag vX.Y.Z && git push origin vX.Y.Z`) — `publish.yml` then publishes — or run the **Publish to crates.io** workflow manually (Actions tab → Run workflow), or publish locally with `cargo publish`.
- [ ] Verify the crate at `https://crates.io/crates/gistui` and that docs.rs built.

**Post-publish docs** (add once the crate is live):

- [x] Add a crates.io badge under the README title:
  ```markdown
  [![crates.io](https://img.shields.io/crates/v/gistui.svg)](https://crates.io/crates/gistui)
  ```
- [x] Add a `cargo install gistui` option to the README install section.
- [x] Add `[package.metadata.binstall]` and verify `cargo binstall --dry-run gistui` (see issue #93).

**Ongoing:** each release is a `vX.Y.Z` tag matching `Cargo.toml`; the tag triggers both the binary release (`release.yml`) and the crates.io publish (`publish.yml`). The [Homebrew tap](https://github.com/akunzai/homebrew-tap) and [Scoop bucket](https://github.com/akunzai/scoop-bucket) both update themselves via their scheduled workflows (`brew livecheck` bump / `checkver` + `autoupdate` Excavator), so neither needs a manual bump on release.

## Reporting Issues

Use the [bug report](.github/ISSUE_TEMPLATE/bug_report.yml) or [feature request](.github/ISSUE_TEMPLATE/feature_request.yml) templates.
