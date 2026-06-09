# AGENTS.md

`gistui` is a Rust 2021 TUI for managing GitHub Gists — browse/diff/download/upload/create/pin gists and pair them with files in the working directory, all through the GitHub CLI (`gh`).

## Build / Test / Run

```bash
cargo run               # launch the TUI (needs a TTY)
cargo run -- --check    # print gh readiness, then exit (no TUI)
cargo test              # full suite; must NOT touch the network or require gh auth
```

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

- Tests must never call the real `gh` or the network. `gh` JSON parsing is tested against fixtures in `tests/fixtures/gh/`; IO functions are left as thin untested boundaries.
- Downloads only write to `cwd/<gist-filename>`. The overwrite gate is the invariant to preserve: an *existing* target is never overwritten without first showing its diff and a `y/n` confirmation (`Screen::Confirm`); writing a path that does not yet exist is allowed directly (no diff forced). Do not add a write path that overwrites an existing file without that diff+confirm.
- No GitHub tokens are stored by the app, and gist *content* is never written to the config file (`~/.config/gistui/config.toml`, XDG-aware). The config holds only `pinned` mappings and `skip_dirs`. See `config.example.toml` for the annotated schema.
- Use `frame.area()` (not `frame.size()`, which was removed in ratatui 0.28). The project now pins ratatui 0.30.
- `Rect::inner` takes `Margin` by value (not `&Margin`) since ratatui 0.28.

## Conventions

- Commit messages: Conventional Commits, in English (e.g. `feat:`, `docs:`, `fix:`).
- Fold same-scope follow-up fixes into the original commit (amend) rather than adding `fix typo` / `review fix` commits.
- Every PR MUST carry a release-note category label (`enhancement`, `bug`, `documentation`, `dependencies`, or `skip-changelog`) — GitHub groups auto-generated release notes by these via `.github/release.yml`.
- When a change adds or alters a user-facing key, screen, or feature, update `README.md` (the Actions/keymap and Safety sections) and the `?` help text in `tui.rs` **in the same PR** — keep docs and behavior in lockstep.

## Claude Code compatibility

`CLAUDE.md` is a symbolic link to this `AGENTS.md`, so Claude Code and any AGENTS.md-aware assistant read the same project memory. Edit `AGENTS.md`; never edit `CLAUDE.md` directly.
