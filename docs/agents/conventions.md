# Agent conventions

Follow [`CONTRIBUTING.md`](../../CONTRIBUTING.md) for the normal contribution flow and [`RELEASING.md`](../../RELEASING.md) when cutting a release. Agent-only deltas:

- Use English Conventional Commits. Amend same-scope follow-ups instead of adding cleanup commits.
- Give every PR one release-note label: `enhancement`, `bug`, `documentation`, `dependencies`, or `skip-changelog` (see `@.github/release.yml`).
- A user-facing key, screen, or feature change updates both `README.md` and the TUI `?` help in the same PR.
- Put an issue and the PR that implements it on the same milestone. `gh issue create --milestone "<x.y.z>"` / `gh pr edit <n> --milestone "<x.y.z>"`; a PR inherits nothing from the issue it closes.
- A user-visible fix or feature adds one bullet under `CHANGELOG.md` `## [Unreleased]`, unless the PR is pure internal work or carries `skip-changelog`.
- A new test goes in the module that owns what it asserts: a screen's key and view-model behaviour in `src/tui/screens/<screen>.rs`, cross-screen dispatch (quit latch, top-bar clicks, scroll, theme, yank) in `src/tui/keys.rs`, a pure function beside its definition. Shared `AppState` fixtures have one home, `tui::test_support`. Never start a central test file — that is how `tui/tests.rs` reached 3,916 lines before #381 retired it.
- Stay on `0.x` while the keymap evolves. After a release, the first feature bumps `Cargo.toml`; keep changelog entries under `[Unreleased]` until the release cut.
