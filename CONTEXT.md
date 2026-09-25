# gistui

A terminal UI for browsing, comparing, and managing GitHub Gists.

## Language

**Gist mutation**:
A change to a gist itself (create, delete, edit a file, description, star, fork, compact, upload-replace). Its async outcome belongs to no single screen — List, Gists, GistDetail, and Confirm can all launch one.
_Avoid_: screen action, background job result, apply handler

**Gist catalog**:
The publishable, cacheable collection of owned and starred Gists together with the account and enrichment metadata needed to browse them. A refresh may publish newer stages of one catalog over time, but never mixes stages from different refreshes.
_Avoid_: gist list, cache snapshot, fetch result

**Gist revision**:
An immutable historical state of a Gist. Restoring a file from one writes that content as a new Gist revision; it never rewrites existing history.
_Avoid_: version snapshot, rewritten revision

**Upload draft**:
One upload awaiting confirmation: its target (local file ↔ gist file) and its pending content — the local file as read, plus any redact edit, JSON pretty/sort, and line-ending normalization. It lives and dies with Confirm; confirming fixes it into the exact bytes sent, and nothing reads Confirm after that.
_Avoid_: upload state, upload buffer

**Sync policy**:
The content rules for syncing a local file with a gist file: which bytes an upload or create sends, which bytes a download writes, when the two sides count as identical, and how the diff between them reads. The `normalize_line_endings` and `ignore_trailing_newline` settings decide it. A pin's baseline is the local file's bytes on disk after a sync. Restoring a Gist revision copies gist to gist, so its payload is not a sync.
_Avoid_: normalization helper, line-ending logic
