# AGENTS.md

`gistui` is a Rust 2021 TUI for browsing, comparing, and managing GitHub Gists through `gh`.

## Commands

Toolchains and task wrappers live in [`mise.toml`](mise.toml); run `mise install` once. `gh` is a user runtime dependency and is not pinned.

- Verification gate: `mise run check`
- Single test: `cargo test <name_filter>`
- Non-TTY readiness check: `cargo run -- --check`
- Demo regeneration: `mise run demo` (see `@scripts/demo/README.md`)

## Pointers

- Architecture, state-machine, jobs, IO boundaries, and safety seams: `@docs/agents/architecture.md`
- Agent-only contribution and release conventions: `@docs/agents/conventions.md`
- Human contribution flow: `@CONTRIBUTING.md`
- Release runbook: `@RELEASING.md`
- Configuration schema (metadata only): `@config.example.toml`
- Issue tracker workflow: `@docs/agents/issue-tracker.md`
- Triage labels: `@docs/agents/triage-labels.md`
- Domain and ADR discovery: `@docs/agents/domain.md`

## Self-Reflection

- **Candidate**: Distill non-obvious project knowledge into at most two concise, context-tagged bullets and propose it before writing.
- **Promote**: After confirmation, add it to a relevant topic file under `docs/` (or `docs/lessons-learned.md`) and link that file above.
- **Prune**: Propose removing entries once they are obsolete, enforced by code, duplicated, or merely a debugging transcript.

## Claude Code compatibility

`CLAUDE.md` is a symbolic link to `AGENTS.md`; edit `AGENTS.md` directly.
