import { defineVideo } from "tcut";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const rootDir = process.cwd();
const castDir = mkdtempSync(join(tmpdir(), "gistui-demo-"));
const demoHome = join(castDir, "home");

process.on("exit", () => {
  try {
    rmSync(castDir, { recursive: true, force: true });
  } catch {
    // Cleanup must not replace a successful render with an exit error.
  }
});

export default defineVideo(
  {
    output: "website/demo.gif",
    cast: join(castDir, "demo.cast"),
    theme: "tokyo-night",
    cols: 100,
    rows: 30,
    fps: 18,
    typingSpeed: 48,
    maxPause: "1.5s",
    shadow: true,
    title: "gistui — GitHub Gists TUI",
    requires: ["cargo", "python3"],
  },
  async (t) => {
    await t.hide(async () => {
      // Build binary if needed
      await t.run("cargo build --release");

      // Setup fake gh & seed data
      await t.run(`export GISTUI_DEMO_HOME=${JSON.stringify(demoHome)}`);
      await t.run(`export XDG_CONFIG_HOME="$GISTUI_DEMO_HOME/xdg"`);
      await t.run(`export XDG_CACHE_HOME="$GISTUI_DEMO_HOME/xdg/cache"`);
      await t.run(`mkdir -p "$GISTUI_DEMO_HOME/bin"`);
      await t.run(`cp "${rootDir}/scripts/demo/fake-gh" "$GISTUI_DEMO_HOME/bin/gh"`);
      await t.run(`chmod +x "$GISTUI_DEMO_HOME/bin/gh"`);
      await t.run(`export PATH="$GISTUI_DEMO_HOME/bin:${rootDir}/target/release:$PATH"`);
      await t.run("unset NO_COLOR");
      await t.run(`python3 "${rootDir}/scripts/demo/seed.py"`);
      await t.run(`cd "$GISTUI_DEMO_HOME/work"`);
      await t.clear();
    });

    // Launch gistui
    await t.type("gistui --no-update-check .\n");
    await t.wait(/aliases\.sh/, { scope: "screen" });
    await t.sleep("1.5s");

    // Revisions screen & screenshot
    await t.type("H");
    await t.wait(/Revisions/i, { scope: "screen" });
    await t.sleep("1.5s");
    await t.snapshot("website/revisions.png");
    await t.sleep("1.0s");
    await t.escape();
    await t.sleep("1.0s");

    // Gist Manager screen & screenshot
    await t.type("g");
    await t.wait(/Gists/i, { scope: "screen" });
    await t.sleep("1.5s");
    await t.snapshot("website/gist-manager.png");
    await t.sleep("1.0s");
    await t.escape();
    await t.sleep("1.0s");

    // Pin aliases.sh
    await t.type("p");
    await t.sleep("1.2s");

    // View Pins
    await t.type("P");
    await t.sleep("2.0s");
    await t.escape();
    await t.sleep("1.0s");

    // Preview
    await t.type(" ");
    await t.sleep("2.5s");
    await t.escape();
    await t.sleep("0.8s");

    // Navigate to starship.toml
    await t.down();
    await t.down();
    await t.sleep("0.8s");

    // Diff
    await t.enter();
    await t.sleep("2.5s");
    await t.type("c"); // toggle context
    await t.sleep("2.5s");
    await t.escape();
    await t.sleep("1.0s");

    // Upload
    await t.type("u");
    await t.sleep("2.0s");
    await t.type("y");
    await t.sleep("2.2s");

    // Navigate to hello.py & download with overwrite gate
    await t.up();
    await t.sleep("0.8s");
    await t.type("d");
    await t.sleep("2.0s");
    await t.type("d");
    await t.sleep("1.8s");
    await t.type("y");
    await t.sleep("1.8s");

    // Help
    await t.type("?");
    await t.sleep("2.2s");
    await t.escape();
    await t.sleep("0.8s");

    // Quit (q twice)
    await t.type("q");
    await t.sleep("0.3s");
    await t.type("q");
    await t.sleep("0.5s");
  },
);
