# Verification

How an agent exercises a change before it reaches review. Human setup narrative lives in
[`CONTRIBUTING.md`](../../CONTRIBUTING.md); this file holds only what an agent needs.

## Readiness check

```sh
cargo run -- --check
```

<!-- drift:forge github -->
<!-- drift:entrypoint-cmd cargo run -- --check -->

**Proof it came up**: exit 0 and `gh is installed and authenticated` on stdout. A missing or
unauthenticated `gh` exits non-zero naming the prerequisite. It never launches the TUI.

The TUI itself needs a TTY, so an agent verifies through `--check` and, where a pane is
available, through the herdr section below. `mise run run` belongs to a person at a terminal.

## Checks

| What | Command |
| --- | --- |
| Full gate — all must pass | `mise run check` |
| One test | `cargo test <name_filter>` |
| Demo media | `mise run demo` (see [`docs/demo.md`](../demo.md)) |

The gate's four steps live in `mise.toml`; `mise tasks` lists them.

<!-- drift:file mise.toml -->
<!-- drift:file tests/fixtures/gh/gist-list.json -->

## Human prerequisites

Run once, by a person. The commands above fail until they are done.

- [ ] `mise install` — provisions the Rust toolchain and `tcut`.
- [ ] Install `gh` and run `gh auth login`. It prompts, so it belongs to a person.

## Interactive keypaths, through a herdr pane

Where the session runs inside a `herdr` pane, drive the real binary in a sibling pane. Check
availability first; without it the keypaths stay unverified.

```sh
test "${HERDR_ENV:-}" = 1 && command -v herdr >/dev/null
```

When that holds:

```sh
cargo build
PANE=$(herdr pane split --current --direction right --cwd "$PWD" --no-focus \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"]["pane"]["pane_id"])')
herdr pane run "$PANE" "./target/debug/gistui --no-update-check"
herdr pane send-text "$PANE" '?'                        # a printable key
herdr pane send-keys "$PANE" escape                     # a named key
herdr pane read  "$PANE" --source visible --lines 60    # assert on the frame
herdr pane close "$PANE"
```

Assert on row 2 of each read — the pane title carries the screen identity. `--no-update-check`
keeps the release probe off the wire. Quitting takes `q` twice; the first arms the latch. A key
the screen does not own is ignored silently and reads like a hang: `g` opens the gist manager
from the list screen only, so check `src/tui/keymap.rs` before calling a swallowed key a defect.

A pane read holds the user's own gist titles, filenames, and descriptions. Treat it as
personally identifiable: assert on the frame or a marker glyph, and keep it out of issues,
pull requests, and transcripts.

## Exercised, not only tested

`mise run check` covers neither of these. Both are per-change calls, made from the paths the
change touches.

- `src/tui/mod.rs`, `src/tui/bg.rs`, `src/tui/run_loop.rs`, `src/tui/render/` — the terminal
  seam: raw mode, the alternate screen, mouse capture. Run `mise run demo`; it drives the real
  binary against `scripts/demo/fake-gh`, reproducible and account-free. Every run rewrites
  `website/demo.gif`, `website/gist-manager.png`, and `website/revisions.png` whether or not
  anything looked different, so stage only the media whose appearance changed and
  `git checkout --` the rest.
- `src/gh/`, `src/update_check.rs` — only a real account reaches these; `tests/gh_integration.rs`
  covers the plans and the parsing through a fake runner. Where `HERDR_ENV=1`, smoke them in a
  pane as above; otherwise record them in the request's unverified list.

## Capturing evidence

- Recording and stills: `mise run demo`, wrapping `tcut scripts/demo.video.ts` and
  `tcut scripts/demo.stills.ts`. See [`docs/demo.md`](../demo.md).
- A screen-visible change goes into the pull request as a still or a short recording; a
  pure-module change goes in as test output.
- Attachments carry no personally identifiable information. Gist titles, filenames, and
  usernames from a real account count — record against a throwaway account or crop.

<!-- drift:file scripts/demo.video.ts -->

## Not verified

- Interactive keypaths outside a herdr pane: no TTY. Covered by unit tests in
  `src/tui/screens/`, `src/tui/keys.rs`, and `src/tui/dispatch.rs`.
- `src/tui/run_loop.rs` and `src/gh/mod.rs`: the seams themselves, not unit-tested by design.
  See [`architecture.md`](architecture.md).
- `src/upgrade.rs`: exercising it calls the real GitHub Releases API and can replace the
  running binary. Verify by hand after a release cut.
- The release and publish workflows: only a tag push exercises them, so a change there is
  verified at the next cut. See [`RELEASING.md`](../../RELEASING.md).
