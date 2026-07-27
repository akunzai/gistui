# AGENTS.md

`gistui` is a Rust 2021 TUI for managing GitHub Gists — browse/diff/download/upload/create/pin gists and pair them with files in the working directory, all through the GitHub CLI (`gh`).

## Build / Test / Run

The dev toolchain (Rust + components, `agg`, and `uv` for the demo's Python
helpers) is pinned in [`mise.toml`](mise.toml). Run `mise install` once to
provision it; `mise tasks` lists the wrappers. `gh` is **not** pinned — it is a
user-provided runtime dependency. Plain `cargo` still works if you manage your
own toolchain.

```bash
mise install            # provision the pinned toolchain (one-time)
cargo run               # launch the TUI (needs a TTY)
cargo run -- --check    # print gh readiness, then exit (no TUI)
cargo test              # full suite; must NOT touch the network or require gh auth
mise run demo-gif       # regenerate the README demo GIF; re-run after any UI change
mise run demo-png       # regenerate the still PNG screenshots (website/*.png)
mise run demo           # regenerate all demo media (demo-gif + demo-png)
```

The demo recording harness (`scripts/demo/`) drives the **real** binary in a pseudo-tty against a **fake `gh`** over fake data, then renders `website/demo.gif` with `agg`. Only the GIF is versioned (the cast is a throwaway intermediate). Edit `storyboard.json` to change what the demo shows; see `scripts/demo/README.md`.

## Verification Gate (run before every commit)

All four MUST pass — the project treats clippy warnings as errors. Run them
together with `mise run check`, or individually:

```bash
cargo fmt --check
cargo test
cargo check
cargo clippy --all-targets -- -D warnings
```

If `cargo fmt --check` fails, run `cargo fmt` and confirm only formatting changed.

## Architecture

Pure, testable domain logic is kept separate from impure shell/filesystem adapters:

