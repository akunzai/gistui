import { defineVideo } from "tcut";
import { join } from "node:path";
import { createWorkspace, prepare } from "./demo/session.ts";

const { castDir, demoHome } = createWorkspace("gistui-demo-");

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
      await prepare(t, demoHome);

      // Launch off-camera so the GIF opens on the TUI itself, not a shell prompt
      // typing the command. `t.hide` keeps the process running past the block.
      await t.type("gistui --no-update-check .\n");
      await t.wait(/aliases\.sh/, { scope: "screen" });
    });

    await t.sleep("1.5s");

    // Revisions screen (the still comes from `demo.stills.ts`, which records it
    // at its own height instead of inside this 30-row session)
    await t.type("H");
    await t.wait(/Revisions/i, { scope: "screen" });
    await t.sleep("2.5s");
    await t.escape();
    await t.sleep("1.0s");

    // Gist Manager screen
    await t.type("g");
    await t.wait(/Gists/i, { scope: "screen" });
    await t.sleep("2.5s");
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
