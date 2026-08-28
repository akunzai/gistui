# Demo recording harness

Regenerates the README demo (`website/demo.gif`) and still screenshots
(`website/gist-manager.png`, `website/revisions.png`) by driving the **real**
`gistui` binary against **fake** data using [`tcut`](https://tcut.amanv.dev),
fully scripted and reproducible in TypeScript — no real GitHub account, no
manual keypresses.

```bash
mise run demo
```

or directly:

```bash
tcut scripts/demo.video.ts    # website/demo.gif
tcut scripts/demo.stills.ts   # website/gist-manager.png, website/revisions.png
```

Both build `gistui` and record a headless session with Ghostty against the same
fake data. They are separate recordings because they need different terminal
heights: the GIF needs 30 rows for the two-pane browse view, while the gist
manager and the revision list draw a handful of rows and are recorded at that
height, so the project page can show them whole and large enough to read.

## Why this exists

A TUI is hard to screenshot consistently by hand, and recording against real
gists leaks private data and drifts every run. This harness pins the data and
the keystrokes, so:

- **Re-record after a UI change** — run `mise run demo` to get a fresh,
  identical-framing GIF and screenshots.
- **Deterministic fixtures** — the same fake gists + local files every time.
- **Single script** — driven by `scripts/demo.video.ts` using `tcut`'s
  screen-asserting driver and snapshot capability (`t.snapshot()`).

## How it works

`gistui` shells out to the GitHub CLI for everything, so we intercept `gh`:

| Piece | Role |
|-------|------|
| `fake-gh` | A stateful stand-in for `gh`. Implements only the commands gistui uses (`api /gists`, `api /gists/{id}/commits`, `api /gists/{id}/{version}`, `gist view/edit/create/delete`, `api PATCH`) over a JSON store, and mutates the store so uploads/downloads/deletes are reflected live. Revision history is read from a per-gist `commits` list in the store (a gist without one gets a single synthetic HEAD). Symlinked/copied to `gh` and put first on `PATH`. |
| `seed.py` | Writes the fake gist store + the local working-dir files into an isolated workspace. Content is crafted so a diff, an upload, and a download-overwrite are all meaningful. |
| `session.ts` | The setup both recordings share: build, fake `gh` on `PATH`, seed, and `cd` into the workspace. Runs inside `t.hide`. |
| `demo.video.ts` | The GIF: 100×30, drives the whole storyboard with keys and screen assertions (`t.wait`). |
| `demo.stills.ts` | The project-page stills: 100×10, opens the two screens and captures them with `t.snapshot`. |

The recording is isolated: a temp `$GISTUI_DEMO_HOME` holds the store, the
working dir, and a fresh `XDG_CONFIG_HOME` (so persisted pins never leak between
runs), and it is deleted on exit.

## Storyboard

Setup and the `gistui` launch run off-camera (`t.hide`), so the GIF opens on the
browse view instead of a shell prompt.

Browse with ranking → Revisions history snapshot (`H`) → Gist manager snapshot
(`g`) → pin a pair + the Pins view (`p`, `P`) → **syntax-highlighted preview**
(`Space`, a TOML file) → shell-script diff with word-level highlight and
**syntax-highlighted context** (`c` context toggle) → upload with the confirm
diff (`u` → `y`) → the **download overwrite gate** (`d` → diff → `d` → `y/n`) →
help (`?`).

## Requirements

`mise install` (from the repo root) provisions `cargo` and `tcut` from
[`mise.toml`](../mise.toml), where `tcut` tracks its latest release.
