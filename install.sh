#!/usr/bin/env bash
#
# gistui installer — downloads the prebuilt release binary for the current
# platform, verifies its SHA-256 checksum, and installs it onto your PATH.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/akunzai/gistui/main/install.sh | bash
#
# Options (as env vars or flags when run directly):
#   --version <tag>     install a specific release (default: latest)
#   --bin-dir <dir>     install directory (default: ~/.local/bin)
#
# Supported: Linux (x86_64, aarch64), macOS (x86_64, arm64), and Windows
# (x86_64) under Git Bash / MSYS2.

set -euo pipefail

REPO="akunzai/gistui"
VERSION="${GISTUI_VERSION:-latest}"
BIN_DIR="${GISTUI_BIN_DIR:-$HOME/.local/bin}"

err() {
  echo "error: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --bin-dir)
      BIN_DIR="${2:-}"
      shift 2
      ;;
    -h | --help)
      sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      err "unknown argument: $1"
      ;;
  esac
done

command -v curl >/dev/null 2>&1 || err "curl is required but was not found on PATH"

# --- Detect platform → Rust target triple, archive format, binary name --------
os="$(uname -s)"
arch="$(uname -m)"
bin_name="gistui"
case "$os" in
  Linux) plat="unknown-linux-gnu"; ext="tar.gz" ;;
  Darwin) plat="apple-darwin"; ext="tar.gz" ;;
  MINGW* | MSYS* | CYGWIN* | Windows_NT)
    plat="pc-windows-msvc"
    ext="zip"
    bin_name="gistui.exe"
    ;;
  *) err "unsupported operating system: $os" ;;
esac
case "$arch" in
  x86_64 | amd64) cpu="x86_64" ;;
  arm64 | aarch64) cpu="aarch64" ;;
  *) err "unsupported architecture: $arch" ;;
esac
target="${cpu}-${plat}"

if [ "$plat" = "pc-windows-msvc" ] && [ "$cpu" != "x86_64" ]; then
  err "no prebuilt Windows binary for $cpu (only x86_64 is published)"
fi

# --- Resolve the release tag --------------------------------------------------
if [ "$VERSION" = "latest" ]; then
  effective="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
    "https://github.com/$REPO/releases/latest")"
  case "$effective" in
    */tag/*) VERSION="${effective##*/tag/}" ;;
    *) err "could not determine the latest release from: $effective" ;;
  esac
fi

pkg="gistui-${VERSION}-${target}"
archive="${pkg}.${ext}"
base_url="https://github.com/$REPO/releases/download/${VERSION}"

# --- Download + verify --------------------------------------------------------
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading $archive ($VERSION)..."
curl -fsSL "$base_url/$archive" -o "$tmp/$archive" \
  || err "download failed; is $VERSION published for $target?"
curl -fsSL "$base_url/$archive.sha256" -o "$tmp/$archive.sha256" \
  || err "checksum download failed for $archive"

echo "verifying checksum..."
(
  cd "$tmp"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$archive.sha256"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "$archive.sha256"
  else
    err "neither sha256sum nor shasum found to verify the download"
  fi
) >/dev/null || err "checksum verification failed for $archive"

# --- Extract ------------------------------------------------------------------
echo "extracting..."
case "$ext" in
  tar.gz)
    tar -xzf "$tmp/$archive" -C "$tmp"
    ;;
  zip)
    if command -v unzip >/dev/null 2>&1; then
      unzip -q "$tmp/$archive" -d "$tmp"
    elif command -v powershell >/dev/null 2>&1; then
      powershell -NoProfile -Command \
        "Expand-Archive -Force '$(cygpath -w "$tmp/$archive")' '$(cygpath -w "$tmp")'"
    else
      err "need unzip or powershell to extract $archive"
    fi
    ;;
esac

# --- Install ------------------------------------------------------------------
mkdir -p "$BIN_DIR"
src="$tmp/$pkg/$bin_name"
[ -f "$src" ] || err "binary not found in archive at $pkg/$bin_name"
if command -v install >/dev/null 2>&1; then
  install -m 755 "$src" "$BIN_DIR/$bin_name"
else
  cp "$src" "$BIN_DIR/$bin_name"
  chmod +x "$BIN_DIR/$bin_name" 2>/dev/null || true
fi

echo "installed gistui $VERSION to $BIN_DIR/$bin_name"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "note: $BIN_DIR is not on your PATH — add it to use 'gistui' directly." ;;
esac
command -v gh >/dev/null 2>&1 \
  || echo "note: gistui needs the GitHub CLI ('gh') at runtime — see https://cli.github.com"
