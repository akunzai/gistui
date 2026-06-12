#!/usr/bin/env bash
#
# Regenerate still PNG screenshots of individual gistui screens (for the README /
# the GitHub Pages landing page). Reuses the demo harness — the same fake `gh`
# (scripts/demo/fake-gh) and seed data (seed.py) as record.sh — so screenshots
# never touch a real GitHub account.
#
# Pipeline per shot: drive the real TUI to one screen (shoot.py) -> render the
# captured frame to a GIF with agg -> extract that frame to a PNG with Pillow.
#
# Requirements: cargo, python3, agg, and a monospace font with box-drawing +
# emoji glyphs. Pillow is installed automatically into scripts/demo/.venv.
#
# Tunables (env): FONT, FONT_SIZE, COLS, ROWS, OUT_DIR.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

FONT="${FONT:-JetBrains Mono,Apple Color Emoji}"
FONT_SIZE="${FONT_SIZE:-16}"
OUT_DIR="${OUT_DIR:-$REPO_ROOT/docs}"

# Each shot drives scripts/demo/shots/<name>.json and writes $OUT_DIR/<name>.png.
SHOTS=("gist-manager")

for tool in cargo python3 agg; do
  command -v "$tool" >/dev/null 2>&1 || { echo "error: '$tool' not found on PATH" >&2; exit 1; }
done

# Pillow lives in an isolated venv so the system python stays clean (PEP 668).
VENV="$SCRIPT_DIR/.venv"
if [ ! -x "$VENV/bin/python" ]; then
  echo "==> creating venv + installing Pillow ($VENV)"
  python3 -m venv "$VENV"
  "$VENV/bin/python" -m pip install --quiet --upgrade pip pillow
fi

echo "==> building gistui (release)"
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"

WORKSPACE="${GISTUI_DEMO_WORKSPACE:-/tmp/gistui-shots}"
rm -rf "$WORKSPACE"
mkdir -p "$WORKSPACE"
trap 'rm -rf "$WORKSPACE"' EXIT

export GISTUI_DEMO_HOME="$WORKSPACE"
export GISTUI_DEMO_BIN="$REPO_ROOT/target/release/gistui"
export GISTUI_DEMO_FAKEBIN="$WORKSPACE/bin"
export GISTUI_DEMO_CAST="$WORKSPACE/shot.cast"
export GISTUI_DEMO_COLS="${COLS:-100}"
export GISTUI_DEMO_ROWS="${ROWS:-30}"

# expose fake-gh as the program named `gh`
mkdir -p "$GISTUI_DEMO_FAKEBIN"
cp "$SCRIPT_DIR/fake-gh" "$GISTUI_DEMO_FAKEBIN/gh"
chmod +x "$GISTUI_DEMO_FAKEBIN/gh"

mkdir -p "$OUT_DIR"
for name in "${SHOTS[@]}"; do
  steps="$SCRIPT_DIR/shots/$name.json"
  [ -f "$steps" ] || { echo "error: missing storyboard $steps" >&2; exit 1; }

  echo "==> [$name] seeding fake gist store + workdir"
  python3 "$SCRIPT_DIR/seed.py" >/dev/null

  echo "==> [$name] driving TUI to the target screen"
  GISTUI_DEMO_STEPS="$steps" python3 "$SCRIPT_DIR/shoot.py"

  gif="$WORKSPACE/$name.gif"
  echo "==> [$name] rendering frame"
  agg --font-family "$FONT" --font-size "$FONT_SIZE" "$GISTUI_DEMO_CAST" "$gif" >/dev/null 2>&1

  "$VENV/bin/python" "$SCRIPT_DIR/last_frame.py" "$gif" "$OUT_DIR/$name.png"
done

echo "==> done"
