//! One table per screen describing every key that screen binds (issue #369).
//!
//! The table is a *description*, not a dispatcher and not a gate: `handle_key_*` still executes
//! keys, and `*_guard` still decides whether an action is available right now (issue #288). What
//! lives here is the wording and the classification that used to be hand-written once per
//! rendering — the palette row, the footer hint, and the action colour — so the four copies of
//! "what does `d` do" cannot drift apart again (#344, #346).
//!
//! Help topics and `README.md` stay hand-written prose: the List topic is fifty lines of
//! sectioned explanation, and flattening it into a generated key list would cost more than the
//! consistency it buys. Tests check them against this table instead.

use super::Screen;
use crossterm::event::KeyCode;

/// What a key risks, which is what its accent colour tells the reader — the fact `action_color`
/// used to infer by word-splitting the label, with a comment admitting the guess was fragile.
///
/// The line is drawn at *the user's data*, not at "does anything persist": changing a setting
/// writes `config.toml` but is still [`Nav`](Category::Nav), because no gist and no local file
/// moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Moving around, switching panes, opening or leaving a screen, changing what is displayed,
    /// adjusting a setting.
    Nav,
    /// Looking at content, or copying it out, without changing it.
    Read,
    /// Changing a gist, a local file, or the pin list.
    Write,
    /// Removing something that does not come back.
    Destructive,
}

/// A binding's place in its screen's footer. `None` on [`Binding::footer`] keeps the key off the
/// footer entirely — most keys are palette-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FooterHint {
    /// Position in the footer, which is deliberately not the palette's order: the footer leads
    /// with navigation, the palette leads with actions.
    pub rank: u8,
    /// How the footer writes the key. Not always [`Binding::key_hint`] — the footer names every
    /// key that leaves a screen (`Esc/q`), the palette names only the one it executes (`q`).
    pub key: &'static str,
    /// Footer wording. The footer paints `"{key} {text}"`.
    pub text: &'static str,
}

/// One key on one screen, described once.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Binding {
    /// How the palette and help write the key: `"d"`, `"Tab"`, `"h/l"`, `"↑↓"`.
    pub key_hint: &'static str,
    /// The key the palette executes for this row, and the key its guard is asked about.
    ///
    /// `None` is a binding the palette never shows — pure navigation (`↑↓`, `Esc/q`), described
    /// here only for the footer and help. Where `key_hint` covers several keys (`"h/l"`), this
    /// is the one the palette sends; the table describes bindings, so which keys reach
    /// `handle_key_*` is that function's business, not this one's.
    pub code: Option<KeyCode>,
    /// Palette row text: `"Download gist → cwd"`.
    pub label: &'static str,
    pub category: Category,
    pub footer: Option<FooterHint>,
    /// Whether the screen's `*_guard` decides this key's availability. `false` means the palette
    /// row is always enabled.
    pub guarded: bool,
}

impl Binding {
    /// A palette action gated by the screen's `*_guard`.
    const fn action(
        key_hint: &'static str,
        code: KeyCode,
        label: &'static str,
        category: Category,
    ) -> Self {
        Self {
            key_hint,
            code: Some(code),
            label,
            category,
            footer: None,
            guarded: true,
        }
    }

    /// A palette action that is always available.
    const fn always(
        key_hint: &'static str,
        code: KeyCode,
        label: &'static str,
        category: Category,
    ) -> Self {
        Self {
            guarded: false,
            ..Self::action(key_hint, code, label, category)
        }
    }

    /// Navigation described for the footer and help, with no palette row and so no key for the
    /// palette to send.
    const fn nav(key_hint: &'static str, label: &'static str) -> Self {
        Self {
            code: None,
            ..Self::always(key_hint, KeyCode::Null, label, Category::Nav)
        }
    }

    /// Put this binding on the footer at `rank`, worded `"{key} {text}"`.
    const fn footer(mut self, rank: u8, key: &'static str, text: &'static str) -> Self {
        self.footer = Some(FooterHint { rank, key, text });
        self
    }
}

