# Architecture (agent deep-dive)

Index: [`AGENTS.md`](../../AGENTS.md). Source of truth for types lives in the modules below — prefer reading them over re-deriving rules from prose.

## Pure / impure boundary

| Kind | Modules | Testing |
| --- | --- | --- |
| **Pure** (unit-tested) | `domain`, `config`, `ranking`, `local`, `diff`, `pins`, actions **plan/guard**, `tui::view_model`, `tui::list_ranking`, `tui::settings`, `tui::gist_content`, `tui::local_scan` | In-crate unit tests |
| **Impure** (thin IO) | `gh`, actions **execute**, `tui::run_loop` / `tui::bg` / `tui::gist_refresh` / `tui::gist_revision` / `tui::pin_sync` | No live `gh`. Spawn/absorb is thin IO; action-job `on_*` apply handlers (screen modules / `gist_mutation.rs`) are unit-tested (#298, #383) |

`build_view_model` (`@src/tui/view_model.rs`): `AppState` + pin-sync cache → presentation facts. Paint helpers apply theme/layout only — no business rules, FS, or network.

## Runtime settings (`@src/tui/settings.rs`, issue #404)

- `RuntimeSettings` is the only runtime owner of the seven Settings-screen preferences and the `--no-mouse` / `--no-update-check` session overrides. Effective mouse and update-check values are derived; `Theme` is derived from `ThemeChoice`.
- Settings-screen edits, global theme toggle, and Diff context toggle all call `RuntimeSettings::adjust`. Only a mouse change returns `SettingsEffect::SyncMouseCapture`; dispatch owns that terminal IO.
- Persistence loads the current `AppConfig`, calls `apply_to_config`, then saves. That projection updates every runtime-owned field and leaves pins, skip directories, and other config data intact.

## Gist content store (`@src/tui/gist_content.rs`, issue #406)

- `GistContentStore` is the only owner of the 64-entry in-memory content LRU. Callers request
  `PreferCache` for Preview or `Refresh` for every explicit/fresh fetch; both miss paths hydrate
  a missing raw URL from `GistCatalog` before IO starts.
- `Refresh` bypasses but does not evict last-known-good content. Only a successful Preview fetch
  inserts its result. Failed, cancelled, or superseded work therefore leaves the store unchanged.
- Successful file-content mutations invalidate that file after upload, remove, or revision
  restore. Successful Gist deletion invalidates every file for the Gist. Metadata-only mutations
  (description, compact, star, fork) do not invalidate content.

## GistFile construction (`@src/domain.rs`, issue #379)

Three constructors, not one:

- **API mapper** (`parse_gist_list_json` in `@src/gh/gists.rs`) lists every field. A new metadata column must fail to compile there — do not fill from `Default`.
- **`for_sync`** — throwaway identity (`gist_id`, `filename`, `raw_url`) for sync/diff/upload. Production callers (`bg.rs`, `GistFileRef::to_gist_file`) stay on it.
- **`fixture`** — tests. Override non-default fields with struct-update syntax.

Fields stay `pub`. No `#[serde(default)]` on the struct.

## Screen state machine

- **`screens::lookup`** (`@src/tui/screens/mod.rs`, issues #377, #388) is the exhaustive match for the per-screen columns: help topic, wheel step, key guard, VM builder, key handler, navigation, and click selection. It first borrows `self.screen`, then calls an `fn(&mut AppState, …)` pointer: that two-phase pattern is #274's borrow rule. `render_screen_vm` matches `ScreenVm`, not `Screen`. `keymap::for_screen` stays in `keymap.rs` (bindings live there; putting them on the lookup would cycle `screens` ↔ `keymap`). No `ScreenModule` trait: the screen files already are the adapters.
- **`build_confirm_vm` is the exhaustive lookup for `PendingAction`** (`@src/tui/screens/confirm.rs`, issue #417). One arm per pending action carries its title, border colour, body (`ConfirmModalKind`) and background together; there is no `_` arm, so a new variant that forgets its row fails to compile instead of silently inheriting the overwrite gate's destructive prompt. `confirm_modal_style` and `view_model::confirm_prompt` are gone — they were two of the five wildcard-terminated matches this replaced. `handle_key_confirm` is the second (and last) exhaustive match: it still returns `KeyOutcome` (ADR-0002) and still clones the action, because the arms need `&mut AppState`; its `None` arm means "not on the Confirm screen", which `screens::lookup` never routes here. Presentation is asserted through `build_confirm_vm` only — do not reach past it to a per-fact helper.
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
handle_key (pure) → KeyOutcome → run_loop / dispatch_outcome (IO) → route_outcome
```

**On the `KeyOutcome` path, only `dispatch_outcome` touches the `Terminal`** (issue #421) —
`run_loop` still owns it for setup and `draw`. `dispatch_outcome` holds the arms that need one
(`EditUpload`, `EditLocal`, `PersistSettings`'s mouse-capture effect) and delegates every other
outcome to `route_outcome`, which takes `&mut AppState` and `&mut Jobs` and nothing else.

Before a fetch/action is spawned, its owning screen's `stage_*` function performs any
branching state work and returns the values dispatch needs. `route_outcome` then only
routes that payload to `Jobs`; it does not restage labels, return paths, caches, or loading flags.
In tests, `Jobs`' recording action-spawner adapter captures the reified job kind, progress
label, and gist-fetch payload without executing the worker closure (issue #422).

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

- **`Jobs`** is the single registry: spawn / absorb / cancel (`@src/tui/bg.rs`, issue #243). It keeps eight methods: `startup`, `spawn_action`, `spawn_gist_fetch_action`, `cancel_action`, `request_local_scan`, `set_upload_edit_watch`, `absorb`, and `command_runner` (the injected `CommandRunner`, see the Gist revision workflow below).
- **`GistRefresh` owns the whole-list pipeline** (`@src/tui/gist_refresh.rs`): it publishes the base `GistCatalog` as soon as owned/starred/login finish, then publishes fork counts, star counts, and fork metadata as coherent stages. Every result carries a refresh generation; only the latest generation may publish. A failed leg retains that field's last-known-good value, and the pipeline emits one aggregate status after the generation finishes.
- **`GistCatalog` is the publish/cache unit** (`@src/domain.rs`). Cache writes serialize one complete catalog stage from one generation; do not cache or publish the refresh module's individual legs.
- **Apply only marks; only the registry spawns** (issue #383). `on_*` handlers are free functions (`fn on_x(state: &mut AppState, ...) -> LoopFlow`) on the screen module that owns the payload they mutate, or on `@src/tui/gist_mutation.rs` when the outcome belongs to no single screen. They set `gist_list_stale` / `revisions_stale`; `Jobs::absorb` consumes those flags immediately after `on_action_outcome` and spawns. Do not spawn from apply. **Revisit** when a third kind of stale need appears: replace the flags with a described follow-up value rather than adding a third field.
- Action jobs and local scans use **generation supersession** (issue #221).
- **Action startup is an internal seam** (issue #422): `Jobs` receives an `ActionJobSpec`
  whose semantic kind carries the non-content identity needed to distinguish work (for
  example `ForkGist { gist_id }`, `FetchComments { gist_id, page }`, or `GistFetch(file)`)
  plus its progress label, then passes the opaque worker closure to an action-spawner adapter.
  Production uses `ThreadActionSpawner`; dispatch tests use the recording adapter and can
  assert the staged request without starting a thread or invoking `gh`; the Gist revision
  workflow's tests use `InlineActionSpawner`, which runs the real worker on the calling
  thread and queues its completion for a normal `absorb`. Each adapter has exactly one role —
  production concurrency, semantic routing observation, complete in-process execution — and
  all three stay private to `Jobs`; call sites still use only `spawn_action` /
  `spawn_gist_fetch_action`. Every revision job reifies as one `ActionJobKind::Revision(…)`
  whose payload the workflow owns.
- **Local-scan orchestration lives in `@src/tui/local_scan.rs`** (issue #409), separate from filesystem walking (`crate::local`) and thread/channel IO (`Jobs`, `@src/tui/bg.rs`). `ScanRequest` (cwd, pinned mappings, `ScanMode::{Flat,Recursive}`, skip dirs, max depth) is the one snapshot startup, `Jobs::request_local_scan`, and `bg::refresh_locals` all build via `AppState::local_scan_request` — `ScanMode` is a snapshot of `local_recursive`, not a live read, so an in-flight scan keeps the mode it started with. A private `LocalScan` (generation + in-flight) on `AppState` is mutated only through `begin_local_scan` / `apply_local_scan` / `end_local_scan`; a stale generation can end no in-flight state and apply no candidates. `apply_local_scan` is the one candidate-application operation background and synchronous paths share: it preserves the selected path (an explicit target — e.g. a just-downloaded file — beats whatever is selected at apply time), clears local hscroll unless that exact path survives, and re-clamps the gist cursor (index *and* hscroll) if reranking invalidated it. A current failure or channel disconnect ends in-flight state and reports it (`"local scan failed: …"` for the interactive scan, `"local refresh failed: …"` appended onto whatever status the caller already set for the synchronous post-download refresh) without touching last-known-good candidates; only success clears its own `SCANNING_STATUS` placeholder, never a newer status a later action wrote. Pin/unpin never rescans — it does not touch the filesystem, and ranking reads `PinnedMapping` directly — so `LocalCandidate.pinned` does not exist; recursive alias dedup (`crate::local::path_priority`) still prefers a pinned path by reading `PinnedMapping` itself.
- Call sites start work via `Jobs` methods (`spawn_action`, `request_local_scan`, …) or `gist_revision::dispatch` — do not own ad-hoc channel fields on `AppState`.
- `run_loop` only **polls** `jobs.absorb`.
- **Action jobs carry their apply** (issue #375, ADR-0002's async-response half): `spawn_action(spec, run, apply)` runs `run` off-thread and boxes `apply(value)` for the event-loop tick. There is no `BgTaskOutcome` enum. `ActionApply` is `FnOnce(&mut AppState) -> LoopFlow`. `on_action_outcome` is a generation-guard shell that calls the closure. `KeyOutcome` / `dispatch_outcome` stay plain data (ADR-0002).
- **`on_*` is the apply seam** (#298, #375, #383): named handlers are the apply bodies and the unit-test surface. They do not live on `Jobs`. `dispatch_outcome` / `route_outcome` sit on the spawn side, not the apply side, so neither is one. A new action is a spawn site plus an `on_*` when the apply is worth testing.
- **A pin's key is three-part** (issue #424): `(local_path, gist_id, gist_filename)`, owned by `@src/pins.rs` (`PinKey`, `PinKey::matches`, `PinnedMapping::key()`, `is_pinned` / `find_mut` / `upsert` / `remove`). One local file pinned to several gist files is a legitimate state, not corruption — `docs/design.md` defines a pin as a local-file to gist-*file* mapping, and `gist_id` alone cannot name a file inside a gist. `crate::actions`' `pin_mapping` / `unpin_mapping` / `record_sync` are thin wrappers that add persistence; never re-derive the key at a call site, and pass `PinnedMapping::key()` when you already hold the mapping. `upsert` reads `None` as "leave the stored value alone"; `record_sync` deliberately uses `find_mut` rather than `upsert`, because confirming a sync must never create a pin. **The one deliberate exception** is a call site that must resolve a possibly-relative stored `local_path` first (`@src/tui/bg.rs`'s `record_sync` caller, `@src/tui/dispatch.rs`'s `SyncSelectedPair`): those compare the *absolutised* path plus the two gist fields, so they still use all three components but cannot take a `PinKey` as-is. Exactly-duplicate triples are degenerate input from a hand-edited `config.toml`: the operations take the first match, and `crate::config::load_config` stays a parser, not a silent rewriter.
- **Shared spawn payload** (#375): a value both `run` and `apply` need is part of `run`'s return, unpacked by `apply`. That is how identity (`gist_id`, `fetch_id`, labels) crosses the thread boundary without a second clone.

## Gist revision workflow (`@src/tui/gist_revision.rs`, issue #430)

One interface owns every Gist revision flow. Callers submit one plain-data `RevisionRequest`
(ADR-0002: comparable, pattern-matchable, never a closure or a job handle) to
`gist_revision::dispatch`, which stages the action job, does the remote work through the
injected `CommandRunner`, and hands a **request-specific typed result** to the screen-owned
apply handler the job already carries. There is no umbrella revision-completion enum and no
result router.

Five flat request kinds: `FetchHistory`, `DiffAdjacent`, `DiffAgainstCurrent`,
`PreviewRestore`, `ExecuteRestore`. The four that act on a *file* share one `RevisionTarget`
— the `GistFileRef` plus owner login; `FetchHistory` is per-Gist and carries only a
`gist_id`. Versions, labels, the `DeferredEntry`, and restore content stay request-specific.
`RevisionTarget` carries a raw URL only for the requests that also fetch current content
(`history_and_current_target` vs. `history_target` on the Revisions screen).

**The split.** Screen intent (`screens::revisions`, `screens::confirm`) owns eligibility
guards, selection and parent lookup, ownership/previewability checks, return-entry capture,
raw-URL and owner snapshots, and intent-time labels; screen `on_*` handlers own every piece
of observable state application — navigation, status, Diff/Confirm entry, cache
invalidation, cursor and history reset, and the stale markers. The workflow owns job
staging, semantic job identity, exact progress labels, command execution, parsing, fallback
ordering, absent-file semantics, the per-buffer size gate, restore JSON and scratch
lifetime, and restore follow-up policy. **It never re-reads mutable UI state after receiving
a request** — that is what keeps a label the user pressed a key on from drifting with how
long the fetch took (`revision_version_label` takes `now` as an input, at intent time).

Invariants worth keeping:

- **Adjacent ordering** — selected child first, then its parent. A missing parent version
  *is* the initial revision and compares against empty content; on either optional side an
  absent historical file also reads as empty content.
- **Selected-versus-current ordering** — the historical side first and **required** (its
  absence stays an error), then current content with its command-to-raw-URL fallback. The
  historical side keeps its own entry-raw-URL / owner-aware-canonical fallback sequence.
- **The size gate applies to every fetched buffer independently.** Diff identical detection
  and the ignore-trailing-newline setting stay at screen apply time.
- **Restore never rewrites history**: it patches the historical content as a new Gist
  revision. Its scratch directory and JSON are prepared synchronously *before* the spawn, so
  a preparation failure keeps its own wording and neither starts nor supersedes a job; on
  success the `ScratchDir` moves into the worker, whose RAII drop runs after success,
  failure, and a completion the generation guard later ignores.
- **Restore follow-up is described, not re-derived.** Success returns `RestoreApplied` —
  the affected file plus the `RestoreRefresh` values the new Gist revision made stale, in
  the order `Jobs::absorb` consumes them. `on_restore_revision_done` walks that list and
  sets the markers; only `Jobs::absorb` consumes them and starts the work.

**Testing.** The seam is the complete path — request → workflow → `Jobs` → command-runner
adapter → `absorb` → apply handler — driven by `Jobs::inline` plus a scripted `SeqRunner`
(`Mutex`-guarded so an `Arc` of it can be shared with worker closures; it records exact
command order and, for `--input` plans, the payload body that only exists while the command
runs). The real `ScratchDir` is used, not a filesystem seam. Do not layer duplicate
implementation-detail assertions beneath this seam: parser and command-plan tests stay in
`gh` / `actions`, screen intent and presentation tests stay on their screens.

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

**Both List panes are `ListCursor`** (`@src/tui/list_cursor.rs`, issue #415), reached through `AppState::focused_cursor` / `focused_cursor_mut` — the one seam where `focus` picks a pane, so a navigation operation is added once rather than once per pane. The `local_index` / `gist_index` / `local_hscroll` / `gist_hscroll` fields and the `scroll_focused_left` / `scroll_focused_right` methods are gone; `right` takes its cap from `focused_hscroll_max()`, computed before the borrow (#274). `ListCursor` owns one policy only — a vertical move clears the offset — so everything a cursor move *triggers* stays on the screen: `list_move_focused` / `list_page_focused` / `click_select_list` keep the anchor re-rank (a single step that hits a bound re-ranks nothing; a page always does), and `apply_local_scan` keeps path-identity resolution, calling `select` when it lands on a different row and assigning the index directly when the previously selected path survived. `anchor` and `list_ranking` only *read* `cursor.index`.

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
- `MouseFrame::resolve_pane` is the seam `ScreenLookup::click_select` consumes (issue #408):
  it walks only `register_pane` hits and returns a resolved `RowTarget::Pane { pane, index }`,
  never raw `(col, row)`. It deliberately ignores `Divider`/other whole-body `Rect` hits —
  `HitTarget::Divider` registers over the List screen's *entire* body (feeding `MouseFrame::split`,
  which just returns the most recently registered `SplitHit` regardless of point containment), far
  wider than its visual grab zone, and would otherwise mask every pane row underneath it if
  `resolve` (the priority-ordered `Rect` resolver used for top-bar/tab targets) were reused here.
  Per-screen `click_select_*` functions match on `pane`/`index` only — List routes Local vs. Gist
  focus and anchor re-ranking; Pins/Gists/Revisions/Help/Config select on their single `PaneTarget::List`;
  GistDetail focuses Files on any hit in `PaneTarget::DetailFiles`, moving `file_cursor` only when
  `index` is `Some`.

List-row horizontal scroll (issue #341) is **per selected row**, not pane-wide: `row_hscroll` applies the pane offset only to the highlighted index; `focused_hscroll_max` / Pins / Gists / Revisions caps are that row's painted string. A non-zero offset prefixes `…` in `visible_list_row` (then `truncate_end` may still mark a clipped tail). Unselected rows stay at column 0. `RevisionState.cursor` is a `ListCursor` like Pins/Gists (issue #408): vertical/page moves and row clicks reset `hscroll`, and Right clamps to `revisions_hscroll_max` (the selected row's rendered label) instead of growing unbounded.

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
- Injectable `gh`/`git` boundary (issue #245, #386, #419): `foo(runner)` with `CommandRunner` first; adapters `SystemRunner` / `SeqRunner`. Every command path goes through it — reads, write actions, and gist compaction (`compact_in_dir`) alike. The seam expresses spawn-and-capture only; the few paths it cannot express are named on the `CommandRunner` doc in `@src/actions.rs`, and that list is the whole set. Fixtures in `tests/fixtures/gh/`.
- Gold-style TUI pure logic: each `tui` module's own `#[cfg(test)] mod tests` (no network); shared `AppState` fixtures live in `@src/tui/test_support.rs`.
- E2E frames: `@scripts/demo/` (real binary + fake `gh` + fake cwd).
