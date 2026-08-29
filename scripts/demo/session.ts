// Shared setup for the tcut recordings. Both the GIF (`demo.video.ts`) and the
// still screenshots (`demo.stills.ts`) drive the real `gistui` binary against
// the same fake `gh` and the same seeded workspace — only the terminal size and
// the keystrokes differ.
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export const rootDir = process.cwd();

/** An isolated, self-deleting workspace for one recording. */
export function createWorkspace(prefix: string) {
  const castDir = mkdtempSync(join(tmpdir(), prefix));

  process.on("exit", () => {
    try {
      rmSync(castDir, { recursive: true, force: true });
    } catch {
      // Cleanup must not replace a successful render with an exit error.
    }
  });

  return { castDir, demoHome: join(castDir, "home") };
}

/**
 * Builds the binary, puts the fake `gh` first on PATH, seeds the fake gists and
 * the local working directory, and leaves the shell sitting in that directory.
 * Call inside `t.hide` — none of this belongs on camera.
 */
export async function prepare(t: any, demoHome: string) {
  await t.run("cargo build --release");

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
}
