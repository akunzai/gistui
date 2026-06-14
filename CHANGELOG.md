# Changelog

All notable changes are summarised here. Each release links to its full,
auto-generated notes on the [GitHub Releases][releases] page, which remains the
authoritative source.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
(currently `0.x` while the keymap/feature surface is still evolving).

## [Unreleased]

- Write the redact edit buffer to the system temp dir so leftover files never
  pollute the working directory.
- Surface local file-read errors on the status line instead of showing a
  misleading empty diff.
- Show a scrollbar on the Diff and Preview panes so position in long diffs and
  large files is visible.
- Bound the in-memory gist preview cache (LRU, 64 entries) so browsing many or
  large gists no longer grows memory without limit.
- Project metadata, README badges, and this changelog for discoverability.

## [0.7.0] — 2026-06-12

Cursor-based file selection in the gist detail view, preview line-wrap toggle,
copy gist URL / file content to the clipboard, syntax-highlighted preview and
diff, and PageUp/PageDown scrolling. Plus a GitHub Pages landing page.
[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.7.0).

## [0.6.0] — 2026-06-12

Preview file content with number keys in the detail view; HTTPS clone during
compaction to avoid SSH passphrase prompts.
[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.6.0).

## [0.5.0] — 2026-06-11

Gist detail view with comments, anchor-driven list ranking with pinned/same-name
markers, and Windows key-repeat fix.
[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.5.0).

## [0.4.0] — 2026-06-11

UI refresh, gist revision compaction, quit guard, and pane-oriented Enter diff
preview.
[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.4.0).

## [0.3.0] — 2026-06-10

Gist-level manager (edit description, remove a file, sort/filter), create with a
description, fully async per-action `gh` fetches, edit/redact before upload with
JSON pretty-print, one-key pinned sync, collapsible diff context, and a
working-directory path argument.
[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.3.0).

## [0.2.0] — 2026-06-09

Paginate beyond 100 gists, delete with confirmation, cross-platform release
binaries, ratatui 0.30 migration, recursive discovery toggle, a pins view, and
word-level inline diff highlighting.
[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.2.0).

## [0.1.0] — 2026-06-09

Initial MVP: browse and rank gists against the working directory, coloured diff,
download/upload/create/pin/preview, filtering and sorting, off-thread loading
with an on-disk cache, and the overwrite-confirm safety gate.
[Full notes](https://github.com/akunzai/gistui/releases/tag/v0.1.0).

[releases]: https://github.com/akunzai/gistui/releases
