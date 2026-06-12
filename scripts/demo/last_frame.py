#!/usr/bin/env python3
"""Save the final frame of a GIF as a flattened RGB PNG.

Used by shots.sh to turn an agg-rendered single-screen GIF into a still
screenshot. Runs under scripts/demo/.venv (Pillow), never the system python.
"""
import sys

from PIL import Image


def main():
    src, dst = sys.argv[1], sys.argv[2]
    im = Image.open(src)
    im.seek(im.n_frames - 1)  # composite up to the last frame (held screen)
    im.convert("RGB").save(dst)
    print(f"wrote {dst} ({im.width}x{im.height})")


if __name__ == "__main__":
    main()
