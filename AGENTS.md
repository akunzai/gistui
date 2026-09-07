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

## Prevent Recurrence

- **Candidate**: Name who hits this again, in which file, on what change. No such scenario, nothing to propose.
- **Promote**: Offer the first tier that reaches them and only that one, pending confirmation — enforce it (assert/type/test) with its size quoted, else a comment at that site, else an agent-facing doc (`docs/agents/<topic>.md`, else `docs/agents/lessons-learned.md`) with one `@path` line under Pointers and one sentence on why the tiers above cannot hold it. Never both.
- **Prune**: When adding to a file, audit the rest of it in the same pass. Drop entries once stale (obsolete version, now enforced, duplicated, or a transcript) — not by a fixed count.

## Claude Code Compatibility

`CLAUDE.md` is a symbolic link pointing to `AGENTS.md`. Edit `AGENTS.md` directly.
