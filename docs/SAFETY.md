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
  sensitive local content or credentials are not accidentally pushed to GitHub.
- Identical files are detected: when the two sides match, upload/download are disabled.

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