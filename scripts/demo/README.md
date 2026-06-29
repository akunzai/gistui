# Demo recording harness

Regenerates the README demo (`docs/demo.gif`) by driving the **real** `gistui`
binary against **fake** data, fully scripted and reproducible — no real GitHub
account, no manual keypresses.

```bash
scripts/demo/record.sh
```

That builds `gistui`, records a session, and writes `docs/demo.gif`. The
intermediate asciinema `.cast` is recorded into a throwaway workspace and
discarded — the GIF is fully reproducible from this harness, so only it is
versioned.

## Why this exists

A TUI is hard to screenshot consistently by hand, and recording against real
gists leaks private data and drifts every run. This harness pins the data and
the keystrokes, so:

- **Re-record after a UI change** — tweak nothing (or just `storyboard.json`)
  and re-run to get a fresh, identical-framing GIF.
- **Deterministic fixtures** — the same fake gists + local files every time.
- **Foundation for end-to-end tests** — the same fake-`gh` + pty-driver pattern
  can assert on rendered frames without touching the network (kept separate
  from the unit suite, which by design never spawns `gh`).

## How it works

`gistui` shells out to the GitHub CLI for everything, so we intercept `gh`:

| Piece | Role |
|-------|------|
| `fake-gh` | A stateful stand-in for `gh`. Implements only the commands gistui uses (`api /gists`, `gist view/edit/create/delete`, `api PATCH`) over a JSON store, and mutates the store so uploads/downloads/deletes are reflected live. Symlinked/copied to `gh` and put first on `PATH`. |
| `seed.py` | Writes the fake gist store + the local working-dir files into an isolated workspace. Content is crafted so a diff, an upload, and a download-overwrite are all meaningful. |
| `record.py` | Forks `gistui` under a pseudo-tty, sets the window size, replays `storyboard.json` keystrokes with pauses, and captures output into an asciinema v2 `.cast`. |
| `storyboard.json` | The script: `["wait", secs]` and `["key", name]` steps. Edit this to change the demo. |
| `record.sh` | Orchestrates the above in a throwaway `mktemp` workspace and renders the GIF with [`agg`](https://github.com/asciinema/agg). |

The recording is isolated: a temp `$GISTUI_DEMO_HOME` holds the store, the
working dir, and a fresh `XDG_CONFIG_HOME` (so persisted pins never leak between
runs), and it is deleted on exit.

## Storyboard

Browse with ranking → pin a pair + the Pins view → **syntax-highlighted preview**
(`Space`, a TOML file) → shell-script diff with word-level highlight and
**syntax-highlighted context** (`c` context toggle) → upload with the confirm
diff → the **download overwrite gate** (`d` → diff → `d` → `y/n`) → help (`?`).
The recording uses an isolated `XDG_CONFIG_HOME` + `XDG_CACHE_HOME` so a stale
host cache cannot leak starred-gist badges into the demo, and unsets `NO_COLOR`
so diff/preview syntax highlighting is captured even when the host shell disables it.

## Requirements

`mise install` (from the repo root) provisions `cargo`, `uv`, and `agg` from the
pinned [`mise.toml`](../../mise.toml); `mise run demo` then wraps `record.sh`.
The Python helpers run through `uv run`, which provisions the interpreter (and
Pillow, for screenshots) on demand — no manual virtualenv. The interpreter
version is pinned in [`.python-version`](.python-version) and picked up
automatically. To set them up manually instead:

- `cargo`, [`uv`](https://docs.astral.sh/uv/)
- [`agg`](https://github.com/asciinema/agg) — `brew install agg`
- A monospace font with box-drawing + emoji glyphs (default `JetBrains Mono` +
  `Apple Color Emoji`; override with `FONT=`).

## Tunables

Environment variables understood by `record.sh`:

| Var | Default | Meaning |
|-----|---------|---------|
| `SPEED` | `1.25` | Playback speed passed to `agg`. |
| `FONT` | `JetBrains Mono,Apple Color Emoji` | `agg --font-family`. |
| `FONT_SIZE` | `16` | `agg --font-size`. |
| `COLS` / `ROWS` | `100` / `30` | Recording terminal size. |
| `GIF` | `docs/demo.gif` | Output GIF path. |
| `CAST` | `<workspace>/demo.cast` | Intermediate cast (discarded by default). |

## Still screenshots

`record.sh` makes the animated demo; `shots.sh` makes **still PNGs** of individual
screens (for the README / the GitHub Pages landing page):

```bash
scripts/demo/shots.sh
```

It reuses the same fake `gh` + `seed.py` data, then for each shot: drives the real
TUI to one screen (`shoot.py`), renders that frame to a GIF with `agg`, and
extracts the frame to a PNG with Pillow. Each entry in the `SHOTS` array maps
`shots/<name>.json` (a storyboard, same format as `storyboard.json`) to
`docs/<name>.png`. The current shots:

| Shot | Screen | Output |
|------|--------|--------|
| `gist-manager` | the gist manager (`g`) | `docs/gist-manager.png` |

`shoot.py` differs from `record.py` in one way: it writes the cast *before* the
TUI tears down, so leaving the alternate screen never blanks the captured frame.
Pillow is pulled in on demand via `uv run --with pillow`, so no manual setup or
virtualenv is needed.

Tunables (env): `FONT`, `FONT_SIZE`, `COLS` / `ROWS`, `OUT_DIR` (default `docs/`).