/// Every key `screen` binds, in palette order. Screens with no palette of their own (the palette
/// itself, the confirm modal) return an empty table.
pub(crate) fn for_screen(screen: &Screen) -> &'static [Binding] {
    match screen {
        Screen::List => LIST,
        Screen::Pins(_) => PINS,
        Screen::Gists(_) => GISTS,
        Screen::GistDetail(_) => DETAIL,
        Screen::Revisions(_) => REVISIONS,
        Screen::Diff(_) => DIFF,
        Screen::Preview(_) => PREVIEW,
        Screen::Help(_) => HELP,
        Screen::Config(_) => CONFIG,
        Screen::Confirm(_) | Screen::Palette(_) => &[],
    }
}

/// The screen's footer hint string, in `rank` order — the wording the footer used to carry as a
/// hand-written `*_HINTS` constant. Screens whose bindings all sit off the footer get `""`; they
/// paint `MINIMAL_HINT` instead.
pub(crate) fn footer_hints(screen: &Screen) -> String {
    let mut hints: Vec<FooterHint> = for_screen(screen)
        .iter()
        .filter_map(|binding| binding.footer)
        .collect();
    hints.sort_unstable_by_key(|hint| hint.rank);
    hints
        .iter()
        .map(|hint| format!("{} {}", hint.key, hint.text))
        .collect::<Vec<_>>()
        .join(FOOTER_SEPARATOR)
}

/// How the footer joins its items. `fit_hints` splits on the `·` to drop whole items when the
/// terminal is too narrow (issue #342).
pub(crate) const FOOTER_SEPARATOR: &str = "  ·  ";

/// The category of the binding a footer item's leading key token names, for accenting that key.
/// A token no binding claims — `;` and `Ctrl+p` in `MINIMAL_HINT`, or
/// a status message that is not a hint list at all — reads as [`Category::Nav`].
pub(crate) fn category_for_footer_key(bindings: &[Binding], key: &str) -> Category {
    bindings
        .iter()
        .filter_map(|binding| binding.footer.map(|hint| (hint.key, binding.category)))
        .find(|(hint_key, _)| *hint_key == key)
        .map_or(Category::Nav, |(_, category)| category)
}

use Category::{Destructive, Nav, Read, Write};

const LIST: &[Binding] = &[
    Binding::action("Enter", KeyCode::Enter, "Diff local ↔ gist", Read).footer(2, "Enter", "diff"),
    Binding::action("Space", KeyCode::Char(' '), "Preview gist content", Read),
    Binding::action("d", KeyCode::Char('d'), "Download gist → cwd", Write)
        .footer(3, "d", "download"),
    Binding::action("u", KeyCode::Char('u'), "Upload local → gist", Write).footer(4, "u", "upload"),
    Binding::action("n", KeyCode::Char('n'), "Create gist from local", Write)
        .footer(5, "n", "new gist"),
    Binding::action("p", KeyCode::Char('p'), "Pin / unpin pair", Write).footer(6, "p", "pin"),
    Binding::always("P", KeyCode::Char('P'), "Open Pins view", Nav).footer(7, "P", "pins"),
    Binding::action("g", KeyCode::Char('g'), "Open Gist manager", Nav).footer(8, "g", "gists"),
    Binding::action("S", KeyCode::Char('S'), "Smart-sync pinned pair", Write),
    Binding::action(
        "X",
        KeyCode::Char('X'),
        "Remove file from gist",
        Destructive,
    ),
    Binding::action("e", KeyCode::Char('e'), "Edit local file", Write),
    Binding::action("y", KeyCode::Char('y'), "Copy gist URL", Read),
    Binding::action("H", KeyCode::Char('H'), "Revision history", Read),
    Binding::action("*", KeyCode::Char('*'), "Star / unstar gist", Write),
    Binding::always("r", KeyCode::Char('r'), "Toggle recursive scan", Nav),
    Binding::always("/", KeyCode::Char('/'), "Filter focused pane", Nav).footer(9, "/", "filter"),
    Binding::always("Tab", KeyCode::Tab, "Switch pane", Nav).footer(1, "Tab", "panes"),
    Binding::always("a", KeyCode::Char('a'), "Flip ranking anchor", Nav),
    Binding::always("t", KeyCode::Char('t'), "Toggle description / id", Nav),
    Binding::always("v", KeyCode::Char('v'), "Cycle gist visibility", Nav),
    Binding::always("s", KeyCode::Char('s'), "Cycle pane sort", Nav),
    Binding::always("?", KeyCode::Char('?'), "Help", Nav),
    Binding::nav("↑↓", "Move the selection").footer(0, "↑↓", "move"),
    Binding::nav("Esc/q", "Leave the screen").footer(10, "Esc/q", "back"),
];

