# gistui

A terminal UI for managing GitHub Gists. Browse, diff, download, upload, create, and pin
your gists — and pair them with files in your working directory — all through the GitHub
CLI (`gh`).

## Requirements

- A Rust toolchain (to build) — <https://rustup.rs>
- The GitHub CLI: [`gh`](https://cli.github.com), installed and on your `PATH`
- An authenticated `gh` session: `gh auth login`

`gistui` shells out to `gh` at runtime (it stores no GitHub token of its own), so `gh` must
be installed and authenticated wherever you run `gistui`.

## Installation

Install the binary into `~/.cargo/bin` (make sure that directory is on your `PATH`):

```bash
cargo install --path .
```

Or build a release binary and place it yourself:

```bash
cargo build --release
# binary is at target/release/gistui — copy or symlink it onto your PATH, e.g.
ln -sf "$PWD/target/release/gistui" ~/.local/bin/gistui
```

## Usage

```bash
gistui            # launch the TUI (needs a TTY)
gistui --check    # print gh readiness, then exit (no TUI)
```

Run `gistui` from the directory whose files you want to pair with your gists. Inside the
TUI press `?` for the full keymap; the footer shows the keys relevant to the focused pane.

The left pane lists the files in your current working directory; the right pane lists your
gists, ranked against the selected local file (stronger matches are prefixed with stars:
⭐⭐⭐ exact-filename/pinned, ⭐⭐ path hint). Browse with `Tab` (switch pane), `Up`/`Down`
(move), and `Left`/`Right` (scroll a long row).

### Actions

- `Enter` (on a gist) — preview the unified diff between the selected local file and the
  gist, with `+`/`-` colour. From there `d` downloads or `u` uploads.
- `d` (on a gist) — download it into the cwd as `./<gist-filename>`. A brand-new file is
  written directly; an existing one is shown as a diff and overwritten only after a `y`/`n`
  confirmation.
- `u` (on a gist) — upload the selected local file into the gist under the local file's
  name (added directly, or diff + `y`/`n` if it would overwrite a same-named gist file).
- `n` (on a local file) — create a new gist from it; choose `s` secret or `p` public.
- `p` (on a gist) — toggle a pin between the selected local file and gist (persisted to
  config; pinned pairs sort to the top).
- `o` (on a gist) — open it on gist.github.com in your browser.
- `e` (on a local file) — open it in `$VISUAL`/`$EDITOR`.
- `Space` (on a gist) — preview the gist's raw content in a scrollable overlay.
- `/` filter by text · `v` cycle visibility (all/public/secret) · `s` cycle sort · `t`
  toggle row view.
- `Esc`/`q` — go back from an overlay; quit the app from the main list.

When no local file is selected (e.g. an empty directory), the right pane lists all gists
unranked so you can still preview and download into the current directory.

## Safety rules

- Downloads only ever write to `./<gist-filename>` in the current working directory.
- An existing file (local download target or remote gist file) is never overwritten without
  first showing its diff and a `y`/`n` confirmation. Writing something that does not yet
  exist is direct.
- Identical files are detected: when the two sides match, upload/download are disabled.
- No GitHub token is stored by the app, and gist content is never written to the config
  file — only path↔gist pin mappings are persisted.

## Building a release

```bash
cargo build --release
```

The optimized binary is `target/release/gistui`. It bundles no assets, but still requires
the `gh` CLI on `PATH` at runtime (`gh` is not vendored). Optionally shrink it with
`strip target/release/gistui`.

## Development

All four checks must pass before committing (clippy warnings are treated as errors):

```bash
cargo fmt --check
cargo test
cargo check
cargo clippy --all-targets -- -D warnings
```
