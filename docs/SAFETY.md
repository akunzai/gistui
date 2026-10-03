# Safety rules

## Read-only gists

Gists you do not own (e.g. from the starred filter) are **read-only**: you can preview, diff,
download, and open in the browser, but pin, upload, and remove-file are refused with a status
message. Edit description, compact, delete, and revision restore are only offered in **gist
detail** for gists you own; on others' gists those keys are hidden (silent no-op). Open gist
detail and press `F` to fork one into your account.

Star/unstar (`*`) and fork (`F`) are remote structure writes; they do not overwrite local files.

## Local writes

- Downloads only ever write to `./<gist-filename>` in the current working directory.
- An existing file (local download target or remote gist file) is never overwritten without
  first showing its diff and confirmation. Confirmations appear as a centered prompt over
  the full-screen diff, so the change you are approving stays visible while you decide.
- Pulling a gist over an existing local file still goes through the diff + `y`/`n`
  confirmation — one-key sync never overwrites a local file silently.

## Uploads

- Uploads allow editing/redacting a temporary buffer in `$EDITOR` before sending, ensuring
  sensitive local content or credentials are not accidentally pushed to GitHub. That buffer
  holds the content before redaction, so it lives in a private scratch directory (owner-only on
  Unix; Windows' temp dir is per-user), is created fresh rather than reusing any existing file,
  and is removed when editing ends — including when gistui quits with the editor still open.
- Identical files are detected: when the two sides match, upload/download are disabled.

## Staged hunk sync

- Hunk copies change only the in-memory Local and Gist buffers. Undo reverses a staged copy;
  leaving with unsaved copies offers Save, Discard, or Cancel, including Quit from the palette.
- Save previews each changed side against its saved content and requires `y save`. Both live
  sides are re-read before writing; if either changed, the save stops and retains the buffers.
- Local and Gist cannot be written as one transaction. A partial failure reports which side
  succeeded and keeps the unfinished side staged. Undo history is cleared after a write so it
  cannot restore a snapshot of already-saved content.
- Unselected differences remain. A pin baseline advances only when the saved pair is identical
  under the Sync policy. Whole-file upload/download are unavailable while hunks are staged.

## Destructive remote actions

Each requires a `y`/`n` confirmation:

- Removing a file from a gist (`X` on the main list).
- Deleting a whole gist (`X` in gist detail).
- Compacting a gist's revisions (`c` in gist detail — a history-rewriting force-push; the
  confirmation prompt displays the gist's info so the target stays visible while you decide).

Restoring a file from an older revision (`r` in revision history) is also confirmed with a
full-screen diff, but it **adds** a new revision rather than rewriting history (the opposite
of `c` compact).

## Credentials and config

- No GitHub token is stored by the app, and gist content is never written to the config
  file — only path↔gist pin mappings are persisted.
- Clipboard copy (`y` URL, `Y` content) hands the text to the system clipboard via the OS
  tool, where other applications can read it. `Y` copies the full previewed file content, so
  treat it like any other paste of potentially sensitive data.

## What leaves the machine

- Every GitHub call goes through your own `gh`, so gistui sends nothing GitHub would not
  already see from the CLI.
- On startup gistui asks GitHub once a day whether a newer release exists and reports it in
  `?` Help → About. That is the only request it makes on its own: no telemetry, silent when
  offline, and off with `check_updates = false` or `--no-update-check`.
- Syntax highlighting honours [`NO_COLOR`](https://no-color.org). The semantic diff `-`/`+`
  colours stay, since the diff is unreadable without them.