const GISTS: &[Binding] = &[
    Binding::action("Enter", KeyCode::Enter, "Open gist detail", Read).footer(1, "Enter", "detail"),
    Binding::action("o", KeyCode::Char('o'), "Open in browser", Read).footer(3, "o", "browser"),
    Binding::action("y", KeyCode::Char('y'), "Copy gist URL", Read).footer(4, "y", "copy URL"),
    Binding::action("H", KeyCode::Char('H'), "Revision history", Read).footer(5, "H", "revisions"),
    Binding::action("*", KeyCode::Char('*'), "Star / unstar gist", Write).footer(2, "*", "star"),
    Binding::always("/", KeyCode::Char('/'), "Filter gists", Nav).footer(8, "/", "filter"),
    Binding::always("s", KeyCode::Char('s'), "Cycle sort", Nav).footer(6, "s", "sort"),
    Binding::always("v", KeyCode::Char('v'), "Cycle visibility", Nav).footer(7, "v", "visibility"),
    Binding::always("q", KeyCode::Char('q'), "Back to list", Nav),
    Binding::always("?", KeyCode::Char('?'), "Help", Nav),
    Binding::nav("↑↓", "Move the selection").footer(0, "↑↓", "move"),
    Binding::nav("Esc/q", "Leave the screen").footer(9, "Esc/q", "back"),
];

const PINS: &[Binding] = &[
    Binding::action("Enter", KeyCode::Enter, "Diff pinned pair", Read).footer(1, "Enter", "diff"),
    Binding::action("s", KeyCode::Char('s'), "Smart-sync", Write).footer(2, "s", "sync"),
    Binding::action("u", KeyCode::Char('u'), "Force push", Write).footer(3, "u", "push"),
    Binding::action("d", KeyCode::Char('d'), "Force pull", Write).footer(4, "d", "pull"),
    Binding::action("x", KeyCode::Char('x'), "Unpin pair", Destructive).footer(5, "x", "unpin"),
    Binding::always("/", KeyCode::Char('/'), "Filter pins", Nav).footer(7, "/", "filter"),
    Binding::always("o", KeyCode::Char('o'), "Cycle sort", Nav).footer(6, "o", "sort"),
    Binding::always("q", KeyCode::Char('q'), "Back to list", Nav),
    Binding::always("?", KeyCode::Char('?'), "Help", Nav),
    Binding::nav("↑↓", "Move the selection").footer(0, "↑↓", "move"),
    Binding::nav("Esc/q", "Leave the screen").footer(8, "Esc/q", "back"),
];

const DETAIL: &[Binding] = &[
    Binding::action("Enter", KeyCode::Enter, "Preview selected file", Read),
    Binding::action("o", KeyCode::Char('o'), "Open in browser", Read),
    Binding::action("y", KeyCode::Char('y'), "Copy gist URL", Read),
    Binding::action("H", KeyCode::Char('H'), "Revision history", Read),
    Binding::action("e", KeyCode::Char('e'), "Edit description", Write),
    Binding::action("c", KeyCode::Char('c'), "Compact revisions", Nav),
    Binding::action("*", KeyCode::Char('*'), "Star / unstar gist", Write),
    Binding::action("F", KeyCode::Char('F'), "Fork gist", Write),
    Binding::action("X", KeyCode::Char('X'), "Delete gist", Destructive),
    Binding::always("Tab", KeyCode::Tab, "Switch Files / Comments", Nav),
    Binding::action("m", KeyCode::Char('m'), "Load older comments", Read),
    Binding::always("q", KeyCode::Char('q'), "Back to Gist manager", Nav),
    Binding::always("?", KeyCode::Char('?'), "Help", Nav),
];

