use super::keymap::Category;
use super::{keys::point_in, *};
use crossterm::event::{KeyCode, KeyModifiers};

/// Menu = context-filtered actions near the click; Command = full list + fuzzy query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaletteMode {
    #[default]
    Menu,
    Command,
}

/// How selecting a palette row is executed once the overlay closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteExec {
    Key(KeyCode, KeyModifiers),
    Cross(CrossAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossAction {
    GoToGists,
    GoToPins,
    OpenHelp,
    OpenConfig,
    ToggleTheme,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub key_hint: String,
    pub label: String,
    pub exec: PaletteExec,
    pub enabled: bool,
    /// What the action risks, which accents its key (see [`Category`]).
    pub category: Category,
    /// Lowercased key+label string used for fuzzy filtering in command mode.
    pub search: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaletteState {
    pub mode: PaletteMode,
    pub query: TextInput,
    pub items: Vec<PaletteItem>,
    pub selected: usize,
    pub origin_screen: Screen,
    /// Menu-mode anchor (terminal col, row); command mode leaves this `None`.
    pub anchor: Option<(u16, u16)>,
}

impl AppState {
    /// Whether a global palette opener (`;` / `Ctrl+p`) should be ignored right now.
    pub(crate) fn palette_blocked(&self) -> bool {
        self.screen.is_confirm() || self.is_any_filtering() || self.editing_description
    }

    pub(crate) fn open_palette_menu(&mut self, anchor: Option<(u16, u16)>) {
        let origin = self.screen.clone();
        let items = build_palette_items(self, &origin, PaletteMode::Menu);
        if items.is_empty() {
            self.set_status("no actions available");
            return;
        }
        self.screen = Screen::Palette(Box::new(PaletteState {
            mode: PaletteMode::Menu,
            items,
            origin_screen: origin,
            anchor,
            ..PaletteState::default()
        }));
    }

    pub(crate) fn open_palette_command(&mut self) {
        let origin = self.screen.clone();
        let items = build_palette_items(self, &origin, PaletteMode::Command);
        self.screen = Screen::Palette(Box::new(PaletteState {
            mode: PaletteMode::Command,
            items,
            origin_screen: origin,
            ..PaletteState::default()
        }));
    }

    pub(crate) fn close_palette(&mut self) {
        let origin = self
            .palette()
            .map(|p| p.origin_screen.clone())
            .unwrap_or(Screen::List);
        self.screen = origin;
    }

    /// Visible palette rows after mode-specific filtering and (in command mode) fuzzy query.
    pub(crate) fn palette_visible_items(&self) -> Vec<&PaletteItem> {
        let Some(p) = self.palette() else {
            return Vec::new();
        };
        let query: &str = &p.query;
        let mut matched: Vec<(&PaletteItem, u32)> = p
            .items
            .iter()
            .filter_map(|item| {
                if p.mode == PaletteMode::Menu && !item.enabled {
                    return None;
                }
                fuzzy_match(query, &item.search).map(|score| (item, score))
            })
            .collect();
        if p.mode == PaletteMode::Command && !query.is_empty() {
            matched.sort_by_key(|item| std::cmp::Reverse(item.1));
        }
        matched.into_iter().map(|(item, _)| item).collect()
    }

    pub(crate) fn palette_clamp_selection(&mut self) {
        let len = self.palette_visible_items().len();
        if let Some(p) = self.palette_mut() {
            if len == 0 {
                p.selected = 0;
            } else if p.selected >= len {
                p.selected = len - 1;
            }
        }
    }

    pub(crate) fn palette_click(&mut self, col: u16, row: u16, layout: &MouseLayout) -> KeyOutcome {
        if let Some(rect) = layout.palette_close {
            if point_in(rect, col, row) {
                self.close_palette();
                return KeyOutcome::None;
            }
        }
        for (i, rect) in layout.palette_rows.iter().enumerate() {
            if point_in(*rect, col, row) {
                if let Some(p) = self.palette_mut() {
                    p.selected = i;
                }
                return self.execute_palette_selection();
            }
        }
        KeyOutcome::None
    }
}

fn palette_item(
    key: &str,
    label: &str,
    exec: PaletteExec,
    enabled: bool,
    category: Category,
) -> PaletteItem {
    PaletteItem {
        key_hint: key.to_string(),
        label: label.to_string(),
        exec,
        enabled,
        category,
        search: format!("{key} {label}").to_ascii_lowercase(),
    }
}

pub(super) fn key_item(
    key: &str,
    label: &str,
    code: KeyCode,
    enabled: bool,
    category: Category,
) -> PaletteItem {
    palette_item(
        key,
        label,
        PaletteExec::Key(code, KeyModifiers::NONE),
        enabled,
        category,
    )
}

fn cross_items() -> Vec<PaletteItem> {
    // includes Open settings
    vec![
        palette_item(
            "g",
            "Go to Gists",
            PaletteExec::Cross(CrossAction::GoToGists),
            true,
            Category::Nav,
        ),
        palette_item(
            "P",
            "Go to Pins",
            PaletteExec::Cross(CrossAction::GoToPins),
            true,
            Category::Nav,
        ),
        palette_item(
            "?",
            "Go to Help",
            PaletteExec::Cross(CrossAction::OpenHelp),
            true,
            Category::Nav,
        ),
        palette_item(
            "C",
            "Open settings",
            PaletteExec::Cross(CrossAction::OpenConfig),
            true,
            Category::Nav,
        ),
        palette_item(
            "T",
            "Toggle theme",
            PaletteExec::Cross(CrossAction::ToggleTheme),
            true,
            Category::Nav,
        ),
        palette_item(
            "q",
            "Quit",
            PaletteExec::Cross(CrossAction::Quit),
            true,
            Category::Nav,
        ),
    ]
}

/// Whether the screen's own guard says `code` is available right now (issue #288). Screens
/// whose keys are never gated — and the two with no table at all — answer `true`.
fn guard_for(state: &AppState, screen: &Screen, code: KeyCode) -> bool {
    (super::screens::lookup(screen).guard)(state, code)
}

/// The screen's palette rows, in the keymap table's order. Command mode appends the
/// screen-independent jumps.
fn build_palette_items(state: &AppState, screen: &Screen, mode: PaletteMode) -> Vec<PaletteItem> {
    let mut items: Vec<PaletteItem> = super::keymap::for_screen(screen)
        .iter()
        .filter_map(|binding| binding.code.map(|code| (binding, code)))
        .map(|(binding, code)| {
            let enabled = !binding.guarded || guard_for(state, screen, code);
            key_item(
                binding.key_hint,
                binding.label,
                code,
                enabled,
                binding.category,
            )
        })
        .collect();
    if mode == PaletteMode::Command {
        items.extend(cross_items());
    }
    items
}

/// Subsequence fuzzy match: every query char must appear in order in `target`.
/// Returns a higher score for tighter matches (used to sort command-mode results).
pub(crate) fn fuzzy_match(query: &str, target: &str) -> Option<u32> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let t = target.to_ascii_lowercase();
    let q_chars: Vec<char> = q.chars().collect();
    let mut qi = 0usize;
    let mut score = 0u32;
    let mut prev_match: Option<usize> = None;
    for (ti, tc) in t.chars().enumerate() {
        if qi < q_chars.len() && tc == q_chars[qi] {
            score += 10;
            if ti > 0 && prev_match == Some(ti - 1) {
                score += 5;
            }
            if ti == 0 || t.chars().nth(ti.saturating_sub(1)) == Some(' ') {
                score += 3;
            }
            prev_match = Some(ti);
            qi += 1;
        }
    }
    if qi == q_chars.len() {
        Some(score)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crossterm::event::KeyCode;
    use crossterm::event::KeyModifiers;
    use std::path::PathBuf;

    #[test]
    fn fuzzy_match_empty_query_matches_all() {
        assert_eq!(fuzzy_match("", "download gist"), Some(0));
    }

    #[test]
    fn fuzzy_match_subsequence() {
        assert!(fuzzy_match("dl", "d download gist").is_some());
        assert!(fuzzy_match("xyz", "download gist").is_none());
    }

    // ── build_palette_items dispatch for the 7 previously-untested screens (issue #292) ──

    fn test_gist(gist_id: &str, filename: &str) -> GistFile {
        GistFile {
            description: "demo".into(),
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            ..GistFile::fixture(gist_id, filename)
        }
    }

    fn menu_items(state: &AppState) -> Vec<PaletteItem> {
        build_palette_items(state, &state.screen, PaletteMode::Menu)
    }

    fn item_tuples(items: &[PaletteItem]) -> Vec<(&str, &str, bool)> {
        items
            .iter()
            .map(|i| (i.key_hint.as_str(), i.label.as_str(), i.enabled))
            .collect()
    }

    fn enabled_for(items: &[PaletteItem], label: &str) -> bool {
        items.iter().find(|i| i.label == label).unwrap().enabled
    }

    #[test]
    fn gists_palette_items_disable_selection_actions_when_empty() {
        let mut state = initial_state();
        state.screen = Screen::Gists(Box::default());
        let items = menu_items(&state);
        assert_eq!(
            item_tuples(&items),
            vec![
                ("Enter", "Open gist detail", false),
                ("o", "Open in browser", false),
                ("y", "Copy gist URL", false),
                ("H", "Revision history", false),
                ("*", "Star / unstar gist", false),
                ("/", "Filter gists", true),
                ("s", "Cycle sort", true),
                ("v", "Cycle visibility", true),
                ("q", "Back to list", true),
                ("?", "Help", true),
            ]
        );
    }

    #[test]
    fn gists_palette_items_enable_selection_actions_when_selected() {
        let mut state = initial_state();
        state.gists = vec![test_gist("g1", "a.txt")];
        state.screen = Screen::Gists(Box::default());
        let items = menu_items(&state);
        assert!(items.iter().all(|i| i.enabled));
    }

    #[test]
    fn detail_palette_items_disable_everything_without_a_gist() {
        let mut state = initial_state();
        state.screen = Screen::GistDetail(Box::default());
        let items = menu_items(&state);
        assert_eq!(
            item_tuples(&items),
            vec![
                ("Enter", "Preview selected file", false),
                ("o", "Open in browser", false),
                ("y", "Copy gist URL", false),
                ("H", "Revision history", false),
                ("e", "Edit description", false),
                ("c", "Compact revisions", false),
                ("*", "Star / unstar gist", false),
                ("F", "Fork gist", false),
                ("X", "Delete gist", false),
                ("Tab", "Switch Files / Comments", true),
                ("m", "Load older comments", false),
                ("q", "Back to Gist manager", true),
                ("?", "Help", true),
            ]
        );
    }

    #[test]
    fn detail_palette_items_enable_owned_actions_for_owned_previewable_gist() {
        let mut state = initial_state();
        state.gists = vec![test_gist("g1", "a.txt")];
        state.screen = Screen::GistDetail(Box::new(DetailState {
            gist_id: Some("g1".into()),
            focus: DetailFocus::Files,
            ..DetailState::default()
        }));
        let items = menu_items(&state);
        assert_eq!(
            item_tuples(&items),
            vec![
                ("Enter", "Preview selected file", true),
                ("o", "Open in browser", true),
                ("y", "Copy gist URL", true),
                ("H", "Revision history", true),
                ("e", "Edit description", true),
                ("c", "Compact revisions", true),
                ("*", "Star / unstar gist", true),
                // Owned gist: fork is disabled (can't fork your own gist).
                ("F", "Fork gist", false),
                ("X", "Delete gist", true),
                ("Tab", "Switch Files / Comments", true),
                ("m", "Load older comments", false),
                ("q", "Back to Gist manager", true),
                ("?", "Help", true),
            ]
        );
    }

    #[test]
    fn detail_palette_items_gate_fork_on_ownership() {
        let mut state = initial_state();
        state.gists = vec![test_gist("g1", "a.txt")];
        state.current_user_login = Some("someone-else".into());
        state.screen = Screen::GistDetail(Box::new(DetailState {
            gist_id: Some("g1".into()),
            focus: DetailFocus::Files,
            ..DetailState::default()
        }));
        let items = menu_items(&state);
        assert!(!enabled_for(&items, "Edit description")); // not owned
        assert!(!enabled_for(&items, "Delete gist")); // not owned
        assert!(enabled_for(&items, "Fork gist")); // not owned + has gist -> forkable
    }

    #[test]
    fn detail_palette_items_load_older_comments_requires_comments_focus() {
        // Issue #288: `can_load_older_comments` doesn't check tab focus, so the palette
        // previously showed "Load older comments" enabled even while the Files tab was
        // focused (where `m` is a no-op in `handle_key_detail`).
        let mut state = initial_state();
        state.gists = vec![test_gist("g1", "a.txt")];
        state.screen = Screen::GistDetail(Box::new(DetailState {
            gist_id: Some("g1".into()),
            focus: DetailFocus::Files,
            comments: Some(vec![]),
            comments_loaded_oldest_page: 2,
            ..DetailState::default()
        }));
        let items = menu_items(&state);
        assert!(!enabled_for(&items, "Load older comments")); // Files tab focused

        if let Screen::GistDetail(d) = &mut state.screen {
            d.focus = DetailFocus::Comments;
        }
        let items = menu_items(&state);
        assert!(enabled_for(&items, "Load older comments"));
    }

    #[test]
    fn list_palette_items_pin_requires_ownership_or_an_existing_pin() {
        // Issue #288: `pin_toggle_intent` allows unpinning any already-pinned pair, but only
        // allows creating a *new* pin on an owned gist — the palette previously enabled "Pin
        // / unpin pair" for any local+gist selection regardless of ownership.
        let mut state = initial_state();
        state.current_user_login = Some("me".into());
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("a.txt"),
            pinned: false,
            modified: None,
        }];
        state.gists = vec![GistFile {
            owner_login: "someone-else".into(),
            ..test_gist("g1", "a.txt")
        }];
        let items = menu_items(&state);
        assert!(!enabled_for(&items, "Pin / unpin pair")); // foreign, not yet pinned

        state.pinned.push(crate::domain::PinnedMapping {
            local_path: PathBuf::from("a.txt"),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        });
        let items = menu_items(&state);
        assert!(enabled_for(&items, "Pin / unpin pair")); // foreign, but already pinned (unpin)
    }

    #[test]
    fn revisions_palette_items_gate_diff_and_restore_on_head_position() {
        let mut state = initial_state();
        state.gists = vec![test_gist("g1", "a.txt")];
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some("g1".into()),
            target_file: "a.txt".into(),
            index: 0,
            entries: Some(vec![
                GistRevision {
                    version: "v2".into(),
                    committed_at: "2026-06-10T00:00:00Z".into(),
                    user: "u".into(),
                    change_status: crate::domain::GistRevisionChangeStatus {
                        total: 1,
                        additions: 1,
                        deletions: 0,
                    },
                },
                GistRevision {
                    version: "v1".into(),
                    committed_at: "2026-06-01T00:00:00Z".into(),
                    user: "u".into(),
                    change_status: crate::domain::GistRevisionChangeStatus {
                        total: 2,
                        additions: 2,
                        deletions: 0,
                    },
                },
            ]),
            ..RevisionState::default()
        }));

        // At head (index 0): incremental diff is available, but "vs head" diff and restore
        // are not (there's nothing above head to diff/restore against). "Cycle target file"
        // is also disabled here — not because of head position, but because this gist only
        // has one file (`gist_filenames("g1").len() == 1`).
        let items = menu_items(&state);
        assert_eq!(
            item_tuples(&items),
            vec![
                ("Enter", "Diff parent → revision", true),
                ("D", "Diff revision vs head", false),
                ("r", "Restore revision", false),
                ("F", "Cycle target file", false),
                ("q", "Back", true),
                ("?", "Help", true),
            ]
        );

        // Off head (index 1): diff/restore become available; "Cycle target file" stays
        // disabled (still a single-file gist).
        if let Screen::Revisions(rev) = &mut state.screen {
            rev.index = 1;
        }
        let items = menu_items(&state);
        assert_eq!(
            item_tuples(&items),
            vec![
                ("Enter", "Diff parent → revision", true),
                ("D", "Diff revision vs head", true),
                ("r", "Restore revision", true),
                ("F", "Cycle target file", false),
                ("q", "Back", true),
                ("?", "Help", true),
            ]
        );
    }

    #[test]
    fn revisions_palette_items_disable_everything_before_entries_load() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::default());
        let items = menu_items(&state);
        assert_eq!(
            item_tuples(&items),
            vec![
                ("Enter", "Diff parent → revision", false),
                ("D", "Diff revision vs head", false),
                ("r", "Restore revision", false),
                ("F", "Cycle target file", false),
                ("q", "Back", true),
                ("?", "Help", true),
            ]
        );
    }

    #[test]
    fn revisions_palette_items_enable_cycle_target_file_before_entries_load() {
        // Issue #288: cycling the target file doesn't need the revision list to have
        // loaded — `cycle_revision_target_file` only checks the gist's file count, not
        // `entries`. A multi-file gist should show "Cycle target file" enabled even while
        // entries are still `None`.
        let mut state = initial_state();
        state.gists = vec![test_gist("g1", "a.txt"), test_gist("g1", "b.txt")];
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some("g1".into()),
            target_file: "a.txt".into(),
            ..RevisionState::default()
        }));
        let items = menu_items(&state);
        assert!(enabled_for(&items, "Cycle target file"));
        assert!(!enabled_for(&items, "Diff parent → revision")); // still no entries
    }

    #[test]
    fn diff_palette_items_enable_sync_by_default() {
        let mut state = initial_state();
        state.screen = Screen::Diff(Box::default());
        let items = menu_items(&state);
        assert_eq!(
            item_tuples(&items),
            vec![
                ("d", "Download", true),
                ("u", "Upload", true),
                ("c", "Toggle full diff context", true),
                ("w", "Toggle line wrap", true),
                ("q", "Back", true),
            ]
        );
    }

    #[test]
    fn diff_palette_items_disable_sync_when_identical() {
        let mut state = initial_state();
        state.screen = Screen::Diff(Box::new(DiffState {
            identical: true,
            ..DiffState::default()
        }));
        let items = menu_items(&state);
        assert!(!enabled_for(&items, "Download"));
        assert!(!enabled_for(&items, "Upload"));
        assert!(enabled_for(&items, "Toggle full diff context")); // unrelated to sync gating
    }

    #[test]
    fn diff_palette_items_disable_sync_within_revision_diff_flow() {
        let mut state = initial_state();
        state.nav_stack.push(Screen::Revisions(Box::default()));
        state.screen = Screen::Diff(Box::default());
        let items = menu_items(&state);
        assert!(!enabled_for(&items, "Download"));
        assert!(!enabled_for(&items, "Upload"));
    }

    #[test]
    fn preview_palette_items_are_always_enabled() {
        let mut state = initial_state();
        state.screen = Screen::Preview(Box::default());
        let items = menu_items(&state);
        assert_eq!(
            item_tuples(&items),
            vec![
                ("R", "Refresh content", true),
                ("w", "Toggle line wrap", true),
                ("y", "Copy gist URL", true),
                ("Y", "Copy file content", true),
                ("q", "Back", true),
            ]
        );
    }

    #[test]
    fn config_palette_items_are_always_enabled() {
        let mut state = initial_state();
        state.screen = Screen::Config(Box::default());
        let items = menu_items(&state);
        assert_eq!(
            item_tuples(&items),
            vec![
                ("Enter", "Toggle / increase value", true),
                ("h/l", "Decrease / increase value", true),
                ("Esc", "Close settings", true),
            ]
        );
    }

    #[test]
    fn help_palette_items_are_always_enabled() {
        let mut state = initial_state();
        state.screen = Screen::Help(Box::default());
        let items = menu_items(&state);
        assert_eq!(
            item_tuples(&items),
            vec![
                ("Tab", "Browse topic index", true),
                ("q", "Close Help", true),
            ]
        );
    }

    #[test]
    fn ctrl_p_opens_command_palette() {
        let mut state = crate::tui::initial_state();
        state.handle_key_with(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert!(state.screen.is_palette());
        assert_eq!(
            state.palette().unwrap().mode,
            crate::tui::palette::PaletteMode::Command
        );
    }

    #[test]
    fn menu_palette_hides_disabled_actions() {
        let mut state = crate::tui::initial_state();
        state.open_palette_menu(None);
        let labels: Vec<_> = state
            .palette_visible_items()
            .iter()
            .map(|i| i.label.as_str())
            .collect();
        assert!(!labels.iter().any(|l| l.contains("Upload")));
    }

    #[test]
    fn command_palette_includes_cross_screen_quit() {
        let mut state = crate::tui::initial_state();
        state.open_palette_command();
        assert!(state
            .palette_visible_items()
            .iter()
            .any(|i| i.label == "Quit"));
    }

    #[test]
    fn command_palette_fuzzy_filter_narrows_items() {
        let mut state = crate::tui::initial_state();
        state.open_palette_command();
        let before = state.palette_visible_items().len();
        state.palette_mut().unwrap().query.set("quit");
        let after = state.palette_visible_items().len();
        assert!(after < before);
        assert!(state
            .palette_visible_items()
            .iter()
            .all(|i| i.label.to_ascii_lowercase().contains("quit")));
    }
}
