# Changelog

All notable changes are summarised here. Each release links to its full,
auto-generated notes on the [GitHub Releases][releases] page, which remains the
authoritative source.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
(currently `0.x` while the keymap/feature surface is still evolving).

## [Unreleased]

- Item counts in the Local, Gists and Pins titles (e.g. `Gists (3/12)` when a filter is
  active, `(N)` otherwise), matching the existing `Files (N)` / `Comments (N)` style.
- Gist detail view is now tabbed — a `Files │ Comments` strip under the basic info shows one
  pane at a time (opens on the Files tab; `Tab` switches), instead of stacking both panes.
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

[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.8.0).

## [0.7.0] — 2026-06-12

- Cursor-based file selection in the gist detail view.
- Preview line-wrap toggle.
- Copy gist URL / file content to the clipboard.
- Syntax-highlighted preview and diff.
- PageUp/PageDown scrolling.
- GitHub Pages landing page.

[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.7.0).

## [0.6.0] — 2026-06-12

- Preview file content with number keys in the detail view.
- HTTPS clone during compaction to avoid SSH passphrase prompts.

[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.6.0).

## [0.5.0] — 2026-06-11

- Gist detail view with comments.
- Anchor-driven list ranking with pinned/same-name markers.
- Windows key-repeat fix.

[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.5.0).

## [0.4.0] — 2026-06-11

- UI refresh.
- Gist revision compaction.
- Quit guard.
- Pane-oriented Enter diff preview.

[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.4.0).

## [0.3.0] — 2026-06-10

- Gist-level manager (edit description, remove a file, sort/filter).
- Create with a description.
- Fully async per-action `gh` fetches.
- Edit/redact before upload with JSON pretty-print.
- One-key pinned sync.
- Collapsible diff context.
- Working-directory path argument.

[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.3.0).

## [0.2.0] — 2026-06-09

- Paginate beyond 100 gists.
- Delete with confirmation.
- Cross-platform release binaries.
- ratatui 0.30 migration.
- Recursive discovery toggle.
- A pins view.
- Word-level inline diff highlighting.

[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.2.0).

## [0.1.0] — 2026-06-09

- Initial MVP: browse and rank gists against the working directory.
- Coloured diff.
- Download/upload/create/pin/preview.
- Filtering and sorting.
- Off-thread loading with an on-disk cache.
- Overwrite-confirm safety gate.

[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.1.0).

[releases]: https://github.com/akunzai/gistui/releases