const REVISIONS: &[Binding] = &[
    Binding::action("Enter", KeyCode::Enter, "Diff parent → revision", Read),
    Binding::action("D", KeyCode::Char('D'), "Diff revision vs head", Read),
    Binding::action("r", KeyCode::Char('r'), "Restore revision", Write),
    // The palette *is* gated through `revisions_guard` here, unlike the real handler's own `F`
    // arm (which stays unconditional — see the comment on that case in `revisions_guard`):
    // cycling the target file doesn't need the revision list loaded, so `revisions_guard`'s `F`
    // case checks file count, not `has_entries` (issue #288).
    Binding::action("F", KeyCode::Char('F'), "Cycle target file", Nav),
    Binding::always("q", KeyCode::Char('q'), "Back", Nav),
    Binding::always("?", KeyCode::Char('?'), "Help", Nav),
];

const DIFF: &[Binding] = &[
    Binding::action("d", KeyCode::Char('d'), "Download", Write),
    Binding::action("u", KeyCode::Char('u'), "Upload", Write),
    Binding::always("c", KeyCode::Char('c'), "Toggle full diff context", Nav),
    Binding::always("w", KeyCode::Char('w'), "Toggle line wrap", Nav),
    Binding::always("q", KeyCode::Char('q'), "Back", Nav),
];

const PREVIEW: &[Binding] = &[
    Binding::always("R", KeyCode::Char('R'), "Refresh content", Read),
    Binding::always("w", KeyCode::Char('w'), "Toggle line wrap", Nav),
    Binding::always("y", KeyCode::Char('y'), "Copy gist URL", Read),
    Binding::always("Y", KeyCode::Char('Y'), "Copy file content", Read),
    Binding::always("q", KeyCode::Char('q'), "Back", Nav),
];

const CONFIG: &[Binding] = &[
    Binding::always("Enter", KeyCode::Enter, "Toggle / increase value", Nav),
    Binding::always("h/l", KeyCode::Char('l'), "Decrease / increase value", Nav),
    Binding::always("Esc", KeyCode::Esc, "Close settings", Nav),
];

