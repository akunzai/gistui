# Architecture (agent deep-dive)

Index: [`AGENTS.md`](../../AGENTS.md). Source of truth for types lives in the modules below — prefer reading them over re-deriving rules from prose.

## Pure / impure boundary

| Kind | Modules | Testing |
| --- | --- | --- |
| **Pure** (unit-tested) | `domain`, `config`, `ranking`, `local`, `diff`, actions **plan/guard**, `tui::view_model`, `tui::list_ranking` | In-crate unit tests |
| **Impure** (thin IO) | `gh`, actions **execute**, `tui::run_loop` / `tui::bg` / `tui::pin_sync` | Fixtures / injectable runners only; no live `gh` |

`build_view_model` (`@src/tui/view_model.rs`): `AppState` + pin-sync cache → presentation facts. Paint helpers apply theme/layout only — no business rules, FS, or network.

## Screen state machine

- **`Screen` is `Clone`, not `Copy`** — payload variants own screen-local UI (`@src/tui/mod.rs`, issue #242).
- **`List` stays a unit tag** — dual-pane selection / filters / sorts are session-global on `AppState` (user story 19).
- Other variants (`Diff`, `Confirm`, `Preview`, `Help`, `Config`, `Revisions`, `Pins`, `Gists`, `GistDetail`, `Palette`, …) carry payloads (body/scroll/return/origin as needed).
- **`nav_stack`** (issue #271) holds return targets; Esc pops. Prefer stack ops over parallel “return” root fields for new navigation.
- Staged root fields (`diff_return` / `preview_return` / `staged_diff_gist` and similar) are consumed on enter only when still present.
- **`back_to_list()` is a hard reset** (clears `nav_stack`) — reserve it for paths whose only possible origin is `Screen::List` itself. Confirm-execute paths with more than one possible origin (e.g. `ExecuteDelete`, `ExecuteCompactGist` in `dispatch.rs`) use `leave()`/`cancel_confirm()` instead, to return to whichever screen actually launched them; pop an extra time if the popped screen is now stale (e.g. `GistDetail` for a gist just deleted).

## Key path (pure intent → impure dispatch)

```
handle_key (pure) → KeyOutcome → run_loop / dispatch_outcome (IO)
```

- New key logic → `AppState::handle_key` (testable).
- New IO → `dispatch` / `bg` helpers, not `handle_key`.
- IO-bearing `KeyOutcome` variants carry payloads (issue #244): `@src/tui/mod.rs` (`KeyOutcome`), `@src/tui/dispatch.rs`.

## Background jobs

- **`Jobs`** is the single registry: spawn / absorb / cancel (`@src/tui/bg.rs`, issue #243).
- Action jobs and local scans use **generation supersession** (issue #221).
- Call sites: `jobs.spawn_action` / `request_*` — do not own ad-hoc channel fields on `AppState`.
- `run_loop` only **polls** `jobs.absorb`.

## Pin-sync presentation (`@src/tui/pin_sync.rs`)

- `refresh_pin_sync_cache` is **impure** (stat/read/hash); fills `AppState::pin_sync_cache`.
- Refresh on: enter/return Pins, pin-list change, successful pin-sync absorb, dirty flag / length mismatch — **not** every frame, **not** from the pure VM builder.
- Action dispatch may call `compute_pin_sync_status` one-shot; paint uses `cached_pin_sync_status` / the VM only.
- No mtime watch: staying on Pins after an external editor edit can leave badges stale until the next refresh.

## List-row and title truncation (`@src/tui/render.rs`)

Two ellipsis operations — different jobs, do not merge:

- **`truncate_end`** (issue #340) — keep the head, append `…`. List rows (`visible_list_row`), pane/overlay titles (`fit_block_title`), and a `fit_title` head that itself cannot fit.
- **`elide_start`** (issue #338) — keep the tail, leading `…`. Only `fit_title`'s trailing context (the Local pane cwd).

Narrow-terminal reflow (issue #342), also in `@src/tui/render.rs` — different jobs from ellipsis, do not merge:

- **`wrap_hanging`** — wrap a line to width, continuing at the source line's leading whitespace. Help body and gist-comment bodies (pre-wrap at paint; do not use `Paragraph` wrap, which drops indent).
- **`fit_hints`** — drop whole ` · `-separated footer items so a coloured hint line stays one row; the last item (leave key) is kept. Status messages still wrap via `wrap_line_count`.

List-row horizontal scroll (issue #341) is **per selected row**, not pane-wide: `row_hscroll` applies the pane offset only to the highlighted index; `focused_hscroll_max` / Pins / Gists caps are that row's painted string. A non-zero offset prefixes `…` in `visible_list_row` (then `truncate_end` may still mark a clipped tail). Unselected rows stay at column 0.

List-row budget: inner pane width minus borders+padding (`LIST_CHROME_CELLS`) minus `LIST_HIGHLIGHT_SYMBOL`. ratatui's default `HighlightSpacing::WhenSelected` indents **every** row once any row is selected — these lists always `select(...)` when they have rows, so unselected rows still pay the `▶ ` indent. Keep the widget's `highlight_symbol` and the budget on `LIST_HIGHLIGHT_SYMBOL` so they cannot drift.

## Terminal lifecycle

`run()` wraps `run_loop()` so raw mode / alternate-screen teardown **always** runs. Keep fallible startup/IO inside `run_loop`, never between `enable_raw_mode` and teardown.

## Safety seams (rich refs)

- Download overwrite gate (issue #246): `@src/actions.rs` — `DownloadMode` / `OverwriteConfirmed`.
- Injectable `gh` boundary (issue #245): `CommandRunner` + `actions::test_support::SeqRunner`; fixtures in `tests/fixtures/gh/`.
- Gold-style TUI pure logic: `@src/tui/tests.rs` (no network).
- E2E frames: `@scripts/demo/` (real binary + fake `gh` + fake cwd).
