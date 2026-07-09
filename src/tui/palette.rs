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
    ToggleTheme,
    Quit,
}

#[derive(Debug, Clone)]
pub struct PaletteItem {
    pub key_hint: String,
    pub label: String,
    pub exec: PaletteExec,
    pub enabled: bool,
    /// Lowercased key+label string used for fuzzy filtering in command mode.
    pub search: String,
}

#[derive(Debug, Clone, Default)]
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
        self.screen == Screen::Confirm
            || self.filtering
            || self.pins.filtering
            || self.gist_manager.filtering
            || self.editing_description
    }

    pub(crate) fn open_palette_menu(&mut self, anchor: Option<(u16, u16)>) {
        let origin = self.screen;
        let items = build_palette_items(self, origin, PaletteMode::Menu);
        if items.is_empty() {
            self.set_status("no actions available");
            return;
        }
        self.palette = PaletteState {
            mode: PaletteMode::Menu,
            items,
            origin_screen: origin,
            anchor,
            ..PaletteState::default()
        };
        self.screen = Screen::Palette;
    }

    pub(crate) fn open_palette_command(&mut self) {
        let origin = self.screen;
        let items = build_palette_items(self, origin, PaletteMode::Command);
        self.palette = PaletteState {
            mode: PaletteMode::Command,
            items,
            origin_screen: origin,
            ..PaletteState::default()
        };
        self.screen = Screen::Palette;
    }

    pub(crate) fn close_palette(&mut self) {
        self.screen = self.palette.origin_screen;
        self.palette = PaletteState::default();
    }

    /// Visible palette rows after mode-specific filtering and (in command mode) fuzzy query.
    pub(crate) fn palette_visible_items(&self) -> Vec<&PaletteItem> {
        let query: &str = &self.palette.query;
        let mut matched: Vec<(&PaletteItem, u32)> = self
            .palette
            .items
            .iter()
            .filter_map(|item| {
                if self.palette.mode == PaletteMode::Menu && !item.enabled {
                    return None;
                }
                fuzzy_match(query, &item.search).map(|score| (item, score))
            })
            .collect();
        if self.palette.mode == PaletteMode::Command && !query.is_empty() {
            matched.sort_by_key(|item| std::cmp::Reverse(item.1));
        }
        matched.into_iter().map(|(item, _)| item).collect()
    }

    fn palette_clamp_selection(&mut self) {
        let len = self.palette_visible_items().len();
        if len == 0 {
            self.palette.selected = 0;
        } else if self.palette.selected >= len {
            self.palette.selected = len - 1;
        }
    }

    pub(crate) fn handle_key_palette(
        &mut self,
        code: KeyCode,
        _modifiers: KeyModifiers,
    ) -> KeyOutcome {
        if self.palette.mode == PaletteMode::Command {
            match code {
                KeyCode::Esc => {
                    self.close_palette();
                    return KeyOutcome::None;
                }
                KeyCode::Up => {
                    self.palette.selected = self.palette.selected.saturating_sub(1);
                    return KeyOutcome::None;
                }
                KeyCode::Down => {
                    let len = self.palette_visible_items().len();
                    if len > 0 && self.palette.selected + 1 < len {
                        self.palette.selected += 1;
                    }
                    return KeyOutcome::None;
                }
                KeyCode::Enter => return self.execute_palette_selection(),
                _ => {
                    if let EditResult::Changed = self.palette.query.apply_edit(code) {
                        self.palette.selected = 0;
                    }
                    self.palette_clamp_selection();
                    return KeyOutcome::None;
                }
            }
        }

        // Menu mode: no query box — arrows pick a row, Enter runs it, Esc closes.
        match code {
            KeyCode::Esc | KeyCode::Char(';') => {
                self.close_palette();
                KeyOutcome::None
            }
            KeyCode::Up => {
                self.palette.selected = self.palette.selected.saturating_sub(1);
                KeyOutcome::None
            }
            KeyCode::Down => {
                let len = self.palette_visible_items().len();
                if len > 0 && self.palette.selected + 1 < len {
                    self.palette.selected += 1;
                }
                KeyOutcome::None
            }
            KeyCode::Enter => self.execute_palette_selection(),
            _ => KeyOutcome::None,
        }
    }

    fn execute_palette_selection(&mut self) -> KeyOutcome {
        let item = self
            .palette_visible_items()
            .get(self.palette.selected)
            .map(|i| (*i).clone());
        let Some(item) = item else {
            return KeyOutcome::None;
        };
        if !item.enabled {
            return KeyOutcome::None;
        }
        let exec = item.exec;
        let origin = self.palette.origin_screen;
        self.close_palette();
        self.screen = origin;
        match exec {
            PaletteExec::Key(code, modifiers) => self.handle_key_with(code, modifiers),
            PaletteExec::Cross(CrossAction::GoToGists) => {
                self.open_gist_manager();
                KeyOutcome::None
            }
            PaletteExec::Cross(CrossAction::GoToPins) => {
                self.open_pins();
                KeyOutcome::None
            }
            PaletteExec::Cross(CrossAction::OpenHelp) => {
                self.open_help();
                KeyOutcome::None
            }
            PaletteExec::Cross(CrossAction::ToggleTheme) => {
                self.theme_choice = match self.theme_choice {
                    crate::config::ThemeChoice::Dark => crate::config::ThemeChoice::Light,
                    crate::config::ThemeChoice::Light => crate::config::ThemeChoice::Dark,
                };
                self.theme = Theme::for_choice(self.theme_choice);
                KeyOutcome::ThemeToggle
            }
            PaletteExec::Cross(CrossAction::Quit) => KeyOutcome::Quit,
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
                self.palette.selected = i;
                return self.execute_palette_selection();
            }
        }
        KeyOutcome::None
    }
}

