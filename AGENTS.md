# AGENTS.md

`gistui` is a Rust 2021 TUI for managing GitHub Gists — browse/diff/download/upload/create/pin gists and pair them with files in the working directory, all through the GitHub CLI (`gh`).

## Build / Test / Run

```bash
cargo run               # launch the TUI (needs a TTY)
cargo run -- --check    # print gh readiness, then exit (no TUI)
cargo test              # full suite; must NOT touch the network or require gh auth
scripts/demo/record.sh  # regenerate the README demo GIF; re-run after any UI change
```

The demo recording harness (`scripts/demo/`) drives the **real** binary in a pseudo-tty against a **fake `gh`** over fake data, then renders `docs/demo.gif` with `agg`. Only the GIF is versioned (the cast is a throwaway intermediate). Edit `storyboard.json` to change what the demo shows; see `scripts/demo/README.md`.

## Verification Gate (run before every commit)

All four MUST pass — the project treats clippy warnings as errors:

```bash
cargo fmt --check
cargo test
cargo check
cargo clippy --all-targets -- -D warnings
```

If `cargo fmt --check` fails, run `cargo fmt` and confirm only formatting changed.

## Architecture

Pure, testable domain logic is kept separate from impure shell/filesystem adapters:

- Pure modules (unit-tested): `domain`, `config`, `ranking`, `local`, `diff`, and the command-planning/guard parts of `actions`.
- Thin IO boundaries (not unit-tested by design): `gh` (`gh` subprocess calls), the `actions` execute helpers, and the IO helper fns in `tui::run_loop`.
- `tui.rs` is a screen state machine (`Screen::{List, Diff, Confirm, Preview, Help, Pins, Gists}`; `Gists` is the gist-level manager). `AppState::handle_key` is **pure** — it mutates state and returns a `KeyOutcome` intent; `run_loop` performs the IO (fetch/download/upload/create/delete/remove-file/edit-description). Keep new key logic in `handle_key` (testable) and new IO in `run_loop` helpers.
- `run()` wraps `run_loop()` so terminal teardown (raw mode / alternate screen) ALWAYS runs, even on error — keep fallible startup/IO inside `run_loop`, never between `enable_raw_mode` and the teardown.

## Non-Obvious Rules

- Tests must never call the real `gh` or the network. `gh` JSON parsing is tested against fixtures in `tests/fixtures/gh/`; IO functions are left as thin untested boundaries. End-to-end TUI exercising (driving the real binary, asserting on rendered frames) belongs to the `scripts/demo/` harness — which fakes `gh` and the working dir — not to the unit suite.
- Downloads only write to `cwd/<gist-filename>`. The overwrite gate is the invariant to preserve: an *existing* target is never overwritten without first showing its diff and a `y/n` confirmation (`Screen::Confirm`); writing a path that does not yet exist is allowed directly (no diff forced). Do not add a write path that overwrites an existing file without that diff+confirm.
- No GitHub tokens are stored by the app, and gist *content* is never written to the config file (`~/.config/gistui/config.toml`, XDG-aware). The config holds only `pinned` mappings and `skip_dirs`. See `config.example.toml` for the annotated schema.
- Use `frame.area()` (not `frame.size()`, which was removed in ratatui 0.28). The project now pins ratatui 0.30.
- `Rect::inner` takes `Margin` by value (not `&Margin`) since ratatui 0.28.

## Conventions

- Commit messages: Conventional Commits, in English (e.g. `feat:`, `docs:`, `fix:`).
- Fold same-scope follow-up fixes into the original commit (amend) rather than adding `fix typo` / `review fix` commits.
- Every PR MUST carry a release-note category label (`enhancement`, `bug`, `documentation`, `dependencies`, or `skip-changelog`) — GitHub groups auto-generated release notes by these via `.github/release.yml`.
- When a change adds or alters a user-facing key, screen, or feature, update `README.md` (the Actions/keymap and Safety sections) and the `?` help text in `tui.rs` **in the same PR** — keep docs and behavior in lockstep.
- Versioning (SemVer): stay on `0.x` while the keymap/feature surface is still evolving; only cut `1.0.0` once it has gone several releases without a breaking UX change. A release is a `vX.Y.Z` tag matching `Cargo.toml`, which triggers `.github/workflows/release.yml` to build and attach the platform binaries the README `install.sh` expects.

## Claude Code compatibility

`CLAUDE.md` is a symbolic link to this `AGENTS.md`, so Claude Code and any AGENTS.md-aware assistant read the same project memory. Edit `AGENTS.md`; never edit `CLAUDE.md` directly.