- Pure modules (unit-tested): `domain`, `config`, `ranking`, `local`, `diff`, the command-planning/guard parts of `actions`, and `tui::view_model` (`build_view_model`: `AppState` + pin-sync cache → presentation facts).
- Thin IO boundaries (not unit-tested by design): `gh` (`gh` subprocess calls), the `actions` execute helpers, and the IO helper fns in `tui::run_loop` / `tui::bg` workers.
- **Background jobs (issue #243):** `Jobs` is the single registry for spawn/absorb/cancel. Action jobs and local scans use generation supersession (issue #221); call sites use `jobs.spawn_action` / `request_*` rather than owning channel fields. `run_loop` only polls `jobs.absorb`.
- `tui` is a screen state machine (`Screen::{List, Diff(payload), Confirm(payload), Preview(payload), Help(payload), Config(payload), Revisions(payload), Pins(payload), Gists(payload), GistDetail(payload), Palette(payload), …}`; `Gists` is the gist-level manager). **Screen is `Clone`, not `Copy`** — payload variants hold screen-local state (issue #242 complete). **List stays a unit tag**: dual-pane selection/filters/sorts are session-global on `AppState` (user story 19 — not everything is forced into `Screen`). Diff owns body/scroll/pairing paths/pin identity/return; Confirm owns pending action + background text + return; Preview owns title/body/scroll/return; staged root `diff_return` / `preview_return` / `staged_diff_gist` are consumed on enter only. Revision/pin payloads park on Diff/Confirm return; detail parks on preview/compact restore; palette origin lives on the palette payload; opening detail from Gists parks the Gists payload on `detail.return_screen` so Esc restores list state. `AppState::handle_key` is **pure** — it mutates state and returns a `KeyOutcome` intent (IO-bearing variants carry payloads — issue #244); `run_loop`/`dispatch_outcome` perform the IO. Keep new key logic in `handle_key` (testable) and new IO in dispatch helpers.
- **View-model seam (issues #241 / #250):** each frame builds a pure `ViewModel` (`ChromeVm` + `ScreenVm`) at the draw entry for **every** screen (including Palette and Confirm with Diff/compact backgrounds). Paint helpers apply theme/layout only — no business rules or FS/network work.
- **Pin sync presentation:** `refresh_pin_sync_cache` (impure: may stat/read/hash local files) fills `AppState::pin_sync_cache`. Refresh on enter/return to Pins, when the pin list changes, after successful pin sync absorb, or when the dirty flag / length mismatch requires it — **not** every frame and not from the pure builder. Action dispatch may call `compute_pin_sync_status` for a one-shot decision; paint uses only `cached_pin_sync_status` / the VM. While staying on Pins, external editor edits may leave badges temporarily stale until the next refresh (no mtime watch).
- `run()` wraps `run_loop()` so terminal teardown (raw mode / alternate screen) ALWAYS runs, even on error — keep fallible startup/IO inside `run_loop`, never between `enable_raw_mode` and the teardown.

## Non-Obvious Rules

- Tests must never call the real `gh` or the network. `gh` JSON parsing is tested against fixtures in `tests/fixtures/gh/`; multi-step collect orchestrations take an injectable `CommandRunner` (`*_with` + `actions::test_support::SeqRunner`, issue #245); remaining IO functions are thin untested boundaries. End-to-end TUI exercising (driving the real binary, asserting on rendered frames) belongs to the `scripts/demo/` harness — which fakes `gh` and the working dir — not to the unit suite.
- Downloads only write to `cwd/<gist-filename>`. The overwrite gate is the invariant to preserve: an *existing* target is never overwritten without first showing its diff and a `y/n` confirmation (`Screen::Confirm`); writing a path that does not yet exist is allowed directly (no diff forced). Do not add a write path that overwrites an existing file without that diff+confirm.
- No GitHub tokens are stored by the app, and gist *content* is never written to the config file (`~/.config/gistui/config.toml`, XDG-aware). The config holds only `pinned` mappings and `skip_dirs`. See `config.example.toml` for the annotated schema.
- Use `frame.area()` (not `frame.size()`, which was removed in ratatui 0.28). The project now pins ratatui 0.30.
- `Rect::inner` takes `Margin` by value (not `&Margin`) since ratatui 0.28.

## Conventions

- Commit messages: Conventional Commits, in English (e.g. `feat:`, `docs:`, `fix:`).
- Fold same-scope follow-up fixes into the original commit (amend) rather than adding `fix typo` / `review fix` commits.
- Every PR MUST carry a release-note category label (`enhancement`, `bug`, `documentation`, `dependencies`, or `skip-changelog`) — GitHub groups auto-generated release notes by these via `.github/release.yml`.
- When a change adds or alters a user-facing key, screen, or feature, update `README.md` (the Actions/keymap and Safety sections) and the `?` help text in `tui.rs` **in the same PR** — keep docs and behavior in lockstep.
- Any user-facing feature or bug fix MUST add a concise bullet under the `## [Unreleased]` section of `CHANGELOG.md` **in the same PR**. Keep it one line, summarising the user-visible effect (GitHub Releases stays the authoritative, detailed source — see the file header). Changes labelled `skip-changelog` or purely internal (dependency bumps, refactors, test-only, typo fixes) do not need an entry.
- Versioning (SemVer): stay on `0.x` while the keymap/feature surface is still evolving; only cut `1.0.0` once it has gone several releases without a breaking UX change. A release is a `vX.Y.Z` tag matching `Cargo.toml`, which triggers `.github/workflows/release.yml` to build and attach the platform binaries the README `install.sh` expects.
- Release flow: bump `Cargo.toml` to the next version **when starting** the first new feature after a release — this keeps the in-development build distinct from the published one. Land changelog entries under `## [Unreleased]` during development (do **not** stamp a version or date on them yet). Only at the actual release does the `## [Unreleased]` heading get renamed to `## [X.Y.Z] — YYYY-MM-DD`; that is also when the `vX.Y.Z` tag is cut. So a version bump alone (no changelog version/date) is the normal mid-cycle state, not an oversight.

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues (`akunzai/gistui`), driven via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage roles, each label string equal to its name. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context — `CONTEXT.md` + `docs/adr/` at the repo root, created lazily. See `docs/agents/domain.md`.

## Claude Code compatibility

`CLAUDE.md` is a symbolic link to this `AGENTS.md`, so Claude Code and any AGENTS.md-aware assistant read the same project memory. Edit `AGENTS.md`; never edit `CLAUDE.md` directly.
