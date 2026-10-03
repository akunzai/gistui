#!/usr/bin/env python3
"""Seed a disposable Local/Gist hunk demo. All values are synthetic."""
import json
import pathlib
import shutil
import sys

root = pathlib.Path(__file__).resolve().parents[2]
home = pathlib.Path(sys.argv[1])
work = home / "work"
work.mkdir(parents=True, exist_ok=False)
(home / "state").mkdir()
(home / "bin").mkdir()
local = '''# Synthetic Codex settings for the hunk demo
model = "local-model"

[preferences]
editor = "local-editor"

[features]
example_feature = false
'''
gist = '''# Synthetic Codex settings for the hunk demo
model = "gist-model"

[preferences]
editor = "gist-editor"

[features]
example_feature = true
'''
(work / "config.toml").write_text(local)
(home / "state" / "gists.json").write_text(json.dumps({
    "gists": {"11111111111111111111111111111111": {
        "description": "Synthetic Codex configuration",
        "public": False,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-02T00:00:00Z",
        "files": {"config.toml": gist},
    }},
    "starred": [],
}))
shutil.copy2(root / "scripts" / "demo" / "fake-gh", home / "bin" / "gh")
(home / "bin" / "gh").chmod(0o755)
