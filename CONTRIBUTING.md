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

## Building a release binary

```bash
cargo build --release
```

The optimized binary is `target/release/gistui`. It bundles no assets, but still requires
the `gh` CLI on `PATH` at runtime (`gh` is not vendored). Optionally shrink it with
`strip target/release/gistui`.

## Architecture

See [AGENTS.md](AGENTS.md) for the full architecture guide, non-obvious rules, and conventions
(coding agents should read `AGENTS.md` / `CLAUDE.md` first).

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

## Releasing

Maintainers: see [RELEASING.md](RELEASING.md) for the release and crates.io publish process.

## Reporting Issues

Use the [bug report](.github/ISSUE_TEMPLATE/bug_report.yml) or [feature request](.github/ISSUE_TEMPLATE/feature_request.yml) templates.
