# Changelog

All notable changes are summarised here. Each version links to its full,
auto-generated notes on the [GitHub Releases][releases] page, which remains the
authoritative source.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
(currently `0.x` while the keymap/feature surface is still evolving).

## [Unreleased]

## [0.19.0] — 2026-08-29

- The interface drops its emoji (GitHub's release-note categories keep theirs). Gist rows now read `3 files`, `1 comment`, and a trailing
  age instead of picture glyphs, so their columns no longer drift out of alignment; the
  anchor pane is marked `⚑`, a pinned pair `↔`, and empty lists and the update notice say
  what they mean in words. `docs/design.md` records the marks the UI may use and why.
- Confirm prompts now separate the question from the answer: the action and its consequence
  on their own lines, then the keys that resolve it, each labelled with the verb it performs
  (`y  delete`, `y  upload`) instead of a bare `(y/n)`. Destructive confirms lead with
  `n  cancel` and say what cannot be undone.
- The README is rebuilt around installing and completing one gist operation, and is now a
  tagline, one paragraph on why the tool exists, the demo, one install path, one quick start
  and a list of links. The keymap, the mouse table and the configuration reference live in
  `?` Help, the project page and `config.example.toml` rather than being duplicated on the
  front page; `NO_COLOR` and what the update check does and does not send moved to
  `docs/SAFETY.md`.
- The project page is rebuilt around the demo, then why the tool exists, then the keys. Its
  two screenshots were unreadable — two full-width terminal stills squeezed side by side into
  the text column and cropped past their own footer — and are now one frame switched by the
  key that opens each screen in the app, wide enough to read and pannable on a phone.
- Deleting a Gist no longer leaves its files available through the preview cache, and a failed
  preview refresh keeps the last usable cached content for the next preview.
- Settings now reliably remember the "Show full diff" choice alongside the other preferences.
- Gist refreshes now keep the last usable list and counts when one GitHub request fails, and a
  superseded refresh can no longer overwrite newer results.
- Mouse double-clicks now respect close buttons, repository links, and top-bar shortcuts instead
  of falling through to the row underneath.
- Command-palette mouse clicks now execute the row under the pointer when disabled commands are
  visible above it.
- Revision history now scrolls like every other list: moving the selection resets the horizontal
  scroll, and scrolling right stops at the selected row's own text instead of running unbounded.
- A failed local file scan now keeps the last known list of files instead of clearing it, ends the
  scanning spinner instead of leaving it running forever, and reports the failure in the status
  line; a successful scan no longer erases a status message set by something else in the meantime.
- Unpinning a file no longer forces a non-recursive rescan — the file list, and its title's
  recursive/flat mode, stay consistent with whatever mode was actually active.

## [0.18.0] — 2026-08-21

- The main screen's two file lists can now be resized with the mouse: hold the left button on
  the divider between them and drag left/right, or double-click it to restore the default
  40/60. The width lasts for the session and is not written to the config file.
- Narrow terminals no longer cut the top-bar `gistui` brand mid-word: the shortcuts keep the
  row, and the name gives way rather than being painted over.
- The `?` help now documents `Tab` on the Help screen (it opens the topic index), and the
  command palette accents `Smart-sync`, `Fork gist`, and `Restore revision` as the writes they
  are — `Pin / unpin pair` no longer reads as a destructive action.
- Revision history rows now behave like every other list screen: a clipped row ends in an
  ellipsis (`…`) instead of looking complete, rows are clamped to the pane width, and
  `←`/`→` scrolls only the selected row behind a leading `…` rather than shifting the
  whole list.
- The quit prompt now names every key that confirms it (`q` and `Esc`), the top-bar `(g)ists` mnemonic now matches the lowercase key that actually opens it, and a command-palette query with no matches now shows an explicit "no matches" message instead of an empty frame.
- Preview now titles itself with the gist's description (consistent with Gist detail); Gist manager and Pinned Mappings rows now lead with the description instead of the raw id, which is demoted to a fixed-width, `#`-abbreviated column that stays aligned even with a starred/forked badge or a legacy short id.
- The diff header now shows the gist file's real update time (matching Gist manager and Pinned Mappings) whenever it's already loaded in memory, instead of always showing `(unknown)`.
- Gist detail now shows every file's size and type, the total size in its Files title, and the comment total in its tab.
- Saving configuration now records only changed settings (plus pinned mappings), so later default changes apply to users who have not overridden them.
- Recursive file discovery now includes hidden files and directories like the flat view, skips only configured directories plus `.git`, `.hg`, and `.svn`, and collapses symlink aliases without losing pinned paths.
- Help's topic index now gives General a working `g` shortcut, keeps every shortcut to one key, and aligns topic names.
- List, Pinned Mappings, and Gist manager now show contextual action hints; Pinned Mappings also explains its sync-status glyphs in the footer.
- Light theme now clearly distinguishes focused-pane and inactive-pane borders, and keeps the diff `gist` label distinct from subdued text.
- Narrow terminals (down to 80 columns) no longer clip the `?` help body mid-word, cut a footer hint in half (the leave key stays; whole hints drop instead), or drop the hanging indent of wrapped gist comments.
- Horizontal scroll in list panes now moves only the selected row, capped to that row's content. Other rows stay readable from their start, and a leading ellipsis (`…`) marks when the selection is offset.
- Clipped list rows and pane titles now end in an ellipsis (`…`) instead of looking like the real, complete value. Truncation follows display width, so wide characters are never split.
- Narrow List panes no longer clip the sort mode, filter, or `⚓` anchor out of their titles: the working directory gives way first and the pane name (`Local` / `Gists`) second, without leaving a dangling `·` behind.
- Downloading from a diff against a recursively discovered nested file now preserves that file's directory and keeps it visible after the local list refreshes.
- Deleting a gist from its file list view now returns to whichever screen you opened it from (Gist Manager, Pins, etc.) instead of always jumping to the main list.

## [0.17.1] — 2026-08-04

- Starred gist rows can fully horizontal-scroll again: the scroll limit now uses the same display string as painting (including the `★ ` prefix), so the trailing characters are reachable.
- Context menu / command palette enablement now shares its logic with the real key handlers, fixing a few spots where the two had drifted: `p` (pin) and Pins' `Enter` (diff) now correctly account for gist ownership and file previewability; Detail's `m` (load older comments) and Revisions' `F` (cycle target file) now match their real keys' focus/file-count requirements exactly; List's `S` (smart-sync) is no longer disabled for a not-yet-pinned pair (pressing it still reports "pair is not pinned" as before).

## [0.17.0] — 2026-07-09

- The top-right shortcut bar now includes `(C)onfig` immediately left of `(?)Help` (click or press `C`, same as duodiff).
- Settings screen (`C`, or Ctrl+p → Open settings): toggle theme, mouse, update checks, trailing-newline ignore, and adjust scan depth / diff context without hand-editing `config.toml` (saved only after a change).
- List navigation, palette enablement, and the dual-pane render build each ranked file list at most once per action (smoother with large gist/local sets).
- Cancelled or superseded background tasks (and overlapping local file scans) no longer apply their results if they finish late.
- Preview, diff, and upload refuse text larger than 10 MiB with a status message instead of loading the whole buffer into memory.
- Startup fetches for owned gists, starred gists, and the current user run in parallel, so cold start is faster on large accounts.

## [0.16.0] — 2026-07-08

- The footer no longer draws a horizontal rule above its text — one fewer row of chrome on every screen that shows a footer hint or status.
- A unified context menu (`;` or right-click) and command palette (`Ctrl+p`) list action verbs for the current screen — the menu shows only what's valid for your selection; the palette shows everything plus cross-screen shortcuts (`Go to Pins`, `Toggle theme`, `Quit`, …) with fuzzy-filter search. The idle footer now hints `; Menu · Ctrl+p Palette`.
- Every screen now shows a `(G)ists (P)ins (?)Help` shortcut bar in the top-right corner (click, or use the existing `g`/`P`/`?` keys, from any screen); the footer's long per-screen hotkey list is gone and it now fully collapses when idle (no divider, no blank row — the space goes back to content), since Help discoverability lives in the top bar (press `?` for the full keymap).
- The app version, the GitHub repo link (click to open in the browser), and update-check status have moved from the footer into a new **About** topic in `?` Help (press `0` from the Help index, or `Tab` then scroll to it).
- The `?` Help topic index is now clickable: click a row to select it, double-click (or `Enter`) to open it — matching every other list in the app.
- The context menu (`;` / right-click) now aligns key hints and action labels in separate columns so longer keys (`Enter`, `Space`, `Ctrl+p`, …) no longer run into the description.

## [0.15.2] — 2026-07-08

- The footer no longer shows the app version number (repo URL stays); the in-app update check already surfaces version freshness, and printing the version there forced a demo GIF/PNG re-recording on every release.

## [0.15.1] — 2026-07-08

- The Homebrew formula (`akunzai/tap/gistui`) now installs a prebuilt binary instead of compiling from source, cutting install time from 1-2 minutes to a few seconds; `brew install --HEAD` still builds from source.

## [0.15.0] — 2026-07-02

- Editing the upload redact buffer (`e` on the upload confirm) in a GUI editor (`zed`, `code`, `cursor`, `subl`, …) now live-updates the diff as you save, instead of only refreshing after you close the editor; `y`/`e` are disabled until you do. Terminal editors are unchanged.
- Fixed: a diff line whose old-side text has no trailing newline (and is no longer the file's last line) no longer merges with the line that follows it into one malformed row in the diff view.
- Fixed: completing or cancelling an upload started from the Pins view now stays on the Pins view instead of returning to the File List view.
- Pins view: a pinned entry whose local file no longer exists on disk is now flagged in the list (a distinct `✕` icon and a red row) instead of only surfacing as an error after you select or act on it.
- Pins view: a pinned entry whose content is actually identical on both sides now shows the synced (`✓`) indicator instead of a misleading push/pull arrow, even if the local and gist timestamps differ.

## [0.14.2] — 2026-06-25

- Fixed: editing the upload buffer (`e`) or a local file with a GUI editor (`zed`, `code`, `cursor`, `subl`, …) now works even when `$EDITOR`/`$VISUAL` omits a wait flag — gistui adds `--wait` automatically, so it no longer reads the file back before you save and upload the pre-edit (un-redacted) content.

## [0.14.1] — 2026-06-23

- Fixed: a gist you own *and* starred no longer shows its files twice in the detail view (it was fetched by both the owned and starred list APIs and merged without deduplication).

## [0.14.0] — 2026-06-22

- Fixed: owned-fork detection now paginates past 100 gists (the `forked` filter no longer misses forks on large accounts), and a failed fork-detection query surfaces a `fork detection unavailable` hint instead of silently showing no forks.
- Fixed three rough edges: opening a gist in the browser (`o`) no longer briefly freezes the UI; upload/restore temp files are written to the system temp directory instead of the working directory (no stray `.gistui_*` dirs if the process is killed mid-op); and an unreadable local file now reports an error instead of silently showing the whole gist as additions on upload.
- Performance: syntax-highlighted diff and preview panes are memoised, so scrolling no longer re-tokenises the whole buffer on every frame — smoother on large, highlighted files.
- TUI polish: the Gists and Gist-detail footers now surface `y copy url`; destructive-key and comment-error colours follow the theme (keeping contrast in light mode); and the filter hint wording is consistent (`Enter apply`).
- Gist comments now load newest-first in pages of 30 (popular gists no longer dump every comment or silently stop at 100); `m` or clicking the top line loads 30 older comments. The Comments pane is restyled with a per-comment header, relative time, and a loaded-range title.
- Diff view: a difference that is *only* a file-final newline no longer shows as a phantom change and no longer forces an overwrite confirm (GitHub stores gists with a trailing newline that local files often lack); disable with `ignore_trailing_newline = false` for byte-exact diffs.

## [0.13.0] — 2026-06-20

- Mouse support: wheel scroll, click to focus/select, double-click to open, and a clickable close button on pop-up screens (on by default; disable with `mouse = false` or `--no-mouse`).
- Startup update check: on launch gistui checks GitHub (once a day, silently) for a newer release and shows a footer hint with the right upgrade command if one exists (disable with `check_updates = false` or `--no-update-check`).
- Diff view: `w` toggles soft line wrapping — long lines wrap to the width instead of needing horizontal scroll (mirrors the preview's wrap toggle).

## [0.12.0] — 2026-06-19

- Vim-style navigation: `h`/`j`/`k`/`l` move and scroll alongside arrow keys; `Ctrl+b` / `Ctrl+f` page up/down by 10 (same as PageUp/PageDown) on every scrollable screen, including the main list (Files/Gists panes), Pins, and Gist manager. Revision history is now `H` (was `h`); cycling the revision target file is `F` (was `f`).
- Fixed: `T` theme toggle stopped working after vim navigation began passing real key modifiers (Shift+T was ignored).
- README slimmed for scannability: installation and safety reference docs moved to `reference/` (kept out of GitHub Pages `docs/`); contributor/agent sections to `CONTRIBUTING.md` / `AGENTS.md`; Usage keeps essential keys and points to in-app `?` for the complete keymap. Landing page updated (install link-out, current gist-manager copy, doc footer links).
- Gist detail: `*` stars or unstars the open gist (footer hint + `★ starred` in the info line when starred); the info line also shows `☆ N` (stargazers), `⑂ N` (forks), and `💬 N` (comments) when non-zero.
- Edit description (`e`), compact revisions (`c`), and delete gist (`X`) moved from the gist manager to gist detail; they appear only for gists you own (silent no-op on others' gists — no read-only warning).
- Image and other binary gist files cannot be previewed or diffed in the TUI (detected via the list API MIME type and filename); the file list tags them `(binary)` and status suggests `o` (browser) or `d` (download) instead.
- Owned forks now appear under the `forked` visibility filter: the gist list API omits `fork_of`, so gistui detects them via GraphQL `isFork` and fills the upstream id from the full gist object (fixes missing forks such as old forked scripts that still show on gist.github.com).
- Revision diffs (incremental and vs current) fetch file content via `gist.githubusercontent.com/.../raw/{sha}/{file}` when the revision API returns HTTP 502 on large gists (same class of fix as preview `raw_url` fallback).
- `F` fork is available only in gist detail, and only for gists you do not own (removed from the main list and gist manager).
- Gist detail comments load only when you open the Comments tab (no upfront fetch on Enter). Gist manager and detail view show `@owner` on gists you do not own. Preview/download falls back to the list API `raw_url` when `gh gist view` fails (e.g. huge starred gists returning HTTP 502). Startup cache now includes starred gists and fork/comment/star counts; fork and stargazer counts refresh in the background so the list appears sooner.
- Starred and forked gists: `v` cycles five visibility modes (all / public / secret / starred / forked); `*` stars or unstars the context gist; `F` forks a gist you do not own into your account. Others' starred gists are read-only (preview, diff, download, browser) — pin, upload, delete, compact, and restore are blocked. The gist manager title shows your starred and owned-fork totals (`★` / `⑂`); rows show `☆ N` (GitHub stargazers), `⑂ N` (forks), and `💬 N` (comments) when non-zero.
- Gist revision history: press `H` on a gist file in the main list, gist manager, or gist detail view to browse revisions (newest first), show the incremental diff for a revision (`Enter`, parent → selected), diff against the current version (`D`), and restore a single file from an older revision (`r`, `y`/`n` confirm — creates a new revision, unlike `c` compact which deletes history). In revision history, `F` cycles the target file on multi-file gists. Revision diffs are read-only (no `d`/`u` download/upload).

## [0.11.0] — 2026-06-17

- Built-in light/dark colour theme: set `theme = "light"` in `config.toml` for terminals with a light background, or press `T` at any time to toggle and save instantly.
- Pre-built binary installs can self-upgrade from GitHub Releases: `gistui --upgrade` (latest), `gistui --upgrade --check` (compare only), and `gistui --upgrade --upgrade-version <tag>` (pin a release). Homebrew, Scoop (including the `scoop/shims/gistui.exe` PATH shim), and cargo installs are detected and pointed at their own upgrade commands instead.
- Pins screen: `o` cycles sort order (default / local path / gist filename); active sort shown in the title bar.
- Pins screen: after a `d` pull completes, the view stays on Pins instead of returning to the main list.
- Confirm overwrite prompt now shows `~`-shortened paths instead of full absolute paths.
- Fixed: pressing `u` or `d` in the diff screen opened from Pins (Enter or `d`-pull) now correctly targets the pin pair's local file instead of the Files-view selection; `record_pin_sync` also fires correctly after a confirmed pull.

## [0.10.0] — 2026-06-16

- `?` help is now contextual: it opens the current screen's keys (and is reachable from the Pins, Gist manager, and Gist detail screens, not just the list), with `Tab` to browse an index of all topics instead of scrolling one long page.
- Local file list now has a text filter: `/` filters the focused pane (Local matches path/filename, Gist matches description/id). Filtering supports typing-while-navigating (↑/↓), `Tab` to apply and switch panes, and `Backspace` on an empty query to exit.
- The Pinned Mappings screen (`P`) gained the same `/` text filter — matches the local path or gist filename, with live ↑/↓ navigation.
- Pin times are now consistent between the Pins list and the diff view: pins pointing outside the scanned working directory show the real local mtime (and a correct ↑/↓ sync status) instead of `?`, and the pin diff header shows the gist's update time instead of `unknown`.
- Inline text inputs (gist description editor and every `/` filter) are now full single-line editors: `←`/`→`/`Home`/`End` move the cursor and `Backspace`/`Del` delete around it, with a block cursor showing its real position — no more deleting back to fix an earlier character.

## [0.9.0] — 2026-06-14

- Gist detail view is now tabbed — a `Files │ Comments` strip under the basic info shows one
  pane at a time (opens on the Files tab; `Tab` switches), instead of stacking both panes.
- Scrollbar on the gist-detail comments pane (the Diff and Preview panes already had one).
- Item counts in the Local, Gists and Pins titles (e.g. `Gists (3/12)` when a filter is
  active, `(N)` otherwise), matching the existing `Files (N)` / `Comments (N)` style.
- Gist manager rows show a `💬 N` comment count for gists that have comments (drawn from the
  existing gist-list fetch — no extra API calls).
- Animated spinner on the scanning, loading and working states (replaces the static `⏳`),
  so long-running `gh` operations no longer look frozen.
- Install from crates.io: `cargo install gistui`, or `cargo binstall gistui` for the
  prebuilt release binaries.

## [0.8.0] — 2026-06-14

- Scrollbars on the Diff and Preview panes.
- `~`-shortened local paths with scrollable Pins rows.
- Bounded (LRU) gist preview cache.
- Surface local file-read errors instead of a misleading empty diff.
- Write redact buffers to the system temp dir.
- Homebrew install: `brew install akunzai/tap/gistui`.
- Crate metadata, README badges, and this changelog.

## [0.7.0] — 2026-06-12

- Cursor-based file selection in the gist detail view.
- Preview line-wrap toggle.
- Copy gist URL / file content to the clipboard.
- Syntax-highlighted preview and diff.
- PageUp/PageDown scrolling.
- GitHub Pages landing page.

## [0.6.0] — 2026-06-12

- Preview file content with number keys in the detail view.
- HTTPS clone during compaction to avoid SSH passphrase prompts.

## [0.5.0] — 2026-06-11

- Gist detail view with comments.
- Anchor-driven list ranking with pinned/same-name markers.
- Windows key-repeat fix.

## [0.4.0] — 2026-06-11

- UI refresh.
- Gist revision compaction.
- Quit guard.
- Pane-oriented Enter diff preview.

## [0.3.0] — 2026-06-10

- Gist-level manager (edit description, remove a file, sort/filter).
- Create with a description.
- Fully async per-action `gh` fetches.
- Edit/redact before upload with JSON pretty-print.
- One-key pinned sync.
- Collapsible diff context.
- Working-directory path argument.

## [0.2.0] — 2026-06-09

- Paginate beyond 100 gists.
- Delete with confirmation.
- Cross-platform release binaries.
- ratatui 0.30 migration.
- Recursive discovery toggle.
- A pins view.
- Word-level inline diff highlighting.

## [0.1.0] — 2026-06-09

- Initial MVP: browse and rank gists against the working directory.
- Coloured diff.
- Download/upload/create/pin/preview.
- Filtering and sorting.
- Off-thread loading with an on-disk cache.
- Overwrite-confirm safety gate.

[unreleased]: https://github.com/akunzai/gistui/compare/v0.19.0...HEAD
[0.19.0]: https://github.com/akunzai/gistui/releases/tag/v0.19.0
[0.18.0]: https://github.com/akunzai/gistui/releases/tag/v0.18.0
[0.17.1]: https://github.com/akunzai/gistui/releases/tag/v0.17.1
[0.17.0]: https://github.com/akunzai/gistui/releases/tag/v0.17.0
[0.16.0]: https://github.com/akunzai/gistui/releases/tag/v0.16.0
[0.15.2]: https://github.com/akunzai/gistui/releases/tag/v0.15.2
[0.15.1]: https://github.com/akunzai/gistui/releases/tag/v0.15.1
[0.15.0]: https://github.com/akunzai/gistui/releases/tag/v0.15.0
[0.14.2]: https://github.com/akunzai/gistui/releases/tag/v0.14.2
[0.14.1]: https://github.com/akunzai/gistui/releases/tag/v0.14.1
[0.14.0]: https://github.com/akunzai/gistui/releases/tag/v0.14.0
[0.13.0]: https://github.com/akunzai/gistui/releases/tag/v0.13.0
[0.12.0]: https://github.com/akunzai/gistui/releases/tag/v0.12.0
[0.11.0]: https://github.com/akunzai/gistui/releases/tag/v0.11.0
[0.10.0]: https://github.com/akunzai/gistui/releases/tag/v0.10.0
[0.9.0]: https://github.com/akunzai/gistui/releases/tag/v0.9.0
[0.8.0]: https://github.com/akunzai/gistui/releases/tag/v0.8.0
[0.7.0]: https://github.com/akunzai/gistui/releases/tag/v0.7.0
[0.6.0]: https://github.com/akunzai/gistui/releases/tag/v0.6.0
[0.5.0]: https://github.com/akunzai/gistui/releases/tag/v0.5.0
[0.4.0]: https://github.com/akunzai/gistui/releases/tag/v0.4.0
[0.3.0]: https://github.com/akunzai/gistui/releases/tag/v0.3.0
[0.2.0]: https://github.com/akunzai/gistui/releases/tag/v0.2.0
[0.1.0]: https://github.com/akunzai/gistui/releases/tag/v0.1.0
[releases]: https://github.com/akunzai/gistui/releases