fn palette_item(key: &str, label: &str, exec: PaletteExec, enabled: bool) -> PaletteItem {
    PaletteItem {
        key_hint: key.to_string(),
        label: label.to_string(),
        exec,
        enabled,
        search: format!("{key} {label}").to_ascii_lowercase(),
    }
}

fn key_item(key: &str, label: &str, code: KeyCode, enabled: bool) -> PaletteItem {
    palette_item(
        key,
        label,
        PaletteExec::Key(code, KeyModifiers::NONE),
        enabled,
    )
}

fn cross_items() -> Vec<PaletteItem> {
    vec![
        palette_item(
            "g",
            "Go to Gists",
            PaletteExec::Cross(CrossAction::GoToGists),
            true,
        ),
        palette_item(
            "P",
            "Go to Pins",
            PaletteExec::Cross(CrossAction::GoToPins),
            true,
        ),
        palette_item(
            "?",
            "Go to Help",
            PaletteExec::Cross(CrossAction::OpenHelp),
            true,
        ),
        palette_item(
            "T",
            "Toggle theme",
            PaletteExec::Cross(CrossAction::ToggleTheme),
            true,
        ),
        palette_item("q", "Quit", PaletteExec::Cross(CrossAction::Quit), true),
    ]
}

fn build_palette_items(state: &AppState, screen: Screen, mode: PaletteMode) -> Vec<PaletteItem> {
    let mut items = match screen {
        Screen::List => list_palette_items(state),
        Screen::Pins => pins_palette_items(state),
        Screen::Gists => gists_palette_items(state),
        Screen::GistDetail => detail_palette_items(state),
        Screen::Revisions => revisions_palette_items(state),
        Screen::Diff => diff_palette_items(state),
        Screen::Preview => preview_palette_items(state),
        Screen::Help => help_palette_items(),
        Screen::Confirm | Screen::Palette => Vec::new(),
    };
    if mode == PaletteMode::Command {
        items.extend(cross_items());
    }
    items
}

