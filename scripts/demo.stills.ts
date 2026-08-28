// Still screenshots for the project page, recorded in a short terminal.
//
// The GIF needs 30 rows to show the two-pane browse view; the gist manager and
// the revision list only ever draw a handful of rows, so recording them at the
// same height left two thirds of each PNG as empty terminal and forced the page
// to crop. Recording them at their natural height instead means the page can
// show them whole, large enough to read.
import { defineVideo } from "tcut";
import { join } from "node:path";
import { createWorkspace, prepare } from "./demo/session.ts";

const { castDir, demoHome } = createWorkspace("gistui-stills-");

export default defineVideo(
  {
    // The stills come from `t.snapshot`; this output only closes the render.
    output: join(castDir, "stills.txt"),
    cast: join(castDir, "stills.cast"),
    theme: "tokyo-night",
    cols: 100,
    rows: 10,
    shadow: true,
    title: "gistui — GitHub Gists TUI",
    requires: ["cargo", "python3"],
  },
  async (t) => {
    await t.hide(async () => {
      await prepare(t, demoHome);
      await t.type("gistui --no-update-check .\n");
      await t.wait(/aliases\.sh/, { scope: "screen" });
    });

    // Revision history of the pair under the cursor, so the footer still names
    // the local file the revisions are being compared against.
    await t.type("H");
    await t.wait(/Revisions:/, { scope: "screen" });
    await t.sleep("1.5s");
    await t.snapshot("website/revisions.png");
    await t.sleep("1.5s");
    await t.escape();
    await t.wait(/Local \(\d+\)/, { scope: "screen" });

    // Gist manager. The browse view already prints "Gists (5)" over its right
    // pane, so wait on a header only the manager draws.
    await t.type("g");
    await t.wait(/sort:updated/, { scope: "screen" });
    await t.sleep("1.5s");
    await t.snapshot("website/gist-manager.png");
    await t.sleep("1.5s");

    await t.hide(async () => {
      await t.escape();
      await t.type("q");
      await t.type("q");
    });
  },
);
