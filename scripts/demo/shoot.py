#!/usr/bin/env python3
"""Drive the real gistui binary to a single screen and write an asciinema v2 .cast
whose *final frame* is that screen — the still source for a PNG screenshot.

This mirrors record.py's pty driver, with one deliberate difference: the cast is
written *before* the gistui process is torn down, so the terminal teardown (raw
mode off / leave alternate screen) never lands in the capture and blanks the last
frame. The storyboard is the same ["wait", secs] / ["key", name-or-literal] format
as record.py; all paths come from the environment (see shots.sh for the contract).
"""
import codecs
import fcntl
import json
import os
import pty
import select
import signal
import struct
import termios
import time

HOME = os.environ["GISTUI_DEMO_HOME"]
BIN = os.environ["GISTUI_DEMO_BIN"]
FAKEBIN = os.environ["GISTUI_DEMO_FAKEBIN"]
STEPS_FILE = os.environ["GISTUI_DEMO_STEPS"]
OUT = os.environ["GISTUI_DEMO_CAST"]
WORK = os.path.join(HOME, "work")
COLS = int(os.environ.get("GISTUI_DEMO_COLS", "100"))
ROWS = int(os.environ.get("GISTUI_DEMO_ROWS", "30"))

# Keep these in sync with record.py — the same key names and recording theme so a
# shot frames identically to the demo GIF.
KEYS = {
    "enter": "\r",
    "esc": "\x1b",
    "tab": "\t",
    "up": "\x1b[A",
    "down": "\x1b[B",
    "right": "\x1b[C",
    "left": "\x1b[D",
}

# Tokyo Night, matching record.py.
THEME = {
    "fg": "#c0caf5",
    "bg": "#1a1b26",
    "palette": ":".join(
        [
            "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7",
            "#7dcfff", "#a9b1d6", "#414868", "#f7768e", "#9ece6a", "#e0af68",
            "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
        ]
    ),
}


def keybytes(name):
    return KEYS.get(name, name).encode()


def main():
    steps = json.loads(open(STEPS_FILE).read())

    env = dict(os.environ)
    env["PATH"] = FAKEBIN + ":" + env["PATH"]  # gistui's `gh` resolves to fake-gh
    env["XDG_CONFIG_HOME"] = os.path.join(HOME, "xdg")
    env["TERM"] = "xterm-256color"
    env["COLUMNS"] = str(COLS)
    env["LINES"] = str(ROWS)

    pid, master = pty.fork()
    if pid == 0:
        os.chdir(WORK)
        os.execvpe(BIN, [BIN, WORK], env)
        os._exit(127)

    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))

    events = []
    start = time.time()
    dec = codecs.getincrementaldecoder("utf-8")("replace")

    def drain(timeout):
        """Read+record pty output for `timeout` seconds. Returns False on EOF."""
        end = time.time() + timeout
        while True:
            remaining = end - time.time()
            if remaining <= 0:
                return True
            r, _, _ = select.select([master], [], [], remaining)
            if master in r:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    return False
                if not data:
                    return False
                text = dec.decode(data)
                if text:
                    events.append([round(time.time() - start, 4), "o", text])

    alive = drain(1.5)  # initial render
    for step in steps:
        if not alive:
            break
        kind, val = step
        if kind == "wait":
            alive = drain(float(val))
        elif kind == "key":
            os.write(master, keybytes(val))
            alive = drain(0.35)  # let the frame settle / redraw
    drain(0.6)  # settle the final frame

    # Write the cast NOW, before tearing the process down: gistui's teardown leaves
    # the alternate screen (a clear), which must not become the captured last frame.
    header = {
        "version": 2,
        "width": COLS,
        "height": ROWS,
        "timestamp": int(start),
        "env": {"TERM": "xterm-256color", "SHELL": "/bin/zsh"},
        "theme": THEME,
    }
    with open(OUT, "w") as f:
        f.write(json.dumps(header) + "\n")
        for e in events:
            f.write(json.dumps(e) + "\n")

    try:
        os.kill(pid, signal.SIGTERM)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except OSError:
        pass
    print(f"wrote {OUT}: {len(events)} events, {events[-1][0] if events else 0}s")


if __name__ == "__main__":
    main()
