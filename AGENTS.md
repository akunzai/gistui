# AGENTS.md

`gistui` is a Rust 2021 TUI for browsing, comparing, and managing GitHub Gists through `gh`.

## Commands

Toolchains and task wrappers live in [`mise.toml`](mise.toml); run `mise install` once. `gh` is a user runtime dependency and is not pinned.

- Verification gate: `mise run check`
- Single test: `cargo test <name_filter>`
- Non-TTY readiness check: `cargo run -- --check`
- Demo regeneration: `mise run demo` (see `@docs/demo.md`)

## Pointers

- Product design — voice, product language, row/column layout, the mark vocabulary, README scope: `@docs/agents/design.md`
- Architecture, state-machine, jobs, IO boundaries, safety seams, truncation, and GistFile constructors: `@docs/agents/architecture.md`
- Agent-only contribution and release conventions: `@docs/agents/conventions.md`
- Human contribution flow: `@CONTRIBUTING.md`
- Release runbook: `@RELEASING.md`
- Configuration schema (metadata only): `@config.example.toml`
- Issue tracker workflow: `@docs/agents/issue-tracker.md`
- Triage labels: `@docs/agents/triage-labels.md`
- Domain and ADR discovery: `@docs/agents/domain.md`

## Self-Reflection

- **Candidate**: Distill a non-obvious gotcha into ≤ 2 context-tagged bullets. Propose it before writing.
- **Promote**: On confirmation, put it where whoever would break it must already pass — enforce it (assert/type/test) when the fix is in hand, else a comment at that site, else an agent-facing doc (`docs/agents/<topic>.md`, else `docs/agents/lessons-learned.md`) with one `@path` line under Pointers. Never both.
- **Prune**: Drop entries once stale (obsolete version, now enforced, duplicated, or a transcript) — not by a fixed count.

## Claude Code compatibility

`CLAUDE.md` is a symbolic link to `AGENTS.md`; edit `AGENTS.md` directly.
