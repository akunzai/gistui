# Issue tracker: GitHub

Issues and PRDs for this repo live as GitHub issues. Use the `gh` CLI for all operations.

Write issue titles and bodies in **English**, matching the existing tracker and release notes.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body "..."`. Use a heredoc for multi-line bodies.
- **Read an issue**: `gh issue view <number> --comments`, filtering comments by `jq` and also fetching labels.
- **List issues**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Comment on an issue**: `gh issue comment <number> --body "..."`
- **Apply / remove labels**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

Infer the repo from `git remote -v` — `gh` does this automatically when run inside a clone.

## Description shape

1. Open with what a maintainer or a new contributor would observe — the symptom, or the thing
   they want to be able to do — in plain language. Skip file paths and module names unless the
   reader cannot otherwise locate the issue.
2. Add a visual the forge renders inline: stills for anything on screen, a short recording for
   a multi-step interaction, a Mermaid `flowchart`/`stateDiagram` for a state or job change.
   No attachment carries personally identifiable information — gist titles, filenames, and
   usernames from a real account count, so record against a throwaway account or crop. Upload
   with `gh issue create --attach './bug.png#alt text'`. The flag also works on
   `gh issue comment` and `gh issue edit`, so a visual can land after the issue is open.
3. Close with a collapsed trailer, so the technical detail does not push the human summary
   below the fold:

   ```markdown
   <details><summary>Technical details</summary>

   Version, platform, `gh` version, affected paths, log excerpts.

   </details>
   ```

## Spec issues

An issue an agent implements from inverts that priority. Acceptance criteria, scope, and how
to verify sit above the fold; `<details>` holds only background.

```markdown
## Acceptance criteria

- [ ] One observable outcome per line.

## Scope

- **In**: the screens, modules, or commands this change may touch.
- **Out**: what it must leave alone.

## How to verify

The gate, plus the per-change calls the touched paths ask for.

<details><summary>Background</summary>

Why this came up, prior attempts, links.

</details>
```

An issue with unanswered open questions is not ready to implement. Ask in a comment and leave
it in triage until the answers land.

## Labels

Read the live set with `gh label list --limit 100`. The CLI defaults to 30 and presents that
page as the whole set, so a label past the first page reads as absent. Apply only labels that
already exist; a genuinely missing one is a conversation with the maintainer, never a
`gh label create`.

- **Triage state** — [`triage-labels.md`](triage-labels.md) owns the canonical roles and their
  label strings. It is their only home here.
- **Priority** — one of `P0` critical, `P1` current cycle, `P2` soon, `P3` nice to have,
  `P4` backlog. An issue carries at most one.
- **Release notes** — every pull request carries exactly one category label; the set and the
  rule live in [`conventions.md`](conventions.md).
- **Invitation** — `good first issue` and `help wanted`, on work a maintainer wants outside
  hands on.
- **Disposition** — `question` for a support request, `duplicate` alongside a comment pointing
  at the original, `invalid` for a report that is not one.
- **Automated** — `dependencies`, `rust`, and `github_actions` are Dependabot's. Leave them to it.
- **Wayfinding** — the `wayfinder:*` labels named below are not in the live set. `/wayfinder`
  needs the maintainer to create them first.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests; `/triage` reads this flag.)_

When set to `yes`, PRs run through the same labels and states as issues, using the `gh pr` equivalents:

- **Read a PR**: `gh pr view <number> --comments` and `gh pr diff <number>` for the diff.
- **List external PRs for triage**: `gh pr list --state open --json number,title,body,labels,author,authorAssociation,comments` then keep only `authorAssociation` of `CONTRIBUTOR`, `FIRST_TIME_CONTRIBUTOR`, or `NONE` (drop `OWNER`/`MEMBER`/`COLLABORATOR`).
- **Comment / label / close**: `gh pr comment`, `gh pr edit --add-label`/`--remove-label`, `gh pr close`.

GitHub shares one number space across issues and PRs, so a bare `#42` may be either — resolve with `gh pr view 42` and fall back to `gh issue view 42`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --comments`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets.

- **Map**: a single issue labelled `wayfinder:map`, holding the Notes / Decisions-so-far / Fog body. `gh issue create --label wayfinder:map`.
- **Child ticket**: an issue linked to the map as a GitHub sub-issue (`gh api` on the sub-issues endpoint). Where sub-issues aren't enabled, add the child to a task list in the map body and put `Part of #<map>` at the top of the child body. Labels: `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, the ticket is assigned to the driving dev.
- **Blocking**: GitHub's **native issue dependencies** — the canonical, UI-visible representation. Add an edge with `gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`, where `<blocker-db-id>` is the blocker's numeric **database id** (`gh api repos/<owner>/<repo>/issues/<n> --jq .id`, _not_ the `#number` or `node_id`). GitHub reports `issue_dependencies_summary.blocked_by` (open blockers only — the live gate). Where dependencies aren't available, fall back to a `Blocked by: #<n>, #<n>` line at the top of the child body. A ticket is unblocked when every blocker is closed.
- **Frontier query**: list the map's open children (`gh issue list --state open`, scoped to the map's sub-issues / task list), drop any with an open blocker (`issue_dependencies_summary.blocked_by > 0`, or an open issue in the `Blocked by` line) or an assignee; first in map order wins.
- **Claim**: `gh issue edit <n> --add-assignee @me` — the session's first write.
- **Resolve**: `gh issue comment <n> --body "<answer>"`, then `gh issue close <n>`, then append a context pointer (gist + link) to the map's Decisions-so-far.
