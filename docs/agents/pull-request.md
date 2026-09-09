# Pull requests

Write pull request titles, descriptions, and comments in **English**. Commit messages are
English too — Conventional Commits, imperative, subject under 72 characters.

[`.github/PULL_REQUEST_TEMPLATE.md`](../../.github/PULL_REQUEST_TEMPLATE.md) is authoritative
on structure; what follows adds what it does not say. Release-note labels, milestones, and the
`CHANGELOG.md` rule live in [`conventions.md`](conventions.md).

## Preparing

- Work from a feature branch, `feat/<topic>` or `fix/issue-<n>`.
- Title carries **no** Conventional Commit prefix. Commit subjects do; the title reads as a
  plain sentence naming the change, because release notes group by label, not by prefix.
- **Open a pull request, draft included, only when asked.**

## Description shape

1. A plain-language opening: what changed and why, for a reviewer who did not write it.
2. A visual GitHub renders inline, chosen by what changed:

   | Change | Visual |
   | --- | --- |
   | Screen, row/column layout, or mark vocabulary | Before/after stills |
   | Multi-step key or mouse interaction | Short recording |
   | State transition or background job lifecycle | Mermaid `flowchart` / `stateDiagram` |
   | `gh` call sequence | Mermaid `sequenceDiagram` |
   | Pure module, config, or dependency only | None — test output instead |

   Pair before with after. At most one diagram unless it is such a pair.

   `--attach '<file>#<alt text>'` works on `gh pr create`, `edit`, and `comment`, and on the
   `gh issue` equivalents, so a visual can land after the request is open. `gh` is unpinned
   here, so `--help` carries the current limits. Stills come from `mise run demo`
   (`website/*.png`) or a one-off `tcut` recording. Where capture is impossible,
   `<!-- screenshot pending: after -->` keeps the gap visible. What an attachment may hold is
   in [`verification.md`](verification.md).
3. A collapsed `<details>` trailer holding affected paths, implementation notes, verification
   commands, and log excerpts.

**No personally identifiable information in any attachment**, whatever ends up attached. Gist
titles, filenames, and usernames from a real account count; the capture rules in
[`verification.md`](verification.md) say what to do instead.

## Tests land with the behaviour

- **Product logic** — everything under `src/`. A change here lands with its tests in the same
  request. Which module owns which test is in [`conventions.md`](conventions.md).
- **Exempt** — `docs/`, `*.md`, `.github/`, `mise.toml`, `scripts/`, and dependency bumps
  with no behaviour change.
- **The two seams** — `src/tui/run_loop.rs` and `src/gh/mod.rs` are not unit-tested by design.
  A change there says so in the description and names what covers it instead.

No coverage threshold. The reviewer judges whether the new behaviour is actually exercised.

## Review readiness

Nothing unverified enters review: `mise run check` passes, and a change touching `gh`
behaviour passes `cargo run -- --check` too. [`verification.md`](verification.md) holds both,
the paths that must be exercised rather than only tested, and the gaps worth declaring.

Verification is local, so a request opens ready rather than as a draft awaiting a pipeline.
State in the description which paths were verified and which were not, with the reason.
