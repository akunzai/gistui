# Agent conventions

Follow [`CONTRIBUTING.md`](../../CONTRIBUTING.md) for the normal contribution flow and [`RELEASING.md`](../../RELEASING.md) when cutting a release. Agent-only deltas:

- Use English Conventional Commits. Amend same-scope follow-ups instead of adding cleanup commits.
- Give every PR one release-note label: `enhancement`, `bug`, `documentation`, `dependencies`, or `skip-changelog` (see `@.github/release.yml`).
- A user-facing key, screen, or feature change updates both `README.md` and the TUI `?` help in the same PR.
- A user-visible fix or feature adds one bullet under `CHANGELOG.md` `## [Unreleased]`, unless the PR is pure internal work or carries `skip-changelog`.
- Stay on `0.x` while the keymap evolves. After a release, the first feature bumps `Cargo.toml`; keep changelog entries under `[Unreleased]` until the release cut.
