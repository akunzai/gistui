# Architecture (agent deep-dive)

Index: [`AGENTS.md`](../../AGENTS.md). Source of truth for types lives in the modules below — prefer reading them over re-deriving rules from prose.

## Pure / impure boundary

| Kind | Modules | Testing |
| --- | --- | --- |
| **Pure** (unit-tested) | `domain`, `config`, `ranking`, `local`, `diff`, actions **plan/guard**, `tui::view_model`, `tui::list_ranking` | In-crate unit tests |
| **Impure** (thin IO) | `gh`, actions **execute**, `tui::run_loop` / `tui::bg` / `tui::pin_sync` | No live `gh`. Spawn/absorb is thin IO; action-job `on_*` apply handlers (screen modules / `gist_mutation.rs`) are unit-tested (#298, #383) |

`build_view_model` (`@src/tui/view_model.rs`): `AppState` + pin-sync cache → presentation facts. Paint helpers apply theme/layout only — no business rules, FS, or network.

## GistFile construction (`@src/domain.rs`, issue #379)

Three constructors, not one:

- **API mapper** (`parse_gist_list_json` in `@src/gh/gists.rs`) lists every field. A new metadata column must fail to compile there — do not fill from `Default`.
- **`for_sync`** — throwaway identity (`gist_id`, `filename`, `raw_url`) for sync/diff/upload. Production callers (`bg.rs`, `GistFileRef::to_gist_file`) stay on it.
- **`fixture`** — tests. Override non-default fields with struct-update syntax.

Fields stay `pub`. No `#[serde(default)]` on the struct.

## Screen state machine

- **`screens::lookup`** (`@src/tui/screens/mod.rs`, issues #377, #388) is the exhaustive match for the per-screen columns: help topic, wheel step, key guard, VM builder, key handler, navigation, and click selection. It first borrows `self.screen`, then calls an `fn(&mut AppState, …)` pointer: that two-phase pattern is #274's borrow rule. `render_screen_vm` matches `ScreenVm`, not `Screen`. `keymap::for_screen` stays in `keymap.rs` (bindings live there; putting them on the lookup would cycle `screens` ↔ `keymap`). No `ScreenModule` trait: the screen files already are the adapters.
- **`Screen` is `Clone`, not `Copy`** — payload variants own screen-local UI (`@src/tui/mod.rs`, issue #242).
- **`List` stays a unit tag** — dual-pane selection / filters / sorts are session-global on `AppState` (user story 19).
- Other variants (`Diff`, `Confirm`, `Preview`, `Help`, `Config`, `Revisions`, `Pins`, `Gists`, `GistDetail`, `Palette`, …) carry payloads (body/scroll/return/origin as needed).
- **Diff/Confirm/Preview body+scroll** is `ScrollBody` (`@src/tui/scroll.rs`, issue #385), reached only via `scroll_body` / `scroll_body_mut`. The ten `diff_body_text` / `scroll_diff_*` methods are gone. Their `screens::lookup` rows share `scroll_navigation`; Help and Detail comments have no `ScrollBody`.
- **`nav_stack`** (issue #271) holds return targets; Esc pops. Prefer stack ops over parallel “return” root fields for new navigation.
- **Async screen entry is a moved value, not root staging.** `DeferredEntry` snapshots the
  return screen at intent time and moves through `KeyOutcome` and the job apply closure.
  Success consumes it through `open_deferred`; failure, cancellation, and generation
  supersession drop it without touching navigation. Preview refresh captures the current
  parent through `defer_replacement`, so refresh replaces Preview instead of stacking it.
  Diff Gist identity is built directly into `DiffState`; do not add another staging field.
- **`back_to_list()` is a hard reset** (clears `nav_stack`) — reserve it for paths whose only possible origin is `Screen::List` itself. Confirm-execute paths with more than one possible origin (e.g. `ExecuteDelete`, `ExecuteCompactGist` in `dispatch.rs`) use `leave()`/`cancel_confirm()` instead, to return to whichever screen actually launched them; pop an extra time if the popped screen is now stale (e.g. `GistDetail` for a gist just deleted).

## Key path (pure intent → impure dispatch)

```
handle_key (pure) → KeyOutcome → run_loop / dispatch_outcome (IO)
```

Before a fetch/action is spawned, its owning screen's `stage_*` function performs any
branching state work and returns the values dispatch needs. `dispatch_outcome` then only
routes that payload to `Jobs`; it does not restage labels, return paths, caches, or loading flags.

- New key logic → `AppState::handle_key` (testable).
- New IO → `dispatch` / `bg` helpers, not `handle_key`.
- IO-bearing `KeyOutcome` variants carry payloads (issue #244): `@src/tui/mod.rs` (`KeyOutcome`), `@src/tui/dispatch.rs`.
- Diff/Confirm/Preview scroll keys go through `scroll_body_mut` (issue #385), not per-axis `AppState` methods.

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

- **`Jobs`** is the single registry: spawn / absorb / cancel (`@src/tui/bg.rs`, issue #243). It keeps seven methods: `startup`, `spawn_action`, `spawn_gist_fetch_action`, `cancel_action`, `request_local_scan`, `set_upload_edit_watch`, `absorb`.
- **Apply only marks; only the registry spawns** (issue #383). `on_*` handlers are free functions (`fn on_x(state: &mut AppState, ...) -> LoopFlow`) on the screen module that owns the payload they mutate, or on `@src/tui/gist_mutation.rs` when the outcome belongs to no single screen. They set `gist_list_stale` / `revisions_stale`; `Jobs::absorb` consumes those flags immediately after `on_action_outcome` and spawns. Do not spawn from apply. **Revisit** when a third kind of stale need appears: replace the flags with a described follow-up value rather than adding a third field.
- Action jobs and local scans use **generation supersession** (issue #221).
- Call sites start work via `Jobs` methods (`spawn_action`, `request_local_scan`, …) or `screens::revisions::request_revisions` — do not own ad-hoc channel fields on `AppState`.
- `run_loop` only **polls** `jobs.absorb`.
- **Action jobs carry their apply** (issue #375, ADR-0002's async-response half): `spawn_action(run, apply)` runs `run` off-thread and boxes `apply(value)` for the event-loop tick. There is no `BgTaskOutcome` enum. `ActionApply` is `FnOnce(&mut AppState) -> LoopFlow`. `on_action_outcome` is a generation-guard shell that calls the closure. `KeyOutcome` / `dispatch_outcome` stay plain data (ADR-0002).
- **`on_*` is the apply seam** (#298, #375, #383): named handlers are the apply bodies and the unit-test surface. They do not live on `Jobs`. `dispatch_outcome` needs a `Terminal`, so it is not one. A new action is a spawn site plus an `on_*` when the apply is worth testing.
- **Shared spawn payload** (#375): a value both `run` and `apply` need is part of `run`'s return, unpacked by `apply`. That is how identity (`gist_id`, `fetch_id`, labels) crosses the thread boundary without a second clone.

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

**The two List panes are user-resizable** (issue #395): `AppState::list_split_percent` is the local pane's share, session-only (no config field, no Settings row, no key binding — a restart is back at `DEFAULT_SPLIT_PERCENT`). Percent is the stored fact but `render_list_vm` sizes the panes in **cells** (`split_cells` → `Length`/`Min`) — handing ratatui a second percentage makes the divider lag the pointer mid-drag. `clamp_split_percent` holds 15–85% *and* `list_pane::MIN_PANE_CELLS` on both sides, and returns `None` when no split leaves two readable panes. The drag policy stays on the List screen; `MouseSession` owns only its lifecycle. The highlight belongs to neither pane, so it is `highlight_pane_divider`, not a `ListPaneVm::focused` variant.

## Mouse interaction (`@src/tui/mouse.rs`)

- `MouseFrame` is rebuilt on every paint. Renderers register closed `HitTarget` values; its
  resolver owns cross-screen priority, independent of registration order. Palette overlays use
  `intercept_all`, and palette row targets carry their original item index.
- Screen adapters retain domain behavior: resolved row geometry still delegates selection and
  activation to the active screen. The mouse module knows no Gist or navigation policy.
- `MouseSession` owns facts that survive a frame: previous press identity and divider-drag state.
  Any key, right-click, release, or background-task takeover calls `interrupt()`; render only asks
  `is_dragging()`.
- Non-mouse render output belongs in `RenderFeedback`. In particular, comments scroll-to-bottom
  is consumed only after the comments renderer supplies `comments_max_scroll`.

List-row horizontal scroll (issue #341) is **per selected row**, not pane-wide: `row_hscroll` applies the pane offset only to the highlighted index; `focused_hscroll_max` / Pins / Gists caps are that row's painted string. A non-zero offset prefixes `…` in `visible_list_row` (then `truncate_end` may still mark a clipped tail). Unselected rows stay at column 0.

List-row budget: inner pane width minus borders+padding (`LIST_CHROME_CELLS`) minus `LIST_HIGHLIGHT_SYMBOL`. ratatui's default `HighlightSpacing::WhenSelected` indents **every** row once any row is selected — these lists always `select(...)` when they have rows, so unselected rows still pay the `▶ ` indent. Keep the widget's `highlight_symbol` and the budget on `LIST_HIGHLIGHT_SYMBOL` so they cannot drift.

## Text fitting (`@src/tui/render/text_fit.rs`)

Two ellipsis operations — different jobs, do not merge:

- **`truncate_end`** (issue #340) — keep the head, append `…`. List rows (`visible_list_row`), pane/overlay titles (`fit_block_title`), and a `fit_title` head that itself cannot fit.
- **`elide_start`** (issue #338) — keep the tail, leading `…`. Only `fit_title`'s trailing context (the Local pane cwd).

Titles reaching `render_list_pane` are always `PaneTitleVm`; a single-segment title is equivalent to `fit_block_title` (both end in `truncate_end` at width − 2).

Narrow-terminal reflow (issue #342), also in `@src/tui/render/text_fit.rs` — different jobs from ellipsis, do not merge:

- **`wrap_hanging`** — wrap a line to width, continuing at the source line's leading whitespace. Help body and gist-comment bodies (pre-wrap at paint; do not use `Paragraph` wrap, which drops indent).
- **`fit_hints`** — drop whole ` · `-separated footer items so a coloured hint line stays one row; the last item (leave key) is kept. Status messages still wrap via `wrap_line_count`.
- **`fit_top_bar`** (issue #371) — the right-aligned shortcuts keep the row; the `gistui` brand is decoration and is dropped whole when the leftover cannot hold it plus a one-cell gap. Whole shortcuts drop from the left if they cannot all fit; the last remaining shortcut is marked with `truncate_end` if even it cannot fit (same last-survivor rule as `fit_hints`). Do not reuse `truncate_end` on the brand — `gis…` is worse than no name.

## Render modules

`@src/tui/render/mod.rs` is the rendering façade: it paints the canvas and dispatches `ScreenVm`.
Keep focused helpers in its children: `@src/tui/render/labels.rs` owns gist/file/time and diff labels; `@src/tui/render/diff_view.rs` owns highlighted diff painting (screens enter through `render_diff_pane_vm`); and `@src/tui/render/chrome.rs` owns top bars, footers, modals, loading overlays, and palette rows.

## Terminal lifecycle

`run()` wraps `run_loop()` so raw mode / alternate-screen teardown **always** runs. Keep fallible startup/IO inside `run_loop`, never between `enable_raw_mode` and teardown.

## Safety seams (rich refs)

- Download overwrite gate (issue #246): `@src/actions.rs` — `DownloadMode` / `OverwriteConfirmed`.
- Injectable `gh` boundary (issue #245, #386): `foo(runner)` with `CommandRunner` first; adapters `SystemRunner` / `SeqRunner`. There is no second door. Fixtures in `tests/fixtures/gh/`.
- Gold-style TUI pure logic: each `tui` module's own `#[cfg(test)] mod tests` (no network); shared `AppState` fixtures live in `@src/tui/test_support.rs`.
- E2E frames: `@scripts/demo/` (real binary + fake `gh` + fake cwd).
