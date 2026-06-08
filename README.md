# gistui

`gistui` is a Rust TUI for pairing local config files with GitHub gist files.

## Requirements

- Rust toolchain
- GitHub CLI: `gh`
- Existing GitHub auth: `gh auth login`

## Available Actions

The left pane lists the files in your current working directory; the right pane lists your
gists, ranked against the selected local file. Stronger matches are prefixed with stars
(⭐⭐⭐ exact-filename/pinned, ⭐⭐ path hint); weak/no matches show none. Browse both panes
(`Tab` to switch focus, `Up`/`Down` to move, `Left`/`Right` to scroll a long row
horizontally). Currently wired:

- `d` (on a gist) — download it into the current working directory under the gist's own
  filename (`./<gist-filename>`). If no such file exists yet it is written straight away; if
  a same-named file already exists, its diff is shown first and you must confirm the
  overwrite with `y`/`n`.
- `Enter` (on a gist) — preview the unified diff in a full-screen overlay without writing
  anything (`Up`/`Down`/`Left`/`Right` to scroll, `d` to download from there, `Esc` to go
  back).
- `t` — toggle the gist rows between description view (`<filename> — <description>`) and id
  view (`<gist-id> / <filename>`), the latter disambiguating same-named files.
- `q` — quit.

When no local file is selected (e.g. an empty directory), the right pane lists all gists
unranked so you can still preview and download into the current directory.

## MVP Safety Rules

- Downloads only ever write to `./<gist-filename>` in the current working directory.
- An existing file at the download target is never overwritten without first showing its
  diff and a `y/n` confirmation. (Writing a brand-new file that does not exist yet is direct.)
- The diff shows the remote content that will be written (fetched fresh at preview time).
- GitHub tokens are not stored by this app.
- Gist content is not stored in config.

## Development

```bash
cargo check
cargo test
cargo run -- --check
```
