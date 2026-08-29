# gistui

[![CI](https://github.com/akunzai/gistui/actions/workflows/ci.yml/badge.svg)](https://github.com/akunzai/gistui/actions/workflows/ci.yml)
[![crates.io](https://badgen.net/crates/v/gistui)](https://crates.io/crates/gistui)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Browse, compare, and manage your GitHub Gists in the terminal.

`gistui` puts your gists next to the files in your working directory and ranks
one list against the other, so you can resolve the difference in place — read
the diff, upload, download, or sync a pinned pair. No existing file is
overwritten without its diff and a `y`/`n` first.

`gh gist` is non-interactive, and a browser tab cannot see the file you are
editing. gistui runs on the GitHub CLI (`gh`), stores no token of its own, and
works against what is actually on disk where you launch it.

![gistui demo](https://raw.githubusercontent.com/akunzai/gistui/main/website/demo.gif)

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/akunzai/gistui/main/install.sh | bash
```

Homebrew, Scoop, crates.io, mise, the Windows PowerShell installer, building
from source, and self-upgrade (`gistui --upgrade`) are in
[docs/INSTALL.md](docs/INSTALL.md).

You also need [`gh`](https://cli.github.com) on your `PATH` and signed in with
`gh auth login` — gistui shells out to it at runtime, wherever you run it.

## Quick start

```bash
cd ~/dotfiles
gistui
```

`Tab` switches panes, `j`/`k` move, `Enter` opens the diff, `u` uploads and `d`
downloads, `p` pins the pair, and `q` quits. Press `?` on any screen for its
full keymap, `;` for a menu of what is valid right now, and `Ctrl+p` for the
command palette. The mouse works by default. `gistui --help` lists the flags.

## Docs

- [Keys and screens](https://akunzai.github.io/gistui/) — the keymap, and what each screen shows.
- [Install](docs/INSTALL.md) — every install path, and self-upgrade.
- [Safety](docs/SAFETY.md) — what confirms, and what is never overwritten silently.
- [Configuration](config.example.toml) — every config field, with its default.
- [Design](docs/design.md) — the principles the UI and these docs follow.
- [Contributing](CONTRIBUTING.md) — setup and the verification gate.
