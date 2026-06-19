# Changelog

All notable changes are summarised here. Each version links to its full,
auto-generated notes on the [GitHub Releases][releases] page, which remains the
authoritative source.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
(currently `0.x` while the keymap/feature surface is still evolving).

## [Unreleased]

- Revision diffs (incremental and vs current) fetch file content via `gist.githubusercontent.com/.../raw/{sha}/{file}` when the revision API returns HTTP 502 on large gists (same class of fix as preview `raw_url` fallback).
- `F` fork is available only in gist detail, and only for gists you do not own (removed from the main list and gist manager).
- Gist detail comments load only when you open the Comments tab (no upfront fetch on Enter). Gist manager and detail view show `@owner` on gists you do not own. Preview/download falls back to the list API `raw_url` when `gh gist view` fails (e.g. huge starred gists returning HTTP 502). Startup cache now includes starred gists and fork/comment counts; fork counts refresh in the background so the list appears sooner.
- Starred and forked gists: `v` cycles five visibility modes (all / public / secret / starred / forked); `*` stars or unstars the context gist; `F` forks a gist you do not own into your account. Others' starred gists are read-only (preview, diff, download, browser) — pin, upload, delete, compact, and restore are blocked. The gist manager title shows your starred and owned-fork totals (`★` / `⑂`); rows show `⭐ N` when a gist has forks.
- Gist revision history: press `h` on a gist file in the main list, gist manager, or gist detail view to browse revisions (newest first), show the incremental diff for a revision (`Enter`, parent → selected), diff against the current version (`D`), and restore a single file from an older revision (`r`, `y`/`n` confirm — creates a new revision, unlike `c` compact which deletes history). In revision history, `f` cycles the target file on multi-file gists. Revision diffs are read-only (no `d`/`u` download/upload).

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

[unreleased]: https://github.com/akunzai/gistui/compare/v0.11.0...HEAD
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
