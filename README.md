# gistui

`gistui` is a Rust TUI for pairing local config files with GitHub gist files.

## Requirements

- Rust toolchain
- GitHub CLI: `gh`
- Existing GitHub auth: `gh auth login`

## MVP Safety Rules

- Upload and download flows preview changes before writing.
- Existing local files are not overwritten without confirmation.
- Remote gist content is fetched again before write-like actions.
- GitHub tokens are not stored by this app.
- Gist content is not stored in config.

## Development

```bash
cargo check
cargo test
cargo run -- --check
```
