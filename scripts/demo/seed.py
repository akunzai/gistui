#!/usr/bin/env python3
"""Seed a fresh fake gist store + working directory for the demo recording.

Everything lives under $GISTUI_DEMO_HOME so the recording never touches a real
GitHub account or the user's real gists. Re-running resets the workspace
(persisted pins + any files a previous run downloaded) so recordings are
deterministic.
"""
import json
import os
import pathlib
import shutil

HOME = pathlib.Path(os.environ["GISTUI_DEMO_HOME"])
STATE = HOME / "state" / "gists.json"
WORK = HOME / "work"
XDG = HOME / "xdg"

# Fake gists keyed by id. The fake `gh` serves these and mutates them in place
# for edit/create/delete so the TUI reflects changes across subprocess calls.
GISTS = {
    "g_aaa111": {
        "description": "Starship prompt config",
        "public": True,
        "updated_at": "2026-06-09T18:20:00Z",
        "created_at": "2026-03-01T00:00:00Z",
        "comments": 2,
        "files": {
            "starship.toml": (
                "# Starship prompt\n"
                "add_newline = false\n"
                "command_timeout = 1000\n\n"
                "[character]\n"
                'success_symbol = "[→](bold green)"\n'
                'error_symbol = "[✗](bold red)"\n\n'
                "[git_branch]\n"
                'symbol = " "\n'
            )
        },
    },
    "g_bbb222": {
        "description": "Handy git aliases",
        "public": True,
        "updated_at": "2026-06-08T09:00:00Z",
        "created_at": "2026-02-10T00:00:00Z",
        "files": {
            "aliases.sh": (
                "#!/usr/bin/env bash\n"
                "# Handy git aliases\n"
                "alias gs='git status'\n"
                "alias gp='git push'\n"
                "alias gl='git log --oneline -20'\n"
                "alias gb='git branch'\n"
                "alias gco='git checkout'\n"
                "alias gf='git fetch --all --prune'\n"
                "alias gst='git stash'\n"
                "alias gpl='git pull --rebase'\n"
                "alias gm='git merge'\n"
                "alias gr='git rebase'\n"
            )
        },
    },
    "g_ccc333": {
        "description": "Tmux base config",
        "public": False,
        "updated_at": "2026-05-30T12:00:00Z",
        "created_at": "2026-01-15T00:00:00Z",
        "files": {
            "tmux.conf": (
                "set -g mouse on\n"
                "set -g base-index 1\n"
                "bind r source-file ~/.tmux.conf\n"
            )
        },
    },
    "g_ddd444": {
        "description": "Hello world (Python)",
        "public": True,
        "updated_at": "2026-06-07T15:30:00Z",
        "created_at": "2026-04-01T00:00:00Z",
        "files": {"hello.py": 'print("Hello, world!")\n'},
    },
    "g_eee555": {
        "description": "Reading list",
        "public": False,
        "updated_at": "2026-04-20T08:00:00Z",
        "created_at": "2026-04-20T08:00:00Z",
        "files": {
            "notes.md": (
                "# Reading list\n\n"
                "- The Rust Programming Language\n"
                "- Crafting Interpreters\n"
            )
        },
    },
}

# Local working-dir files. Some pair with gists by filename; the deliberate
# content differences drive the diff / upload-confirm / download-overwrite gate.
LOCAL = {
    # differs from the gist -> a meaningful upload diff
    "starship.toml": (
        "# Starship prompt\n"
        "add_newline = false\n"
        "command_timeout = 1500\n\n"
        "[character]\n"
        'success_symbol = "[→](bold green)"\n'
        'error_symbol = "[✗](bold red)"\n\n'
        "[git_branch]\n"
        'symbol = " "\n'
        'style = "bold purple"\n\n'
        "[directory]\n"
        "truncation_length = 3\n"
    ),
    # differs from the gist -> triggers the download overwrite gate
    "hello.py": ("#!/usr/bin/env python3\n" 'print("Hello from the gistui demo")\n'),
    # differs from the gist -> a meaningful upload diff (shell highlighting);
    # changes top + bottom with a long unchanged middle so the `c` context
    # toggle visibly collapses/expands hidden lines.
    "aliases.sh": (
        "#!/usr/bin/env bash\n"
        "# Handy git aliases\n"
        "alias gs='git status --short'\n"
        "alias gp='git push'\n"
        "alias gl='git log --oneline -20'\n"
        "alias gb='git branch'\n"
        "alias gco='git checkout'\n"
        "alias gf='git fetch --all --prune'\n"
        "alias gst='git stash'\n"
        "alias gpl='git pull --rebase'\n"
        "alias gm='git merge'\n"
        "alias gr='git rebase'\n"
        "alias gd='git diff'\n"
        "alias gca='git commit --amend'\n"
    ),
}


def main():
    if XDG.exists():
        shutil.rmtree(XDG)  # drop persisted pins from a previous run
    XDG.mkdir(parents=True, exist_ok=True)
    STATE.parent.mkdir(parents=True, exist_ok=True)
    WORK.mkdir(parents=True, exist_ok=True)

    STATE.write_text(json.dumps({"gists": GISTS}, indent=2))
    for name, content in LOCAL.items():
        (WORK / name).write_text(content)
    # remove files a previous run downloaded (anything not in LOCAL)
    for p in WORK.iterdir():
        if p.is_file() and p.name not in LOCAL:
            p.unlink()
    print(f"seeded {STATE} and {WORK}")


if __name__ == "__main__":
    main()