fn list_palette_items(state: &AppState) -> Vec<PaletteItem> {
    // One dual-pane snapshot for all enablement checks (issue #224).
    let (visible_locals, ranked) = state.list_pane_snapshots();
    let has_gist = ranked.get(state.gist_index).is_some();
    let has_local = visible_locals.get(state.local_index).is_some();
    let gist = ranked.get(state.gist_index);
    let gist_id = gist.map(|g| g.file.gist_id.clone());
    let owned = gist_id
        .as_deref()
        .map(|id| state.gist_is_owned(id))
        .unwrap_or(false);
    let gist_file = gist.map(|g| g.file.clone());
    let previewable = gist_file
        .as_ref()
        .is_some_and(|f| state.gist_file_is_text_previewable(&f.gist_id, &f.filename));
    let diffable = gist_file.as_ref().is_some_and(|f| {
        let local_path = visible_locals
            .get(state.local_index)
            .map(|r| r.candidate.path.as_path());
        diff_pair_previewable(state, &f.gist_id, &f.filename, local_path)
    });
    let multi_file = gist_id
        .as_deref()
        .map(|id| state.gist_file_count(id) > 1)
        .unwrap_or(false);
    let pinned_pair =
        visible_locals
            .get(state.local_index)
            .zip(gist)
            .is_some_and(|(local, gist)| {
                state.pinned.iter().any(|m| {
                    m.local_path == local.candidate.path
                        && m.gist_id == gist.file.gist_id
                        && m.gist_filename == gist.file.filename
                })
            });

    vec![
        key_item("Enter", "Diff local ↔ gist", KeyCode::Enter, diffable),
        key_item(
            "Space",
            "Preview gist content",
            KeyCode::Char(' '),
            previewable,
        ),
        key_item(
            "d",
            "Download gist → cwd",
            KeyCode::Char('d'),
            has_gist && state.focus == FocusPane::Gist,
        ),
        key_item(
            "u",
            "Upload local → gist",
            KeyCode::Char('u'),
            has_local && has_gist && owned,
        ),
        key_item("n", "Create gist from local", KeyCode::Char('n'), has_local),
        key_item(
            "p",
            "Pin / unpin pair",
            KeyCode::Char('p'),
            has_local && has_gist,
        ),
        key_item("P", "Open Pins view", KeyCode::Char('P'), true),
        key_item(
            "g",
            "Open Gist manager",
            KeyCode::Char('g'),
            !state.gists.is_empty(),
        ),
        key_item(
            "S",
            "Smart-sync pinned pair",
            KeyCode::Char('S'),
            pinned_pair,
        ),
        key_item(
            "X",
            "Remove file from gist",
            KeyCode::Char('X'),
            has_gist && owned && multi_file,
        ),
        key_item("e", "Edit local file", KeyCode::Char('e'), has_local),
        key_item(
            "y",
            "Copy gist URL",
            KeyCode::Char('y'),
            state.context_gist_id().is_some(),
        ),
        key_item("H", "Revision history", KeyCode::Char('H'), has_gist),
        key_item(
            "*",
            "Star / unstar gist",
            KeyCode::Char('*'),
            state.context_gist_id().is_some(),
        ),
        key_item("r", "Toggle recursive scan", KeyCode::Char('r'), true),
        key_item("/", "Filter focused pane", KeyCode::Char('/'), true),
        key_item("Tab", "Switch pane", KeyCode::Tab, true),
        key_item("a", "Flip ranking anchor", KeyCode::Char('a'), true),
        key_item("t", "Toggle description / id", KeyCode::Char('t'), true),
        key_item("v", "Cycle gist visibility", KeyCode::Char('v'), true),
        key_item("s", "Cycle pane sort", KeyCode::Char('s'), true),
        key_item("?", "Help", KeyCode::Char('?'), true),
    ]
}

fn pins_palette_items(state: &AppState) -> Vec<PaletteItem> {
    let has_pin = !state.pinned.is_empty() && state.selected_pin_index().is_some();
    vec![
        key_item("Enter", "Diff pinned pair", KeyCode::Enter, has_pin),
        key_item("s", "Smart-sync", KeyCode::Char('s'), has_pin),
        key_item("u", "Force push", KeyCode::Char('u'), has_pin),
        key_item("d", "Force pull", KeyCode::Char('d'), has_pin),
        key_item("x", "Unpin pair", KeyCode::Char('x'), has_pin),
        key_item("/", "Filter pins", KeyCode::Char('/'), true),
        key_item("o", "Cycle sort", KeyCode::Char('o'), true),
        key_item("q", "Back to list", KeyCode::Char('q'), true),
        key_item("?", "Help", KeyCode::Char('?'), true),
    ]
}

fn gists_palette_items(state: &AppState) -> Vec<PaletteItem> {
    let groups = state.visible_gist_groups();
    let has_sel = state.gist_manager.index < groups.len();
    vec![
        key_item("Enter", "Open gist detail", KeyCode::Enter, has_sel),
        key_item("o", "Open in browser", KeyCode::Char('o'), has_sel),
        key_item("y", "Copy gist URL", KeyCode::Char('y'), has_sel),
        key_item("H", "Revision history", KeyCode::Char('H'), has_sel),
        key_item("*", "Star / unstar gist", KeyCode::Char('*'), has_sel),
        key_item("/", "Filter gists", KeyCode::Char('/'), true),
        key_item("s", "Cycle sort", KeyCode::Char('s'), true),
        key_item("v", "Cycle visibility", KeyCode::Char('v'), true),
        key_item("q", "Back to list", KeyCode::Char('q'), true),
        key_item("?", "Help", KeyCode::Char('?'), true),
    ]
}

