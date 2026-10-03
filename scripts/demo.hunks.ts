// Reproducible, account-free evidence for bidirectional hunk sync.
import { defineVideo } from "tcut";
import { join } from "node:path";
import { mkdtempSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { createWorkspace, rootDir } from "./demo/session.ts";

const { castDir } = createWorkspace("gistui-hunks-recording-");
const home = join(realpathSync(castDir), "home");
// Keep review attachments outside the repo and after fixture cleanup.
const mediaDir = mkdtempSync(join(tmpdir(), "gistui-hunks-media-"));
const quote = (text: string) => "'" + text.replaceAll("'", "'\\''") + "'";

export default defineVideo(
  {
    output: join(mediaDir, "hunk-sync.gif"),
    cast: join(castDir, "hunks.cast"),
    theme: "tokyo-night",
    cols: 110,
    rows: 16,
    shadow: true,
    title: "gistui — staged hunk sync",
    requires: ["cargo", "python3"],
  },
  async (t) => {
    await t.hide(async () => {
      await t.run("cargo build");
      await t.run(`python3 ${quote(join(rootDir, "scripts/demo/hunk_fixture.py"))} ${quote(home)}`);
      await t.run(`export GISTUI_DEMO_HOME=${quote(home)}`);
      await t.run('export XDG_CONFIG_HOME="$GISTUI_DEMO_HOME/xdg"');
      await t.run('export XDG_CACHE_HOME="$GISTUI_DEMO_HOME/cache"');
      await t.run('export HOME="$GISTUI_DEMO_HOME"');
      await t.run('export PATH="$GISTUI_DEMO_HOME/bin:$PATH"');
      await t.run("unset NO_COLOR");
      await t.run('cd "$GISTUI_DEMO_HOME/work"');
      await t.type(`${quote(join(rootDir, "target/debug/gistui"))} --no-update-check --no-mouse .\n`);
      await t.wait(/Synthetic Codex configuration/, { scope: "screen" });
      await t.enter();
      await t.wait(/Hunk 1\/3/, { scope: "screen" });
    });
    await t.type("]");
    await t.wait(/Gist \[staged\]/, { scope: "screen" });
    await t.type("[");
    await t.wait(/Local \[staged\]/, { scope: "screen" });
    await t.expect(/Hunk 1\/1/);
    await t.type("z");
    await t.wait(/Hunk 1\/2/, { scope: "screen" });
    await t.type("[");
    await t.wait(/Hunk 1\/1/, { scope: "screen" });
    await t.pageUp();
    await t.sleep("300ms");
    await t.snapshot(join(mediaDir, "hunk-sync.png"));
    await t.sleep("300ms");
    await t.type("T");
    await t.wait(/Theme: light/, { scope: "screen" });
    await t.sleep("300ms");
    await t.snapshot(join(mediaDir, "hunk-sync-light.png"));
    await t.sleep("300ms");
    await t.type("T");
    await t.wait(/Theme: dark/, { scope: "screen" });
    await t.type("s");
    await t.wait(/Save staged changes/, { scope: "screen" });
    await t.expect(/Local \(saved\)/);
    await t.expect(/Gist \(saved\)/);
    await t.type("y");
    await t.wait(/Saved staged changes to Local and Gist/, { scope: "screen" });
    await t.type("]");
    await t.escape();
    await t.wait(/Unsaved staged changes/, { scope: "screen" });
    await t.type("n");
    await t.wait(/Gist \[staged\]/, { scope: "screen" });
    await t.type("z");
    await t.wait(/Hunk 1\/1/, { scope: "screen" });
    await t.hide(async () => {
      await t.escape();
      await t.wait(/Local \(1\)/, { scope: "screen" });
      await t.type("q");
      await t.wait(/Press q or Esc again/, { scope: "screen" });
      await t.type("q");
      await t.wait(/>\s*$/, { scope: "line" });
      const check = `import json,os,pathlib; h=pathlib.Path(os.environ['GISTUI_DEMO_HOME']); local=(h/'work/config.toml').read_text(); gist=next(iter(json.loads((h/'state/gists.json').read_text())['gists'].values()))['files']['config.toml']; assert 'model = "local-model"' in local and 'model = "local-model"' in gist; assert 'editor = "gist-editor"' in local and 'editor = "gist-editor"' in gist; assert 'example_feature = false' in local and 'example_feature = true' in gist; print('PASS: bidirectional save preserves the unselected difference')`;
      await t.run(`python3 -c ${quote(check)}`);
    });
  },
);
