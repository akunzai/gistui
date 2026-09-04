# Product design

Long-term principles for the TUI's visible surface, its wording, and the README. Screen
mechanics live in [`docs/agents/architecture.md`](architecture.md); this file governs
what the user reads and sees.

## Product thesis

gistui is a working desk for gists, not a gist browser. Every screen should let the user tell
which gist they are on, what state it is in relative to their local files, and what the next
key does.

The experience is compact, aligned, and recognizable through structure and wording rather
than decoration.

## Voice

Write like an operator documenting their own tool.

- Active voice, sentence case, concrete verbs, stable product terms.
- Keep an action's verb the same from footer hint through confirm prompt through result.
- Errors and confirmations state what happened and what to do; they do not apologize.
- State a recovery action only when there is one.
- Personality belongs in the README and the Help topics, not in row labels or status lines.

Avoid emoji, emotional reactions, stacked punctuation, filler adverbs, and claims that the
tool makes anything effortless or magical.

## Product language

Use the terms the user acts on. `CONTEXT.md` is the source of truth for domain nouns; these
are the presentation-layer terms:

- **Anchor** is the pane that drives match ranking. `a` flips it; it is independent of focus.
- **Pin** is a persistent local-file to gist-file mapping. **Sync** reconciles a pinned pair.
- **Push** sends local to gist; **pull** brings gist to local. Do not mix in
  upload/download wording once a screen has committed to one pair.
- **Revision** is a gist commit; **restore** brings one back.
- Gists the user does not own are **read-only**, not "locked" or "protected".

## Rows and columns

A list row reads left to right as: badge, identity, id, metadata, age.

```text
★  My ZSH profile  #616796d  3 files  ☆ 4  4d
   My Antigravity CLI settings  #dd0e600  4 files  ☆ 1  1 comment  1d
 ⑂ Magento DB backup script  #8a2a283  1 file  2mo
```

- The badge is a fixed-width column (three cells) so identity starts at the same column on
  every row, and the id column is fixed-width too, so a legacy short id does not shift what
  follows it. The rest flows: a description worth reading beats a ruled column.
- Metadata appears only when it is non-zero. Quiet rows stay quiet.
- Age is last; its position carries the meaning, so it needs no label.
- A count that needs a word gets the word (`3 files`, `1 comment`), not a picture.

## Marks and terminal capability

Only these marks are used, and only where the meaning is unambiguous in context:

| Mark | Meaning | Where |
| --- | --- | --- |
| `★` / `☆` | you starred it / stargazer count | gist rows, gist info line |
| `⑂` | fork | gist rows, gist info line |
| `⚑` | this pane is the anchor | List pane titles |
| `↔` | a pinned local/gist pair | List rows, Pins rows |
| `✓ ↑ ↓ ✕ ?` | pin sync status | Pins rows, with a legend on screen |
| `▶` / `▸` | the selected row | every list |

Every mark is single-width. A double-width glyph misaligns the columns beside it and forces
width special-cases into the fitting code, so a mark that renders wide is not a candidate.

Emoji are never a mark and never a fallback. A meaning that needs an emoji to be legible
needs a short text label instead. This rule covers what gistui itself draws — the TUI, its
CLI output, and the README. GitHub's own surfaces keep GitHub's conventions: the release-note
category emoji in `.github/release.yml` are how a reader expects a releases page to look.

Colour reinforces status or hierarchy; it never carries meaning alone. Honour `NO_COLOR` for
syntax highlighting, and keep the semantic diff `-`/`+` colours, which the diff would be
unreadable without.

## Screens

Default output leaves one durable result on screen. Transient progress is a spinner on the
one pane that is waiting, not a modal.

An empty list says what is empty and, when the emptiness is the user's own filter, says so:

```text
No gists found
No gists match the filter
No pinned mappings yet — press p on a local/gist pair to add one
```

Every screen answers "what can I press" without the user asking: a footer of the primary
actions, the `(g)ists (P)ins (C)onfig (?)Help` bar, and `?` for the full contextual keymap.
A new key is not shipped until all three know about it.

## Confirms

A confirm asks one question and lists the keys that answer it. The two are never the same
line: the question and its consequence come first, then a blank row, then the keys.

```text
╭ Compact revisions ─────────────────────────────────────╮
│                                                        │
│  Compact 12 revisions of "My ZSH profile" into one?    │
│  This force-pushes and cannot be undone.               │
│                                                        │
│  n  cancel        y  compact                           │
│                                                        │
╰────────────────────────────────────────────────────────╯
```

- Every key is labelled with the verb it performs — `y delete`, `y upload`, `y overwrite` —
  the same word the footer hint used and the resulting status will use. Never a bare `(y/n)`.
- A destructive action states its consequence on the second line and puts `n cancel` first.
  The border colour reinforces that; it never carries it alone.
- Toggles that change what the primary key would do (`p pretty`, `s sort`) go on their own
  row beneath the actions, in the same columns.
- Keys that do not all fit one row are packed onto further rows, never word-wrapped: a key
  separated from its own verb is worse than a taller modal.
- Every confirm is the same width, so the two steps of one action do not resize between
  keystrokes.

## Interaction

- A write that can lose data shows the diff first and takes `y/n`. This is not configurable
  away.
- Confirm destructive remote actions individually; never batch them behind one prompt.
- Preferences are editable in the app (`C`); the config file is for the fields that are not.
- Keys stay where muscle memory expects them. Adding a screen does not renumber the top bar.

## README and project page

The README's first job is to get a new user from nothing to one successful gist operation.
Its second job is to say why the tool exists next to `gh gist` and the web UI.

One plain heading, a short value proposition, one install path, one quick start, one real
terminal demo. Reference material that the app itself already carries — the full keymap, the
mouse table, the config schema — lives in `?` Help, `config.example.toml`, and `docs/`, and
is linked, not duplicated. Document released behaviour, not plans.

`website/index.html` makes the same case to someone who has not installed anything: the demo
above the fold, then why, then the keys themselves. It borrows the TUI's palette so the page
and the app read as one thing. Feature cards are not a substitute for showing the tool —
a claim that the demo or the keymap already makes does not also get a card.

The demo uses fixed fixture data, dimensions, theme, and timing, and exposes no local
username or home path. Regeneration rules live in [`docs/demo.md`](../demo.md).