fn detail_palette_items(state: &AppState) -> Vec<PaletteItem> {
    let gist_id = state.detail.gist_id.clone();
    let owned = gist_id
        .as_deref()
        .map(|id| state.gist_is_owned(id))
        .unwrap_or(false);
    let on_files = state.detail.focus == DetailFocus::Files;
    let file_count = gist_id
        .as_deref()
        .map(|id| state.gist_filenames(id).len())
        .unwrap_or(0);
    let has_file = on_files && state.detail.file_cursor < file_count;
    let previewable = gist_id.as_ref().is_some_and(|id| {
        state
            .gist_filenames(id)
            .into_iter()
            .nth(state.detail.file_cursor)
            .is_some_and(|name| state.gist_file_is_text_previewable(id, &name))
    }) && has_file;
    vec![
        key_item(
            "Enter",
            "Preview selected file",
            KeyCode::Enter,
            previewable,
        ),
        key_item(
            "o",
            "Open in browser",
            KeyCode::Char('o'),
            gist_id.is_some(),
        ),
        key_item("y", "Copy gist URL", KeyCode::Char('y'), gist_id.is_some()),
        key_item(
            "H",
            "Revision history",
            KeyCode::Char('H'),
            gist_id.is_some(),
        ),
        key_item("e", "Edit description", KeyCode::Char('e'), owned),
        key_item("c", "Compact revisions", KeyCode::Char('c'), owned),
        key_item(
            "*",
            "Star / unstar gist",
            KeyCode::Char('*'),
            gist_id.is_some(),
        ),
        key_item(
            "F",
            "Fork gist",
            KeyCode::Char('F'),
            gist_id.is_some() && !owned,
        ),
        key_item("X", "Delete gist", KeyCode::Char('X'), owned),
        key_item("Tab", "Switch Files / Comments", KeyCode::Tab, true),
        key_item(
            "m",
            "Load older comments",
            KeyCode::Char('m'),
            state.can_load_older_comments(),
        ),
        key_item("q", "Back to Gist manager", KeyCode::Char('q'), true),
        key_item("?", "Help", KeyCode::Char('?'), true),
    ]
}

fn revisions_palette_items(state: &AppState) -> Vec<PaletteItem> {
    let entries_len = state
        .revision
        .entries
        .as_ref()
        .map(|e| e.len())
        .unwrap_or(0);
    let has_entries = entries_len > 0;
    let not_head = state.revision.index > 0;
    let gist_id = state.revision.gist_id.clone();
    let owned = gist_id
        .as_deref()
        .map(|id| state.gist_is_owned(id))
        .unwrap_or(false);
    let file = state.revision.target_file.clone();
    let previewable = gist_id
        .as_ref()
        .is_some_and(|id| state.gist_file_is_text_previewable(id, &file));
    vec![
        key_item(
            "Enter",
            "Diff parent → revision",
            KeyCode::Enter,
            has_entries && previewable,
        ),
        key_item(
            "D",
            "Diff revision vs head",
            KeyCode::Char('D'),
            has_entries && not_head && previewable,
        ),
        key_item(
            "r",
            "Restore revision",
            KeyCode::Char('r'),
            entries_len > 1 && not_head && owned,
        ),
        key_item("F", "Cycle target file", KeyCode::Char('F'), has_entries),
        key_item("q", "Back", KeyCode::Char('q'), true),
        key_item("?", "Help", KeyCode::Char('?'), true),
    ]
}

fn diff_palette_items(state: &AppState) -> Vec<PaletteItem> {
    let sync = state.diff_allows_sync() && !state.diff_identical;
    vec![
        key_item("d", "Download", KeyCode::Char('d'), sync),
        key_item("u", "Upload", KeyCode::Char('u'), sync),
        key_item("c", "Toggle full diff context", KeyCode::Char('c'), true),
        key_item("w", "Toggle line wrap", KeyCode::Char('w'), true),
        key_item("q", "Back", KeyCode::Char('q'), true),
    ]
}

fn preview_palette_items(_state: &AppState) -> Vec<PaletteItem> {
    vec![
        key_item("R", "Refresh content", KeyCode::Char('R'), true),
        key_item("w", "Toggle line wrap", KeyCode::Char('w'), true),
        key_item("y", "Copy gist URL", KeyCode::Char('y'), true),
        key_item("Y", "Copy file content", KeyCode::Char('Y'), true),
        key_item("q", "Back", KeyCode::Char('q'), true),
    ]
}

fn help_palette_items() -> Vec<PaletteItem> {
    vec![
        key_item("Tab", "Browse topic index", KeyCode::Tab, true),
        key_item("q", "Close Help", KeyCode::Char('q'), true),
    ]
}

fn diff_pair_previewable(
    state: &AppState,
    gist_id: &str,
    filename: &str,
    local_path: Option<&std::path::Path>,
) -> bool {
    if !state.gist_file_is_text_previewable(gist_id, filename) {
        return false;
    }
    if let Some(path) = local_path {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if !crate::domain::gist_file_is_text_previewable(name, None) {
                return false;
            }
        }
    }
    true
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

    #[test]
    fn fuzzy_match_empty_query_matches_all() {
        assert_eq!(fuzzy_match("", "download gist"), Some(0));
    }

    #[test]
    fn fuzzy_match_subsequence() {
        assert!(fuzzy_match("dl", "d download gist").is_some());
        assert!(fuzzy_match("xyz", "download gist").is_none());
    }
}