const HELP: &[Binding] = &[
    Binding::always("Tab", KeyCode::Tab, "Browse topic index", Nav),
    Binding::always("q", KeyCode::Char('q'), "Close Help", Nav),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every screen's table, for the invariants that must hold across all of them.
    fn all_tables() -> Vec<(&'static str, &'static [Binding])> {
        vec![
            ("List", LIST),
            ("Gists", GISTS),
            ("Pins", PINS),
            ("Detail", DETAIL),
            ("Revisions", REVISIONS),
            ("Diff", DIFF),
            ("Preview", PREVIEW),
            ("Config", CONFIG),
            ("Help", HELP),
        ]
    }

    /// A footer is built by sorting on `rank`, so two bindings sharing one would paint in an
    /// order the table does not pin down.
    #[test]
    fn footer_ranks_are_unique_within_a_screen() {
        for (name, table) in all_tables() {
            let mut ranks: Vec<u8> = table
                .iter()
                .filter_map(|b| b.footer)
                .map(|f| f.rank)
                .collect();
            let count = ranks.len();
            ranks.sort_unstable();
            ranks.dedup();
            assert_eq!(ranks.len(), count, "{name} has a duplicate footer rank");
        }
    }

    /// The palette executes `code`, so two rows sharing one would send the same key twice.
    #[test]
    fn palette_codes_are_unique_within_a_screen() {
        for (name, table) in all_tables() {
            let mut codes: Vec<KeyCode> = table.iter().filter_map(|b| b.code).collect();
            let count = codes.len();
            codes.sort_by_key(|c| format!("{c:?}"));
            codes.dedup();
            assert_eq!(
                codes.len(),
                count,
                "{name} binds one key to two palette rows"
            );
        }
    }

    /// `for_screen` must answer for every variant; the two screens without a palette of their
    /// own answer with an empty table rather than a missing arm.
    #[test]
    fn for_screen_answers_for_every_variant() {
        assert!(!for_screen(&Screen::List).is_empty());
        assert!(!for_screen(&Screen::Pins(Box::default())).is_empty());
        assert!(!for_screen(&Screen::Gists(Box::default())).is_empty());
        assert!(!for_screen(&Screen::GistDetail(Box::default())).is_empty());
        assert!(!for_screen(&Screen::Revisions(Box::default())).is_empty());
        assert!(!for_screen(&Screen::Diff(Box::default())).is_empty());
        assert!(!for_screen(&Screen::Preview(Box::default())).is_empty());
        assert!(!for_screen(&Screen::Help(Box::default())).is_empty());
        assert!(!for_screen(&Screen::Config(Box::default())).is_empty());
        assert!(for_screen(&Screen::Confirm(Box::default())).is_empty());
        assert!(for_screen(&Screen::Palette(Box::default())).is_empty());
    }

    /// The three footers this table replaced were hand-written constants. Pinning them verbatim
    /// is what makes the move a refactor: `rank` has to reproduce the old order exactly, and the
    /// wording has to survive the trip through `FooterHint`.
    #[test]
    fn derived_footers_match_the_strings_they_replaced() {
        assert_eq!(
            footer_hints(&Screen::List),
            "↑↓ move  ·  Tab panes  ·  Enter diff  ·  d download  ·  u upload  ·  n new gist  ·  p pin  ·  P pins  ·  g gists  ·  / filter  ·  Esc/q back"
        );
        assert_eq!(
            footer_hints(&Screen::Gists(Box::default())),
            "↑↓ move  ·  Enter detail  ·  * star  ·  o browser  ·  y copy URL  ·  H revisions  ·  s sort  ·  v visibility  ·  / filter  ·  Esc/q back"
        );
        assert_eq!(
            footer_hints(&Screen::Pins(Box::default())),
            "↑↓ move  ·  Enter diff  ·  s sync  ·  u push  ·  d pull  ·  x unpin  ·  o sort  ·  / filter  ·  Esc/q back"
        );
    }

    /// The eight screens that never had a footer of their own must not grow one: they paint
    /// `MINIMAL_HINT`, and a stray `footer` on one of their bindings would silently replace it.
    #[test]
    fn screens_without_a_footer_derive_an_empty_one() {
        for screen in [
            Screen::GistDetail(Box::default()),
            Screen::Revisions(Box::default()),
            Screen::Diff(Box::default()),
            Screen::Preview(Box::default()),
            Screen::Help(Box::default()),
            Screen::Config(Box::default()),
            Screen::Confirm(Box::default()),
            Screen::Palette(Box::default()),
        ] {
            assert_eq!(footer_hints(&screen), "", "{screen:?} grew a footer");
        }
    }

    /// A footer key the table does not claim keeps the navigation accent rather than picking up
    /// whichever binding happens to sort first.
    #[test]
    fn an_unclaimed_footer_key_reads_as_navigation() {
        assert_eq!(category_for_footer_key(LIST, ";"), Category::Nav);
        assert_eq!(category_for_footer_key(LIST, "d"), Category::Write);
        // `q` alone is a palette row on some screens; the footer writes the leave key `Esc/q`.
        assert_eq!(category_for_footer_key(GISTS, "q"), Category::Nav);
    }

    /// A binding with no palette row has no key for a guard to be asked about, so guarding one
    /// would be a rule that never runs.
    #[test]
    fn navigation_bindings_are_never_guarded() {
        for (name, table) in all_tables() {
            for binding in table.iter().filter(|b| b.code.is_none()) {
                assert!(
                    !binding.guarded,
                    "{name}'s {} is not a palette row, so no guard can apply to it",
                    binding.key_hint
                );
            }
        }
    }
}

/// The drift nets for the two renderings that stay hand-written prose (issue #369).
///
/// Deriving Help and `README.md` from the table would flatten explanation that earns its keep;
/// checking them against it costs nothing and catches the same drift — the #344 direction (a key
/// the docs forgot) and the #346 direction (a key the docs invented or mis-cased).
#[cfg(test)]
mod docs_tests {
    use super::*;
    use crate::tui::screens::help::help_topic_body;
    use crate::tui::HelpTopic;

    /// The topic that documents each screen's keys. Not `screens::*::HELP_TOPIC`, which answers a
    /// different question — where `?` jumps to from that screen.
    fn topics() -> Vec<(&'static str, &'static [Binding], HelpTopic)> {
        vec![
            ("List", LIST, HelpTopic::List),
            ("Gists", GISTS, HelpTopic::GistManager),
            ("Pins", PINS, HelpTopic::Pins),
            ("Detail", DETAIL, HelpTopic::GistDetail),
            ("Revisions", REVISIONS, HelpTopic::Revisions),
            ("Diff", DIFF, HelpTopic::Diff),
            ("Preview", PREVIEW, HelpTopic::Preview),
            ("Config", CONFIG, HelpTopic::Config),
            ("Help", HELP, HelpTopic::General),
        ]
    }

