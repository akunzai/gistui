# Installation

Each [release](https://github.com/akunzai/gistui/releases/latest) attaches prebuilt,
checksummed binaries — no Rust toolchain required.

## Download a prebuilt binary (recommended)

The install script detects your platform, downloads the matching release asset, verifies
its SHA-256 checksum, and installs it into `~/.local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/akunzai/gistui/main/install.sh | bash
```

It supports Linux (x86-64/ARM64), macOS (Intel/Apple Silicon), and Windows (x86-64) under
[Git Bash](https://gitforwindows.org)/MSYS2 — on Windows, prefer the native
[PowerShell installer](#windows-powershell) below. Pass `--version <tag>` to pin a release or
`--bin-dir <dir>` to change the install location.

### Manual download

Grab the archive for your platform from the releases page:

| Platform | Asset |
|----------|-------|
| macOS (Apple Silicon) | `gistui-<version>-aarch64-apple-darwin.tar.gz` |
| macOS (Intel) | `gistui-<version>-x86_64-apple-darwin.tar.gz` |
| Linux (x86-64) | `gistui-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux (ARM64) | `gistui-<version>-aarch64-unknown-linux-gnu.tar.gz` |
| Windows (x86-64) | `gistui-<version>-x86_64-pc-windows-msvc.zip` |

Then extract it and put `gistui` somewhere on your `PATH`, e.g. on macOS/Linux:

```bash
tar -xzf gistui-<version>-<target>.tar.gz
install -m 755 gistui-<version>-<target>/gistui ~/.local/bin/gistui
```

## Homebrew (macOS / Linux)

```bash
brew install akunzai/tap/gistui
```

This installs `gistui` (and its `gh` dependency) from the
[`akunzai/homebrew-tap`](https://github.com/akunzai/homebrew-tap) tap. The fully-qualified
name trusts only this formula — Homebrew 6.0.0+ requires non-official taps to be trusted
before their code runs; see the tap README for the `brew tap` + short-name flow and the
`Brewfile` `trusted:` option.

## Windows (PowerShell)

Install natively from PowerShell — this downloads the `x86_64-pc-windows-msvc` build,
verifies its SHA-256 checksum, installs into `~\.local\bin`, and adds that directory to your
user `PATH`:

```powershell
irm https://raw.githubusercontent.com/akunzai/gistui/main/install.ps1 | iex
```

To pin a release or change the install directory, invoke it as a script block:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/akunzai/gistui/main/install.ps1))) -Version v0.9.0 -BinDir 'C:\tools\bin'
```

> Don't pipe `install.sh` into `bash` from PowerShell: if WSL is installed, `bash` resolves to
> the WSL launcher and installs the **Linux** binary inside WSL. Use `install.ps1` for a native
> Windows install, or run `install.sh` from [Git Bash](https://gitforwindows.org).

## Scoop (Windows)

With [Scoop](https://scoop.sh), install from the
[`akunzai/scoop-bucket`](https://github.com/akunzai/scoop-bucket) bucket — it handles `PATH`,
updates, and the `gh` dependency for you:

```powershell
scoop bucket add akunzai https://github.com/akunzai/scoop-bucket
scoop install gistui
```

## crates.io

With a Rust toolchain, install the published crate from
[crates.io](https://crates.io/crates/gistui):

```bash
cargo install gistui
```

Or grab the same checksummed release binaries without compiling, via
[`cargo binstall`](https://github.com/cargo-bins/cargo-binstall):

```bash
cargo binstall gistui
```

## Build from source

With a Rust toolchain, install into `~/.cargo/bin` (make sure that directory is on your
`PATH`):

```bash
cargo install --path .
```

Or build a release binary and place it yourself:

```bash
cargo build --release
# binary is at target/release/gistui — copy or symlink it onto your PATH, e.g.
ln -sf "$PWD/target/release/gistui" ~/.local/bin/gistui
```

## Upgrading pre-built binaries

If you installed via `install.sh`, `install.ps1`, or a manual GitHub Release download,
`gistui` can upgrade itself without re-running the installer:

```bash
gistui --upgrade                         # upgrade to the latest release
gistui --upgrade --check                 # print current vs latest; exit 0 if up to date
gistui --upgrade --upgrade-version v0.12.0  # pin to a specific release (0.12.0 also works)
```

The upgrader downloads the same checksummed release assets as the install scripts,
verifies SHA-256, and replaces the **currently running** binary. On Windows the
running `.exe` cannot be overwritten immediately — gistui stages the new binary and
finishes the swap after you exit the process.

Package-manager and toolchain installs are **not** self-upgraded; gistui detects them
and prints the right command instead (`brew upgrade gistui`, `scoop update gistui`,
`cargo install gistui --force`, or `cargo binstall gistui --force`).