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

## Keymap (`@src/tui/keymap.rs`)

`keymap::for_screen` returns one table per screen describing every key it binds (issue #369). The table **describes**; it does not dispatch and it does not gate — `handle_key_*` still executes keys, and `*_guard` still answers "is this available right now" (issue #288). Adding a key means adding a row *and* a `handle_key_*` arm; the tests below make the omission loud.

Three renderings derive from it — do not hand-write any of them again:

- **Palette rows** — `build_palette_items` walks the table and calls the screen's guard. There are no per-screen `*_palette_items` builders any more.
- **Footer hints** — `keymap::footer_hints`, ordered by `FooterHint::rank`. Only the three screens with a `footer` column get one; the rest paint `MINIMAL_HINT`, and `footer_hints` returns `""` for them.
- **Action colour** — `Category` → `category_color`. Paint never infers a category from wording.

Two columns exist because the two surfaces genuinely disagree, not by oversight: `FooterHint::rank` is separate because footer order leads with navigation while palette order leads with actions, and `FooterHint::key` is separate because the footer names every key that leaves a screen (`Esc/q`) where the palette names the one it executes (`q`).

**`Category` is about the user's data, not about persistence.** Changing a setting writes `config.toml` and is still `Nav`; `Write` means a gist, a local file, or the pin list moves; `Destructive` means something does not come back.

Help topics and `README.md` stay hand-written — the List topic is fifty lines of sectioned prose, and generating it would flatten explanation to win a check a test can do instead. `keymap::docs_tests` is that check, in both directions: every bound key must appear in its help topic or in General (#344's direction), and every key `README.md`'s Key/Action table promises must be bound (#346's direction).

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

## List panes (`@src/tui/render/list_pane.rs`)

`render_list_pane` paints **every** bordered list of selectable rows (issue #367): both List panes, Gist manager, Pinned Mappings, Revisions. Callers describe the pane with a `ListPaneVm` and never assemble the widget — clipping, horizontal scroll, empty state, title fit, the scrollbar, and the `PaneHit` all live behind that one call.

- **Row geometry is module-private** (`row_hscroll`, `visible_list_row`, `LIST_CHROME_CELLS`). Only `LIST_HIGHLIGHT_SYMBOL` stays visible, for Help's ad-hoc topic index.
- **Settings and Help's index are deliberately out**: different highlight symbol, bottom title, untruncated rows. Two callers would not justify the seam — do not fold them in without a third.
- **Emphasis is a presentation fact, not a paint decision**: builders map their domain reason (`MatchMark::ExactFilename`, `SyncStatus::Missing`) to `RowEmphasis::{Strong, Danger}`. Paint only looks it up — do not reintroduce a domain match on the paint side.
- **`focused`** drives border colour *and* selection highlight together (solid bar when focused, bold when not); single-pane screens pass `true`. `scrollbar` is `true` only for the two List panes.
- **Empty state** is `ListPaneEmpty` plus a prebuilt `empty_message` from the builder. No screen hard-codes an empty message at paint time.

List-row horizontal scroll (issue #341) is **per selected row**, not pane-wide: `row_hscroll` applies the pane offset only to the highlighted index; `focused_hscroll_max` / Pins / Gists caps are that row's painted string. A non-zero offset prefixes `…` in `visible_list_row` (then `truncate_end` may still mark a clipped tail). Unselected rows stay at column 0.

List-row budget: inner pane width minus borders+padding (`LIST_CHROME_CELLS`) minus `LIST_HIGHLIGHT_SYMBOL`. ratatui's default `HighlightSpacing::WhenSelected` indents **every** row once any row is selected — these lists always `select(...)` when they have rows, so unselected rows still pay the `▶ ` indent. Keep the widget's `highlight_symbol` and the budget on `LIST_HIGHLIGHT_SYMBOL` so they cannot drift.

## Text fitting (`@src/tui/render/mod.rs`)

Two ellipsis operations — different jobs, do not merge:

- **`truncate_end`** (issue #340) — keep the head, append `…`. List rows (`visible_list_row`), pane/overlay titles (`fit_block_title`), and a `fit_title` head that itself cannot fit.
- **`elide_start`** (issue #338) — keep the tail, leading `…`. Only `fit_title`'s trailing context (the Local pane cwd).

Titles reaching `render_list_pane` are always `PaneTitleVm`; a single-segment title is equivalent to `fit_block_title` (both end in `truncate_end` at width − 2).

Narrow-terminal reflow (issue #342), also in `@src/tui/render/mod.rs` — different jobs from ellipsis, do not merge:

- **`wrap_hanging`** — wrap a line to width, continuing at the source line's leading whitespace. Help body and gist-comment bodies (pre-wrap at paint; do not use `Paragraph` wrap, which drops indent).
- **`fit_hints`** — drop whole ` · `-separated footer items so a coloured hint line stays one row; the last item (leave key) is kept. Status messages still wrap via `wrap_line_count`.
- **`fit_top_bar`** (issue #371) — the right-aligned shortcuts keep the row; the `gistui` brand is decoration and is dropped whole when the leftover cannot hold it plus a one-cell gap. Whole shortcuts drop from the left if they cannot all fit; the last remaining shortcut is marked with `truncate_end` if even it cannot fit (same last-survivor rule as `fit_hints`). Do not reuse `truncate_end` on the brand — `gis…` is worse than no name.

## Terminal lifecycle

`run()` wraps `run_loop()` so raw mode / alternate-screen teardown **always** runs. Keep fallible startup/IO inside `run_loop`, never between `enable_raw_mode` and teardown.

## Safety seams (rich refs)

- Download overwrite gate (issue #246): `@src/actions.rs` — `DownloadMode` / `OverwriteConfirmed`.
- Injectable `gh` boundary (issue #245): `CommandRunner` + `actions::test_support::SeqRunner`; fixtures in `tests/fixtures/gh/`.
- Gold-style TUI pure logic: `@src/tui/tests.rs` (no network).
- E2E frames: `@scripts/demo/` (real binary + fake `gh` + fake cwd).
