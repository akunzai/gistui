#!/usr/bin/env bash
#
# Regenerate the README demo recording end to end:
#   build gistui -> seed a fake gist store + workdir -> drive the real TUI in a
#   pseudo-tty with scripted keystrokes -> write docs/demo.cast -> render
#   docs/demo.gif with agg.
#
# The recording runs entirely against a fake `gh` (scripts/demo/fake-gh) backed
# by an isolated temp workspace, so it never touches a real GitHub account.
#
# Requirements: cargo, uv (runs the Python helpers), agg
# (https://github.com/asciinema/agg), and a monospace font with box-drawing +
# emoji glyphs.
#
# Tunables (env): SPEED, FONT, FONT_SIZE, COLS, ROWS, CAST, GIF.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

SPEED="${SPEED:-1.25}"
FONT="${FONT:-JetBrains Mono,Apple Color Emoji}"
FONT_SIZE="${FONT_SIZE:-16}"
GIF="${GIF:-$REPO_ROOT/docs/demo.gif}"

# Python helpers run through `uv run`, which provisions the interpreter on demand
# (stdlib only here — no third-party deps). The version comes from
# scripts/demo/.python-version, discovered via `--directory`.
PY=(uv run --no-project --directory "$SCRIPT_DIR" python)

for tool in cargo uv agg; do
  command -v "$tool" >/dev/null 2>&1 || { echo "error: '$tool' not found on PATH" >&2; exit 1; }
done

echo "==> building gistui (release)"
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"

# A short, fixed scratch workspace keeps the path shown in the TUI title bar
# clean (e.g. /private/tmp/gistui-demo/work). seed.py fully resets it each run.
WORKSPACE="${GISTUI_DEMO_WORKSPACE:-/tmp/gistui-demo}"
rm -rf "$WORKSPACE"
mkdir -p "$WORKSPACE"
trap 'rm -rf "$WORKSPACE"' EXIT

# The .cast is just an intermediate for agg; it lives in the scratch workspace
# and is discarded — only docs/demo.gif is kept in the repo.
CAST="${CAST:-$WORKSPACE/demo.cast}"

export GISTUI_DEMO_HOME="$WORKSPACE"
export GISTUI_DEMO_BIN="$REPO_ROOT/target/release/gistui"
export GISTUI_DEMO_FAKEBIN="$WORKSPACE/bin"
export GISTUI_DEMO_STEPS="$SCRIPT_DIR/storyboard.json"
export GISTUI_DEMO_CAST="$CAST"
export GISTUI_DEMO_COLS="${COLS:-100}"
export GISTUI_DEMO_ROWS="${ROWS:-30}"

# expose fake-gh as the program named `gh`
mkdir -p "$GISTUI_DEMO_FAKEBIN"
cp "$SCRIPT_DIR/fake-gh" "$GISTUI_DEMO_FAKEBIN/gh"
chmod +x "$GISTUI_DEMO_FAKEBIN/gh"

echo "==> seeding fake gist store + workdir"
"${PY[@]}" "$SCRIPT_DIR/seed.py"

echo "==> recording TUI session"
"${PY[@]}" "$SCRIPT_DIR/record.py"

echo "==> rendering GIF (speed ${SPEED}x)"
agg --font-family "$FONT" --font-size "$FONT_SIZE" --speed "$SPEED" "$CAST" "$GIF"

echo "==> done"
echo "    gif: $GIF"
