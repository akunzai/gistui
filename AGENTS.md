# AGENTS.md

`gistui` is a Rust 2021 TUI for GitHub Gists (browse / diff / download / upload / create / pin) via `gh`.

## Commands

Toolchain + task wrappers: [`mise.toml`](mise.toml) (`mise install` once; `gh` is a user runtime dep, not pinned).

```bash
mise install            # pinned Rust/agg/uv
cargo run               # TUI (needs TTY)
cargo run -- --check    # gh readiness, no TUI
cargo test              # no network / no gh auth
mise run check          # verification gate (fmt + clippy -D warnings + test + check)
mise run demo           # regenerate demo media (see @scripts/demo/README.md)
```

Single-test: `cargo test <name_filter>`.

## Architecture (index)

**Pure vs impure**, **Screen SM**, **Jobs**, **VM seam**, **pin-sync cache**, **teardown**: full jargon + rich refs in [`docs/agents/architecture.md`](docs/agents/architecture.md).

Cheat sheet:

| Seam | Rule |
| --- | --- |
| Pure modules | unit-tested; no FS/network |
| Thin IO | `gh` / execute / `run_loop`+`bg` — not unit-tested by design |
| Keys | `handle_key` pure → `KeyOutcome` → dispatch IO |
| Jobs | single `Jobs` registry; gen supersession; absorb only in `run_loop` |
| Paint | pure `ViewModel` each frame; paint = theme/layout only |
| Screen | `Clone` not `Copy`; `List` unit tag; payloads + `nav_stack` for return |

Rich refs: `@src/tui/mod.rs` (`Screen`, `AppState`, `KeyOutcome`), `@src/tui/view_model.rs`, `@src/tui/bg.rs` (`Jobs`), `@src/actions.rs` (`DownloadMode`).

## Non-obvious constraints

- **No live `gh`/network in tests.** Parse fixtures: `tests/fixtures/gh/`. Multi-step collect: injectable `CommandRunner` (`*_with` + `SeqRunner`, #245). E2E frames → `@scripts/demo/`, not the unit suite.
- **Download overwrite is type-enforced** (#246): `DownloadMode::CreateNew` \| `Overwrite` (token only via `overwrite_after_user_confirm()` after Confirm `y`). Never a boolean bypass. Target: `cwd/<gist-filename>` only.
- **Config is metadata-only**: `pinned` + `skip_dirs` (and UI prefs) — never gist *content* or GitHub tokens. Schema: `@config.example.toml`.
- **ratatui 0.30**: `frame.area()` (not removed `frame.size()`); `Rect::inner` takes `Margin` by value.

## Conventions

Human contributor flow: [`CONTRIBUTING.md`](CONTRIBUTING.md). Release cut: [`RELEASING.md`](RELEASING.md).

Agent-facing deltas (not fully restated in those docs):

- Conventional Commits (English); fold same-scope follow-ups via amend (no `fix typo` noise).
- Every PR: one release-note label — `enhancement` \| `bug` \| `documentation` \| `dependencies` \| `skip-changelog` (see `.github/release.yml`).
- User-facing key/screen/feature change → update `README.md` + `?` help in `tui` **same PR**.
- User-visible fix/feature → one-line bullet under `CHANGELOG.md` `## [Unreleased]` **same PR** (skip for `skip-changelog` / pure internal).
- Stay on `0.x` while keymap surface evolves. Mid-cycle: bump `Cargo.toml` on first post-release feature; keep changelog under `[Unreleased]` until the cut renames it and tags `vX.Y.Z`.

## Agent skills (lazy-load)

- Issue tracker (`gh`): `@docs/agents/issue-tracker.md`
- Triage labels: `@docs/agents/triage-labels.md`
- Domain / ADR lazy layout: `@docs/agents/domain.md`
- Architecture deep-dive: `@docs/agents/architecture.md`

## Knowledge writeback

When problem-solving surfaces **non-obvious** project knowledge (gotcha, env quirk, framework behavior, hidden config), the agent **must**:

1. **Distill** — one durable, context-tagged bullet (e.g. `[ratatui 0.30] …`), not a debug transcript.
2. **Propose** — show the candidate snippet and ask: *“This insight may be worth preserving. Shall I add it to `AGENTS.md`?”*
3. **Write only after explicit approval** — never silent writeback.
4. **Route**:
   - Durable architecture / seam rule → `@docs/agents/architecture.md` or a Rich Ref (type/test), not long prose here.
   - Short gotcha → `## Lessons Learned` below (or `## Non-obvious constraints` if it is a standing gate).
5. **Quality gates** (all required): non-derivable from code alone · not a drifting metric · not micromanagement · ≤ 2 bullets.
6. **Active pruning**: `## Lessons Learned` max **5** entries. Over 5 → propose drop obsolete version-scoped gotchas or promote into types/tests/architecture docs.

## Lessons Learned (actively pruned, max 5)

<!-- Populated only via Knowledge writeback after user approval. -->

## Claude Code compatibility

> [!NOTE]
> `CLAUDE.md` is a symbolic link to `AGENTS.md`. Edit **only** `AGENTS.md`; do not edit or replace the symlink.