    /// Every key a help body names, read from the key column alone.
    ///
    /// A key line is `"<keys>  <description>"`. The double space is what separates the key
    /// column from the description, and is a more reliable marker than the leading indent, which
    /// Rust's `\`-continuation strips from a body's first line — the line carrying `Esc / q`.
    /// `/` separates alternatives (`Esc / q`, `Up/Down`), so it splits too.
    ///
    /// Only the key column counts. Reading keys out of descriptions too would let prose vouch
    /// for a binding: a topic that had lost its `a` line would still pass on the strength of an
    /// unrelated sentence starting with "a". That is why every key documented here gets its own
    /// line rather than sharing one behind a `·`.
    fn documented_keys(topic: HelpTopic) -> Vec<String> {
        help_topic_body(topic)
            .lines()
            .filter_map(|line| line.trim().split_once("  "))
            .flat_map(|(keys, _)| keys.split('/'))
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .collect()
    }

    /// A binding's keys, as the table writes them: `"h/l"` is two keys, `"Esc/q"` is two.
    fn binding_keys(binding: &Binding) -> Vec<&str> {
        binding
            .key_hint
            .split('/')
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .collect()
    }

    /// Issue #344's direction: a key the table binds but no help topic mentions is a key the user
    /// cannot discover. Screen-wide keys (`?`, `Esc`/`q`) live on the General topic rather than
    /// being repeated on every screen, so both bodies count.
    #[test]
    fn every_bound_key_is_documented_in_help() {
        let general = documented_keys(HelpTopic::General);
        for (name, table, topic) in topics() {
            let mut documented = documented_keys(topic);
            documented.extend(general.iter().cloned());
            for binding in table.iter().filter(|b| b.code.is_some()) {
                for key in binding_keys(binding) {
                    assert!(
                        documented.iter().any(|d| d == key),
                        "{name}'s `{key}` ({}) is bound but appears in neither the {topic:?} help \
                         topic nor General",
                        binding.label
                    );
                }
            }
        }
    }

    /// Issue #346's direction: `README.md` promises keys the app must actually bind. The full
    /// Key/Action table has moved to `?` Help and the project page, so what the README now
    /// promises is the Quick start prose: every single-token code span in it is a key claim.
    #[test]
    fn every_key_readme_promises_is_bound() {
        let readme = include_str!("../../README.md");
        let section: Vec<&str> = readme
            .lines()
            .skip_while(|line| !line.starts_with("## Quick start"))
            .skip(1)
            .take_while(|line| !line.starts_with("## "))
            .collect();
        assert!(
            !section.is_empty(),
            "README's Quick start section moved or changed shape"
        );

        // The fenced block holds shell commands, not key promises.
        let mut in_fence = false;
        let prose: Vec<&str> = section
            .iter()
            .filter(|line| {
                if line.starts_with("```") {
                    in_fence = !in_fence;
                    return false;
                }
                !in_fence
            })
            .copied()
            .collect();

        let bound: Vec<&str> = LIST
            .iter()
            .flat_map(binding_keys)
            .chain(GISTS.iter().flat_map(binding_keys))
            .collect();
        // Keys the README documents that belong to another screen or to the global set, neither
        // of which the List and Gists tables carry.
        let elsewhere = [
            "C", "T", ";", "Ctrl+p", "?", "j", "k", "h", "l", "q", "Esc", "1", "2",
        ];

        let joined = prose.join("\n");
        let mut promised = 0;
        for span in joined.split('`').skip(1).step_by(2) {
            // A code span with a space in it is a command, not a key.
            if span.is_empty() || span.contains(' ') || elsewhere.contains(&span) {
                continue;
            }
            promised += 1;
            assert!(
                bound.contains(&span),
                "README's Quick start promises `{span}`, which no List or Gists binding provides"
            );
        }
        assert!(
            promised > 0,
            "README's Quick start no longer names any key the List or Gists screens bind"
        );
    }
}
