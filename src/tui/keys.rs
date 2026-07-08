use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

/// Vim-style navigation alias alongside arrow / page keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NavAction {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
}

fn nav_action(code: KeyCode, modifiers: KeyModifiers) -> Option<NavAction> {
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    match code {
        KeyCode::Up if !ctrl => Some(NavAction::Up),
        KeyCode::Char('k') if !ctrl => Some(NavAction::Up),
        KeyCode::Down if !ctrl => Some(NavAction::Down),
        KeyCode::Char('j') if !ctrl => Some(NavAction::Down),
        KeyCode::Left if !ctrl => Some(NavAction::Left),
        KeyCode::Char('h') if !ctrl => Some(NavAction::Left),
        KeyCode::Right if !ctrl => Some(NavAction::Right),
        KeyCode::Char('l') if !ctrl => Some(NavAction::Right),
        KeyCode::PageUp if !ctrl => Some(NavAction::PageUp),
        KeyCode::Char('b') if ctrl => Some(NavAction::PageUp),
        KeyCode::PageDown if !ctrl => Some(NavAction::PageDown),
        KeyCode::Char('f') if ctrl => Some(NavAction::PageDown),
        _ => None,
    }
}

/// Lines moved per PageUp/PageDown in the scrollable views. Matches the gist-detail paging step
/// (`detail_nav(10)`); `handle_key` is pure and cannot read the viewport height, so a fixed step
/// keeps paging predictable without threading terminal size into the key logic.
const PAGE_SCROLL: u16 = 10;

/// `T` theme toggle accepts a plain capital key (Caps Lock) or Shift+T; reject Ctrl/Alt combos.
fn theme_toggle_modifiers_ok(modifiers: KeyModifiers) -> bool {
    modifiers.is_empty() || modifiers == KeyModifiers::SHIFT
}

impl AppState {
    pub fn handle_key(&mut self, code: KeyCode) -> KeyOutcome {
        self.handle_key_with(code, KeyModifiers::NONE)
    }

    pub fn handle_key_with(&mut self, code: KeyCode, modifiers: KeyModifiers) -> KeyOutcome {
        // Global theme toggle: skip while any inline text input is active so `T` can still
        // be typed into filters and description editors.
        if code == KeyCode::Char('T')
            && theme_toggle_modifiers_ok(modifiers)
            && !self.filtering
            && !self.pins.filtering
            && !self.gist_manager.filtering
            && !self.editing_description
        {
            self.theme_choice = match self.theme_choice {
                crate::config::ThemeChoice::Dark => crate::config::ThemeChoice::Light,
                crate::config::ThemeChoice::Light => crate::config::ThemeChoice::Dark,
            };
            self.theme = Theme::for_choice(self.theme_choice);
            return KeyOutcome::ThemeToggle;
        }
        if let Some(action) = nav_action(code, modifiers) {
            if self.apply_navigation(action) {
                self.dismiss_ephemeral_screen_state();
                return KeyOutcome::None;
            }
        }
        match self.screen {
            Screen::List if self.filtering => self.handle_key_filter(code),
            Screen::List => self.handle_key_list(code),
            Screen::Diff => self.handle_key_diff(code),
            Screen::Confirm => self.handle_key_confirm(code),
            Screen::Preview => self.handle_key_preview(code),
            Screen::Help => self.handle_key_help(code),
            Screen::Pins => self.handle_key_pins(code),
            Screen::Gists => self.handle_key_gists(code),
            Screen::GistDetail => self.handle_key_detail(code),
            Screen::Revisions => self.handle_key_revisions(code),
        }
    }

    /// Open the Help screen on the topic for the current screen, remembering where to return.
    /// A no-op while already on Help — otherwise the top bar's `(?)Help` click (reachable from
    /// any screen, including Help itself) would overwrite `return_screen` with `Screen::Help`,
    /// trapping Esc/`?`/the close button in Help with no keyboard way out.
    fn open_help(&mut self) {
        if self.screen == Screen::Help {
            return;
        }
        self.help.return_screen = self.screen;
        self.help.topic = HelpTopic::for_screen(self.screen);
        self.help.index_open = false;
        self.help.scroll = 0;
        self.screen = Screen::Help;
    }

    /// Arrow / hjkl / Ctrl+b/f navigation. Returns true when the key was consumed.
    /// Filter and text-input modes keep `hjkl` as typed characters (arrows still move
    /// selection while filtering — handled in the filter branches).
    fn apply_navigation(&mut self, action: NavAction) -> bool {
        if self.editing_description {
            return false;
        }
        // While filtering, arrows/hjkl are typed or handled in the filter branches; page keys
        // still jump the live selection by PAGE_SCROLL.
        if (self.filtering || self.pins.filtering || self.gist_manager.filtering)
            && !matches!(action, NavAction::PageUp | NavAction::PageDown)
        {
            return false;
        }
        match self.screen {
            Screen::Help => {
                let topics = HelpTopic::all();
                if self.help.index_open {
                    match action {
                        NavAction::Up => {
                            self.help.index_sel = self.help.index_sel.saturating_sub(1)
                        }
                        NavAction::Down => {
                            if self.help.index_sel + 1 < topics.len() {
                                self.help.index_sel += 1;
                            }
                        }
                        _ => return false,
                    }
                } else {
                    match action {
                        NavAction::Up => {
                            self.help.scroll = self.help.scroll.saturating_sub(1);
                        }
                        NavAction::Down => {
                            self.help.scroll = self.help.scroll.saturating_add(1);
                        }
                        NavAction::PageUp => {
                            self.help.scroll = self.help.scroll.saturating_sub(PAGE_SCROLL);
                        }
                        NavAction::PageDown => {
                            self.help.scroll = self.help.scroll.saturating_add(PAGE_SCROLL);
                        }
                        _ => return false,
                    }
                }
                true
            }
            Screen::Pins => {
                let len = self.visible_pin_indices().len();
                match action {
                    NavAction::Down => {
                        if self.pins.index + 1 < len {
                            self.pins.index += 1;
                            self.pins.hscroll = 0;
                        }
                    }
                    NavAction::Up => {
                        if self.pins.index > 0 {
                            self.pins.index -= 1;
                            self.pins.hscroll = 0;
                        }
                    }
                    NavAction::Right => {
                        self.pins.hscroll = (self.pins.hscroll + 1).min(self.pins_hscroll_max());
                    }
                    NavAction::Left => {
                        self.pins.hscroll = self.pins.hscroll.saturating_sub(1);
                    }
                    NavAction::PageDown => {
                        if len > 0 {
                            let max = len - 1;
                            self.pins.index = (self.pins.index + PAGE_SCROLL as usize).min(max);
                            self.pins.hscroll = 0;
                        }
                    }
                    NavAction::PageUp => {
                        self.pins.index = self.pins.index.saturating_sub(PAGE_SCROLL as usize);
                        self.pins.hscroll = 0;
                    }
                }
                true
            }
            Screen::Gists => {
                let groups = self.visible_gist_groups();
                match action {
                    NavAction::Down => {
                        if self.gist_manager.index + 1 < groups.len() {
                            self.gist_manager.index += 1;
                            self.gist_manager.hscroll = 0;
                        }
                    }
                    NavAction::Up => {
                        if self.gist_manager.index > 0 {
                            self.gist_manager.index -= 1;
                            self.gist_manager.hscroll = 0;
                        }
                    }
                    NavAction::Right => {
                        self.gist_manager.hscroll =
                            (self.gist_manager.hscroll + 1).min(self.gists_hscroll_max());
                    }
                    NavAction::Left => {
                        self.gist_manager.hscroll = self.gist_manager.hscroll.saturating_sub(1);
                    }
                    NavAction::PageDown => {
                        let len = groups.len();
                        if len > 0 {
                            let max = len - 1;
                            self.gist_manager.index =
                                (self.gist_manager.index + PAGE_SCROLL as usize).min(max);
                            self.gist_manager.hscroll = 0;
                        }
                    }
                    NavAction::PageUp => {
                        self.gist_manager.index =
                            self.gist_manager.index.saturating_sub(PAGE_SCROLL as usize);
                        self.gist_manager.hscroll = 0;
                    }
                }
                true
            }
            Screen::GistDetail => {
                match action {
                    NavAction::Down => self.detail_nav(1),
                    NavAction::Up => self.detail_nav(-1),
                    NavAction::PageDown => self.detail_nav(10),
                    NavAction::PageUp => self.detail_nav(-10),
                    _ => return false,
                }
                true
            }
            Screen::Revisions => {
                let entries_len = self.revision.entries.as_ref().map(|e| e.len()).unwrap_or(0);
                if entries_len == 0 {
                    return false;
                }
                match action {
                    NavAction::Down => {
                        self.revision.index = (self.revision.index + 1).min(entries_len - 1);
                    }
                    NavAction::Up => {
                        self.revision.index = self.revision.index.saturating_sub(1);
                    }
                    NavAction::PageDown => {
                        self.revision.index =
                            (self.revision.index + PAGE_SCROLL as usize).min(entries_len - 1);
                    }
                    NavAction::PageUp => {
                        self.revision.index =
                            self.revision.index.saturating_sub(PAGE_SCROLL as usize);
                    }
                    NavAction::Left => {
                        self.revision.hscroll = self.revision.hscroll.saturating_sub(1);
                    }
                    NavAction::Right => {
                        self.revision.hscroll = self.revision.hscroll.saturating_add(1);
                    }
                }
                true
            }
            Screen::List => {
                match action {
                    NavAction::Down => self.list_move_focused(true),
                    NavAction::Up => self.list_move_focused(false),
                    NavAction::PageDown => self.list_page_focused(true),
                    NavAction::PageUp => self.list_page_focused(false),
                    NavAction::Left => self.scroll_focused_left(),
                    NavAction::Right => self.scroll_focused_right(),
                }
                true
            }
            // Diff, Preview and Confirm all scroll the same diff/preview buffer identically.
            Screen::Diff | Screen::Preview | Screen::Confirm => {
                match action {
                    NavAction::Down => self.scroll_diff_down(),
                    NavAction::Up => self.scroll_diff_up(),
                    NavAction::PageDown => self.scroll_diff_page_down(PAGE_SCROLL),
                    NavAction::PageUp => self.scroll_diff_page_up(PAGE_SCROLL),
                    NavAction::Right => self.scroll_diff_right(),
                    NavAction::Left => self.scroll_diff_left(),
                }
                true
            }
        }
    }

    /// Screens that clear a one-shot status (and the list quit arm) on any key — including
    /// navigation keys handled before the per-screen handler runs.
    fn dismiss_ephemeral_screen_state(&mut self) {
        match self.screen {
            Screen::List => {
                self.status = None;
                self.quit_armed = false;
            }
            Screen::Pins
            | Screen::Gists
            | Screen::GistDetail
            | Screen::Revisions
            | Screen::Preview => self.status = None,
            _ => {}
        }
    }

    fn handle_key_help(&mut self, code: KeyCode) -> KeyOutcome {
        let topics = HelpTopic::all();
        if self.help.index_open {
            match code {
                KeyCode::Enter => {
                    self.help.topic = topics[self.help.index_sel];
                    self.help.index_open = false;
                    self.help.scroll = 0;
                }
                KeyCode::Char(c @ '1'..='9') if (c as u8 - b'1') < topics.len() as u8 => {
                    self.help.topic = topics[(c as u8 - b'1') as usize];
                    self.help.index_open = false;
                    self.help.scroll = 0;
                }
                KeyCode::Char('0') if topics.len() > 9 => {
                    self.help.topic = topics[9];
                    self.help.index_open = false;
                    self.help.scroll = 0;
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.screen = self.help.return_screen;
                    self.help.index_open = false;
                    self.help.scroll = 0;
                }
                _ => {}
            }
        } else {
            match code {
                KeyCode::Tab => {
                    self.help.index_sel = topics
                        .iter()
                        .position(|&t| t == self.help.topic)
                        .unwrap_or(0);
                    self.help.index_open = true;
                }
                KeyCode::Char(c @ '1'..='9') if (c as u8 - b'1') < topics.len() as u8 => {
                    self.help.topic = topics[(c as u8 - b'1') as usize];
                    self.help.scroll = 0;
                }
                KeyCode::Char('0') if topics.len() > 9 => {
                    self.help.topic = topics[9];
                    self.help.scroll = 0;
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.screen = self.help.return_screen;
                    self.help.scroll = 0;
                }
                _ => {}
            }
        }
        KeyOutcome::None
    }

    fn handle_key_pins(&mut self, code: KeyCode) -> KeyOutcome {
        // One-shot: any key dismisses a lingering sync status; the run_loop IO helper for this
        // key may set a fresh one afterwards (e.g. "already in sync").
        self.status = None;
        // Inline text filter: live-navigate with arrows; Tab is a no-op (single pane).
        if self.pins.filtering {
            match code {
                KeyCode::Up if self.pins.index > 0 => {
                    self.pins.index -= 1;
                    self.pins.hscroll = 0;
                }
                KeyCode::Up => {}
                KeyCode::Down => {
                    if self.pins.index + 1 < self.visible_pin_indices().len() {
                        self.pins.index += 1;
                        self.pins.hscroll = 0;
                    }
                }
                _ => match apply_filter_edit(code, &mut self.pins.filter_query) {
                    FilterKey::Edited => {
                        self.pins.index = 0;
                        self.pins.hscroll = 0;
                    }
                    FilterKey::Cleared => {
                        self.pins.filtering = false;
                        self.pins.index = 0;
                        self.pins.hscroll = 0;
                    }
                    FilterKey::Exited => self.pins.filtering = false,
                    FilterKey::Moved | FilterKey::Pass => {}
                },
            }
            return KeyOutcome::None;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.screen = Screen::List,
            KeyCode::Char('/') => self.pins.filtering = true,
            KeyCode::Enter if !self.pinned.is_empty() => {
                if let Some(idx) = self.selected_pin_index() {
                    let (gist_id, gist_filename, local_path) = {
                        let pin = &self.pinned[idx];
                        (
                            pin.gist_id.clone(),
                            pin.gist_filename.clone(),
                            pin.local_path.clone(),
                        )
                    };
                    if self.block_if_non_previewable_diff(
                        &gist_id,
                        &gist_filename,
                        Some(local_path.as_path()),
                    ) {
                        return KeyOutcome::None;
                    }
                }
                return KeyOutcome::PreviewPinDiff;
            }
            KeyCode::Char('x') if !self.pinned.is_empty() => return KeyOutcome::UnpinAtPin,
            KeyCode::Char('s') if !self.pinned.is_empty() => return KeyOutcome::SyncPinAuto,
            KeyCode::Char('u') if !self.pinned.is_empty() => return KeyOutcome::SyncPinPush,
            KeyCode::Char('d') if !self.pinned.is_empty() => return KeyOutcome::SyncPinPull,
            KeyCode::Char('o') => {
                self.pins.sort = self.pins.sort.next();
                self.pins.index = 0;
                self.pins.hscroll = 0;
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }

    fn handle_key_gists(&mut self, code: KeyCode) -> KeyOutcome {
        self.status = None;
        // Inline text filter: live-navigate with arrows; Tab is a no-op (single pane).
        if self.gist_manager.filtering {
            match code {
                KeyCode::Up if self.gist_manager.index > 0 => {
                    self.gist_manager.index -= 1;
                    self.gist_manager.hscroll = 0;
                }
                KeyCode::Up => {}
                KeyCode::Down => {
                    if self.gist_manager.index + 1 < self.visible_gist_groups().len() {
                        self.gist_manager.index += 1;
                        self.gist_manager.hscroll = 0;
                    }
                }
                _ => match apply_filter_edit(code, &mut self.gist_manager.filter_query) {
                    FilterKey::Edited => {
                        self.gist_manager.index = 0;
                        self.gist_manager.hscroll = 0;
                    }
                    FilterKey::Cleared => {
                        self.gist_manager.filtering = false;
                        self.gist_manager.index = 0;
                        self.gist_manager.hscroll = 0;
                    }
                    FilterKey::Exited => self.gist_manager.filtering = false,
                    FilterKey::Moved | FilterKey::Pass => {}
                },
            }
            return KeyOutcome::None;
        }
        let groups = self.visible_gist_groups();
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.screen = Screen::List,
            KeyCode::Char('/') => self.gist_manager.filtering = true,
            KeyCode::Char('s') => {
                self.gist_manager.sort = self.gist_manager.sort.next();
                self.gist_manager.index = 0;
                self.gist_manager.hscroll = 0;
            }
            KeyCode::Char('v') => {
                self.gist_manager.type_filter = self.gist_manager.type_filter.next();
                self.gist_manager.index = 0;
                self.gist_manager.hscroll = 0;
            }
            KeyCode::Char('*') => return self.star_toggle_intent(),
            KeyCode::Enter if self.gist_manager.index < groups.len() => {
                return KeyOutcome::OpenGistDetail;
            }
            KeyCode::Char('o') if self.gist_manager.index < groups.len() => {
                return KeyOutcome::OpenBrowser
            }
            KeyCode::Char('y') if self.gist_manager.index < groups.len() => {
                return KeyOutcome::CopyGistUrl
            }
            KeyCode::Char('H') if self.gist_manager.index < groups.len() => {
                if self.open_revisions(Screen::Gists) {
                    return KeyOutcome::FetchRevisions;
                }
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }

    /// Pure key handling for `Screen::GistDetail`: scroll comments, compact, browser, back.
    fn handle_key_detail(&mut self, code: KeyCode) -> KeyOutcome {
        self.status = None;
        if self.editing_description {
            match code {
                KeyCode::Esc => {
                    self.editing_description = false;
                    self.description_input.clear();
                }
                KeyCode::Enter => return KeyOutcome::ApplyDescription,
                _ => {
                    self.description_input.apply_edit(code);
                }
            }
            return KeyOutcome::None;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.screen = Screen::Gists;
            }
            KeyCode::Char('o') => return KeyOutcome::OpenBrowser,
            KeyCode::Char('y') => return KeyOutcome::CopyGistUrl,
            KeyCode::Char('H') => {
                if self.open_revisions(Screen::GistDetail) {
                    return KeyOutcome::FetchRevisions;
                }
            }
            KeyCode::Char('e') => {
                let Some(id) = self.detail.gist_id.clone() else {
                    return KeyOutcome::None;
                };
                if !self.gist_is_owned(&id) {
                    return KeyOutcome::None;
                }
                if let Some(group) = self.group_by_id(&id) {
                    self.editing_description = true;
                    self.description_input.set(group.description.clone());
                }
            }
            KeyCode::Char('c') => {
                let Some(id) = self.detail.gist_id.clone() else {
                    return KeyOutcome::None;
                };
                if !self.gist_is_owned(&id) {
                    return KeyOutcome::None;
                }
                self.detail.compact_return_screen = Screen::GistDetail;
                return KeyOutcome::CompactGist;
            }
            KeyCode::Char('*') => return self.star_toggle_intent(),
            KeyCode::Char('F') => return self.fork_intent(),
            // 1–9 preview the content of the Nth file in the gist (full-screen preview).
            KeyCode::Char(c @ '1'..='9') => {
                return self.preview_detail_file((c as u8 - b'1') as usize);
            }
            KeyCode::Tab => {
                self.detail.focus = match self.detail.focus {
                    DetailFocus::Comments => DetailFocus::Files,
                    DetailFocus::Files => DetailFocus::Comments,
                };
                if self.detail.focus == DetailFocus::Comments
                    && self.detail.comments.is_none()
                    && !self.detail.comments_loading
                {
                    return KeyOutcome::FetchComments;
                }
            }
            // X deletes the whole gist (y/n confirm). Reuses the shared Delete confirm path,
            // which lands on the list once the gist is gone. Owned gists only (no-op otherwise).
            KeyCode::Char('X') => {
                if let Some(group) = self
                    .detail
                    .gist_id
                    .clone()
                    .filter(|id| self.gist_is_owned(id))
                    .and_then(|id| self.group_by_id(&id))
                {
                    let label = if group.description.is_empty() {
                        group.id.clone()
                    } else {
                        group.description.clone()
                    };
                    self.diff_text = format!(
                        "Delete gist {} ({} file(s)): {label}.\n\nThis permanently removes the entire gist and all its files.",
                        group.id, group.file_count
                    );
                    self.diff_scroll = 0;
                    self.diff_hscroll = 0;
                    self.pending_action = Some(PendingAction::Delete {
                        gist_id: group.id.clone(),
                        label,
                    });
                    self.screen = Screen::Confirm;
                }
            }
            KeyCode::Enter if self.detail.focus == DetailFocus::Files => {
                if let Some(gist_id) = self.detail.gist_id.clone() {
                    let cursor = self.detail.file_cursor;
                    if let Some(filename) = self.gist_filenames(&gist_id).into_iter().nth(cursor) {
                        if self.block_if_non_previewable_gist_file(&gist_id, &filename) {
                            return KeyOutcome::None;
                        }
                        self.preview_request = Some((gist_id, filename));
                        self.preview_return = Screen::GistDetail;
                        return KeyOutcome::PreviewContent;
                    }
                }
            }
            KeyCode::Char('m') if self.detail.focus == DetailFocus::Comments => {
                if self.can_load_older_comments() {
                    return KeyOutcome::LoadOlderComments;
                }
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }

    fn handle_key_revisions(&mut self, code: KeyCode) -> KeyOutcome {
        self.status = None;
        let entries_len = self.revision.entries.as_ref().map(|e| e.len()).unwrap_or(0);
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.screen = self.revision.return_screen;
            }
            KeyCode::Enter if entries_len > 0 => {
                if let (Some(id), file) = (
                    self.revision.gist_id.clone(),
                    self.revision.target_file.clone(),
                ) {
                    if self.block_if_non_previewable_gist_file(&id, &file) {
                        return KeyOutcome::None;
                    }
                }
                return KeyOutcome::RevisionDiffIncremental;
            }
            KeyCode::Char('D') if entries_len > 0 && self.revision.index > 0 => {
                if let (Some(id), file) = (
                    self.revision.gist_id.clone(),
                    self.revision.target_file.clone(),
                ) {
                    if self.block_if_non_previewable_gist_file(&id, &file) {
                        return KeyOutcome::None;
                    }
                }
                return KeyOutcome::RevisionDiff;
            }
            KeyCode::Char('D') if entries_len > 0 => {
                self.set_status("already at current revision");
            }
            KeyCode::Char('r') if entries_len > 1 && self.revision.index > 0 => {
                if let Some(id) = self.revision.gist_id.clone() {
                    if !self.gist_is_owned(&id) {
                        return KeyOutcome::None;
                    }
                }
                return KeyOutcome::RestoreRevisionPreview;
            }
            KeyCode::Char('r') if entries_len <= 1 => {
                self.set_status("only one revision — nothing to restore");
            }
            KeyCode::Char('r') if self.revision.index == 0 => {
                self.set_status("already at current revision");
            }
            KeyCode::Char('F') if !self.cycle_revision_target_file() => {
                self.set_status("only one file in this gist");
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }

    /// Move within the focused detail pane: scroll comments, or move the file cursor
    /// (clamped to the gist's file count). `delta` is signed rows.
    fn detail_nav(&mut self, delta: i32) {
        match self.detail.focus {
            DetailFocus::Comments => {
                self.detail.scroll = if delta < 0 {
                    self.detail.scroll.saturating_sub((-delta) as u16)
                } else {
                    self.detail.scroll.saturating_add(delta as u16)
                };
            }
            DetailFocus::Files => {
                let count = self
                    .detail
                    .gist_id
                    .as_deref()
                    .map(|id| self.gist_filenames(id).len())
                    .unwrap_or(0);
                if count == 0 {
                    return;
                }
                let max = count - 1;
                let next = self.detail.file_cursor as i64 + delta as i64;
                self.detail.file_cursor = next.clamp(0, max as i64) as usize;
            }
        }
    }

    /// Number of navigation steps per mouse wheel tick. List/index screens move one row;
    /// content panes (Diff, Preview, Confirm, GistDetail) scroll three lines for faster
    /// panning. Help body also scrolls three; the Help topic index is a list (one row).
    fn wheel_step(&self) -> usize {
        match self.screen {
            Screen::Diff | Screen::Preview | Screen::Confirm => 3,
            // GistDetail: the comments body scrolls like content (3 lines); the file list
            // steps one file at a time.
            Screen::GistDetail if self.detail.focus == DetailFocus::Comments => 3,
            Screen::Help if !self.help.index_open => 3, // help body scrolls; topic index is a list
            _ => 1, // List/Pins/Gists/Revisions/Help index/GistDetail Files
        }
    }

    /// Select the clicked list row on the current screen, focusing its pane/list. Returns
    /// `true` when a row was hit (so a double-click should "open" it). A click in a pane's
    /// blank area or border focuses it but selects nothing (returns `false`); a click off
    /// every list returns `false`.
    fn click_select(&mut self, col: u16, row: u16, layout: &MouseLayout) -> bool {
        match self.screen {
            Screen::List => {
                if let Some(hit) = layout.local {
                    if point_in(hit.rect, col, row) {
                        // A click anywhere in the pane (incl. blank/border) focuses it; a
                        // click on a row also selects it.
                        self.focus = FocusPane::Local;
                        if let Some(idx) = hit.index_at(row, self.visible_locals().len()) {
                            self.local_index = idx;
                            self.local_hscroll = 0;
                            if self.anchor == FocusPane::Local {
                                self.reset_ranked_pane();
                            }
                            return true;
                        }
                        return false;
                    }
                }
                if let Some(hit) = layout.gist {
                    if point_in(hit.rect, col, row) {
                        self.focus = FocusPane::Gist;
                        if let Some(idx) = hit.index_at(row, self.ranked_gists().len()) {
                            self.gist_index = idx;
                            self.gist_hscroll = 0;
                            if self.anchor == FocusPane::Gist {
                                self.reset_ranked_pane();
                            }
                            return true;
                        }
                        return false;
                    }
                }
                false
            }
            Screen::Gists => {
                if let Some(hit) = layout.list {
                    if point_in(hit.rect, col, row) {
                        if let Some(idx) = hit.index_at(row, self.visible_gist_groups().len()) {
                            self.gist_manager.index = idx;
                            self.gist_manager.hscroll = 0;
                            return true;
                        }
                    }
                }
                false
            }
            Screen::Pins => {
                if let Some(hit) = layout.list {
                    if point_in(hit.rect, col, row) {
                        if let Some(idx) = hit.index_at(row, self.visible_pin_indices().len()) {
                            self.pins.index = idx;
                            self.pins.hscroll = 0;
                            return true;
                        }
                    }
                }
                false
            }
            Screen::Revisions => {
                if let Some(hit) = layout.list {
                    if point_in(hit.rect, col, row) {
                        let count = self.revision.entries.as_ref().map_or(0, |e| e.len());
                        if let Some(idx) = hit.index_at(row, count) {
                            self.revision.index = idx;
                            self.revision.hscroll = 0;
                            return true;
                        }
                    }
                }
                false
            }
            Screen::GistDetail => {
                if let Some(hit) = layout.detail_files {
                    if point_in(hit.rect, col, row) {
                        // Clicking the file list focuses the Files tab; a row also moves the cursor.
                        self.detail.focus = DetailFocus::Files;
                        let count = self
                            .detail
                            .gist_id
                            .as_deref()
                            .map_or(0, |id| self.gist_filenames(id).len());
                        if let Some(idx) = hit.index_at(row, count) {
                            self.detail.file_cursor = idx;
                            return true;
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Open/activate the currently selected row on the current screen (the double-click
    /// action), reusing each screen's `Enter` behaviour where one exists.
    fn activate_selected(&mut self) -> KeyOutcome {
        match self.screen {
            // GistDetail files have no `Enter`; they preview via number keys, so a
            // double-click previews the file under the cursor.
            Screen::GistDetail => self.preview_detail_file(self.detail.file_cursor),
            _ => self.handle_key_with(KeyCode::Enter, KeyModifiers::NONE),
        }
    }

    /// Preview the `index`-th file of the gist shown on `Screen::GistDetail` (full-screen),
    /// the action behind the `1`–`9` keys and a file double-click.
    fn preview_detail_file(&mut self, index: usize) -> KeyOutcome {
        if let Some(gist_id) = self.detail.gist_id.clone() {
            if let Some(filename) = self.gist_filenames(&gist_id).into_iter().nth(index) {
                if self.block_if_non_previewable_gist_file(&gist_id, &filename) {
                    return KeyOutcome::None;
                }
                self.preview_request = Some((gist_id, filename));
                self.preview_return = Screen::GistDetail;
                return KeyOutcome::PreviewContent;
            }
        }
        KeyOutcome::None
    }

    /// Switch the GistDetail tab if `col`/`row` lands on a tab header. Returns the outcome
    /// (possibly `FetchComments`) when a tab was clicked, else `None` to fall through.
    fn click_detail_tab(&mut self, col: u16, row: u16, layout: &MouseLayout) -> Option<KeyOutcome> {
        if self.screen != Screen::GistDetail {
            return None;
        }
        if let Some(rect) = layout.detail_tab_files {
            if point_in(rect, col, row) {
                self.detail.focus = DetailFocus::Files;
                return Some(KeyOutcome::None);
            }
        }
        if let Some(rect) = layout.detail_tab_comments {
            if point_in(rect, col, row) {
                self.detail.focus = DetailFocus::Comments;
                if self.detail.comments.is_none() && !self.detail.comments_loading {
                    return Some(KeyOutcome::FetchComments);
                }
                return Some(KeyOutcome::None);
            }
        }
        None
    }

    /// A click on the GistDetail "load older comments" affordance line.
    fn click_comments_load_older(
        &mut self,
        col: u16,
        row: u16,
        layout: &MouseLayout,
    ) -> Option<KeyOutcome> {
        if self.screen != Screen::GistDetail || self.detail.focus != DetailFocus::Comments {
            return None;
        }
        let rect = layout.comments_load_older?;
        if point_in(rect, col, row) && self.can_load_older_comments() {
            return Some(KeyOutcome::LoadOlderComments);
        }
        None
    }

    /// Translate a classified mouse intent into a state change, reusing existing keyboard
    /// logic. Pure (no IO, no clock); returns a `KeyOutcome` so `run_loop` can perform any
    /// follow-up IO (e.g. `PreviewDiff` on double-click).
    pub fn handle_mouse(&mut self, input: MouseInput, layout: &MouseLayout) -> KeyOutcome {
        match input {
            MouseInput::ScrollUp | MouseInput::ScrollDown => {
                let action = if matches!(input, MouseInput::ScrollUp) {
                    NavAction::Up
                } else {
                    NavAction::Down
                };
                for _ in 0..self.wheel_step() {
                    self.apply_navigation(action);
                }
                KeyOutcome::None
            }
            MouseInput::Click { col, row } => {
                // Close button takes priority on non-List screens.
                if let Some(rect) = layout.close_button {
                    if point_in(rect, col, row) {
                        // Esc is the universal cancel across all screens and all
                        // pending-action variants (including the create-description
                        // editing sub-state where 'n' would type into the field).
                        return self.handle_key_with(KeyCode::Esc, KeyModifiers::NONE);
                    }
                }
                // GitHub repo link click opens it in the browser.
                if let Some(rect) = layout.repo_link {
                    if point_in(rect, col, row) {
                        return KeyOutcome::OpenRepoUrl;
                    }
                }
                // Top-bar (G)ists / (P)ins / (?)Help — same effect as pressing the key,
                // from any screen (not just wherever that key happens to be bound).
                if let Some(rect) = layout.top_bar_gists {
                    if point_in(rect, col, row) {
                        self.open_gist_manager();
                        return KeyOutcome::None;
                    }
                }
                if let Some(rect) = layout.top_bar_pins {
                    if point_in(rect, col, row) {
                        self.open_pins();
                        return KeyOutcome::None;
                    }
                }
                if let Some(rect) = layout.top_bar_help {
                    if point_in(rect, col, row) {
                        self.open_help();
                        return KeyOutcome::None;
                    }
                }
                // A GistDetail tab header click switches focus (single-click action).
                if let Some(outcome) = self.click_detail_tab(col, row, layout) {
                    return outcome;
                }
                if let Some(outcome) = self.click_comments_load_older(col, row, layout) {
                    return outcome;
                }
                self.click_select(col, row, layout);
                KeyOutcome::None
            }
            MouseInput::DoubleClick { col, row } => {
                // A tab double-click is just a tab switch (no "open").
                if let Some(outcome) = self.click_detail_tab(col, row, layout) {
                    return outcome;
                }
                if self.click_select(col, row, layout) {
                    // Selection landed on a row — open/activate it.
                    return self.activate_selected();
                }
                KeyOutcome::None
            }
        }
    }

    fn handle_key_preview(&mut self, code: KeyCode) -> KeyOutcome {
        // One-shot: any key dismisses a lingering status (e.g. a previous "fetch failed: …"); the
        // run_loop refresh helper may set a fresh one afterwards.
        self.status = None;
        match code {
            // In the preview, q and Esc return to wherever it was launched from (the list, or
            // the gist detail view) — never an accidental app exit. Reset to List afterwards.
            KeyCode::Char('q') | KeyCode::Esc => {
                self.screen = self.preview_return;
                self.preview_return = Screen::List;
                self.diff_text.clear();
                self.preview_title.clear();
                self.preview_gist_key = None;
            }
            KeyCode::Char('R') => return KeyOutcome::RefreshPreview,
            KeyCode::Char('w') => self.preview_wrap = !self.preview_wrap,
            KeyCode::Char('y') => return KeyOutcome::CopyGistUrl,
            KeyCode::Char('Y') => return KeyOutcome::CopyPreviewContent,
            _ => {}
        }
        KeyOutcome::None
    }
}

/// Outcome of applying one key to a filter query's text (the shared edit transitions
/// for every inline filter input). Nav keys (Up/Down) and Tab are handled by the caller.
enum FilterKey {
    /// Query text changed (char appended or backspace popped a char); caller resets
    /// the affected pane's selection index + horizontal scroll.
    Edited,
    /// Leave filter input, keeping the current query (Enter, or Backspace on empty).
    Exited,
    /// Esc: query cleared; caller leaves input and resets index + scroll.
    Cleared,
    /// Only the cursor moved (←/→/Home/End): caller stays in input, no re-rank.
    Moved,
    /// Not a text-edit key (e.g. Up/Down or Tab the caller already handled); ignore.
    Pass,
}

/// Apply one key to `query` and report the transition. Pure: only mutates `query`.
/// Text editing (insert/delete/cursor movement) is delegated to [`TextInput`]; this
/// only owns the filter-specific Esc/Enter/empty-Backspace exit policy.
fn apply_filter_edit(code: KeyCode, query: &mut TextInput) -> FilterKey {
    match code {
        KeyCode::Esc => {
            query.clear();
            FilterKey::Cleared
        }
        KeyCode::Enter => FilterKey::Exited,
        // Backspace on an already-empty query leaves the input (keeps the old shortcut).
        KeyCode::Backspace if query.is_empty() => FilterKey::Exited,
        _ => match query.apply_edit(code) {
            EditResult::Changed => FilterKey::Edited,
            EditResult::Moved => FilterKey::Moved,
            EditResult::Ignored => FilterKey::Pass,
        },
    }
}

impl AppState {
    fn handle_key_filter(&mut self, code: KeyCode) -> KeyOutcome {
        // Live navigation while typing: arrows move the focused pane's selection.
        match code {
            KeyCode::Up => {
                self.list_move_focused(false);
                return KeyOutcome::None;
            }
            KeyCode::Down => {
                self.list_move_focused(true);
                return KeyOutcome::None;
            }
            // Tab commits (keeps the query), leaves input, and switches pane.
            KeyCode::Tab => {
                self.filtering = false;
                self.focus = match self.focus {
                    FocusPane::Local => FocusPane::Gist,
                    FocusPane::Gist => FocusPane::Local,
                };
                return KeyOutcome::None;
            }
            _ => {}
        }
        let focus = self.focus;
        let query = match focus {
            FocusPane::Local => &mut self.local_filter_query,
            FocusPane::Gist => &mut self.filter_query,
        };
        match apply_filter_edit(code, query) {
            FilterKey::Edited => self.reset_focused_filter_scroll(),
            FilterKey::Cleared => {
                self.filtering = false;
                self.reset_focused_filter_scroll();
            }
            FilterKey::Exited => self.filtering = false,
            FilterKey::Moved | FilterKey::Pass => {}
        }
        KeyOutcome::None
    }

    fn handle_key_list(&mut self, code: KeyCode) -> KeyOutcome {
        // Any key dismisses a lingering status message (e.g. "Downloaded …"). A new
        // status may be set afterwards by the run_loop IO helper for this key.
        self.status = None;
        // Any key disarms the pending quit; the quit arm below re-arms on the first q/Esc.
        let quit_armed = std::mem::take(&mut self.quit_armed);
        match code {
            // Quitting the app is a two-step tap so a stray q/Esc on the list does not exit.
            KeyCode::Char('q') | KeyCode::Esc => {
                if quit_armed {
                    return KeyOutcome::Quit;
                }
                self.quit_armed = true;
                self.status = Some("Press q again to quit (any other key cancels)".into());
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    FocusPane::Local => FocusPane::Gist,
                    FocusPane::Gist => FocusPane::Local,
                };
            }
            // 1/2 jump straight to a pane (mirrors Tab; selection indices are untouched).
            KeyCode::Char('1') => self.focus = FocusPane::Local,
            KeyCode::Char('2') => self.focus = FocusPane::Gist,
            // Flip which pane drives the match ranking (anchor), independent of focus.
            KeyCode::Char('a') => {
                self.anchor = match self.anchor {
                    FocusPane::Local => FocusPane::Gist,
                    FocusPane::Gist => FocusPane::Local,
                };
                // Reset the newly-ranked (non-driver) pane to its top match.
                self.reset_ranked_pane();
            }
            KeyCode::Char('t') => {
                self.gist_view = match self.gist_view {
                    GistView::Description => GistView::Id,
                    GistView::Id => GistView::Description,
                };
            }
            KeyCode::Char('v') => {
                self.gist_type_filter = self.gist_type_filter.next();
                self.gist_index = 0;
                self.gist_hscroll = 0;
            }
            KeyCode::Char('*') => return self.star_toggle_intent(),
            KeyCode::Char('s') => self.cycle_focused_sort(),
            KeyCode::Char('r') => {
                self.local_recursive = !self.local_recursive;
                self.local_index = 0;
                self.local_hscroll = 0;
                return KeyOutcome::RefreshLocals;
            }
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('y') => return KeyOutcome::CopyGistUrl,
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char('P') => self.open_pins(),
            KeyCode::Char('S') => return KeyOutcome::SyncSelectedPair,
            KeyCode::Char('g') => self.open_gist_manager(),
            KeyCode::Char('H') if self.gist_index < self.ranked_gists().len() => {
                if self.open_revisions(Screen::List) {
                    return KeyOutcome::FetchRevisions;
                }
                self.status = Some("select a gist file to view revision history".into());
            }
            KeyCode::Char('e') => {
                if self.selected_local().is_some() {
                    return KeyOutcome::EditLocal;
                }
                self.status = Some("select a local file to edit".into());
            }
            KeyCode::Char(' ') if let Some(gist) = self.selected_gist() => {
                if self.block_if_non_previewable_gist_file(&gist.file.gist_id, &gist.file.filename)
                {
                    return KeyOutcome::None;
                }
                self.preview_return = Screen::List;
                return KeyOutcome::PreviewContent;
            }
            KeyCode::Char('d')
                if self.focus == FocusPane::Gist && self.gist_index < self.ranked_gists().len() =>
            {
                return KeyOutcome::DownloadGist;
            }
            // Enter works from either pane: it diffs the selected local file against the
            // selected gist (the top match when focus is on the local pane). Snapshot both
            // ranked lists once here instead of recomputing them through the bounds guard plus
            // `selected_gist`/`selected_local` (perf-1, #154).
            KeyCode::Enter => {
                let ranked = self.ranked_gists();
                let Some(gist) = ranked.get(self.gist_index) else {
                    return KeyOutcome::None;
                };
                let local_path = self
                    .visible_locals()
                    .get(self.local_index)
                    .map(|r| r.candidate.path.clone());
                if self.block_if_non_previewable_diff(
                    &gist.file.gist_id,
                    &gist.file.filename,
                    local_path.as_deref(),
                ) {
                    return KeyOutcome::None;
                }
                return KeyOutcome::PreviewDiff;
            }
            KeyCode::Char('p') => return self.pin_toggle_intent(),
            KeyCode::Char('u') => return self.upload_intent(),
            KeyCode::Char('X') => self.remove_gist_file_intent(),
            KeyCode::Char('n') => self.create_gist_intent(),
            _ => {}
        }
        KeyOutcome::None
    }

    /// Reset the focused pane's selection index and horizontal scroll (used when a
    /// filter edit changes the visible rows).
    fn reset_focused_filter_scroll(&mut self) {
        match self.focus {
            FocusPane::Local => {
                self.local_index = 0;
                self.local_hscroll = 0;
            }
            FocusPane::Gist => {
                self.gist_index = 0;
                self.gist_hscroll = 0;
            }
        }
    }

    /// Page the focused list-pane selection by [`PAGE_SCROLL`] rows (clamped at bounds).
    fn list_page_focused(&mut self, forward: bool) {
        let step = PAGE_SCROLL as usize;
        match self.focus {
            FocusPane::Local => {
                let len = self.visible_locals().len();
                if len == 0 {
                    return;
                }
                let max = len - 1;
                self.local_index = if forward {
                    (self.local_index + step).min(max)
                } else {
                    self.local_index.saturating_sub(step)
                };
                self.local_hscroll = 0;
                if self.anchor == FocusPane::Local {
                    self.reset_ranked_pane();
                }
            }
            FocusPane::Gist => {
                let len = self.ranked_gists().len();
                if len == 0 {
                    return;
                }
                let max = len - 1;
                self.gist_index = if forward {
                    (self.gist_index + step).min(max)
                } else {
                    self.gist_index.saturating_sub(step)
                };
                self.gist_hscroll = 0;
                if self.anchor == FocusPane::Gist {
                    self.reset_ranked_pane();
                }
            }
        }
    }

    /// Move the selection in the focused list pane. `forward` advances toward the end of the
    /// list; otherwise it moves toward the top. Both directions clamp at the pane's bounds,
    /// reset the horizontal scroll, and re-rank the opposite pane when the focused pane is the
    /// ranking anchor.
    fn list_move_focused(&mut self, forward: bool) {
        match self.focus {
            FocusPane::Local => {
                let len = self.visible_locals().len();
                if forward {
                    if self.local_index + 1 >= len {
                        return;
                    }
                    self.local_index += 1;
                } else {
                    if self.local_index == 0 {
                        return;
                    }
                    self.local_index -= 1;
                }
                self.local_hscroll = 0;
                if self.anchor == FocusPane::Local {
                    self.reset_ranked_pane();
                }
            }
            FocusPane::Gist => {
                if forward {
                    if self.gist_index + 1 >= self.ranked_gists().len() {
                        return;
                    }
                    self.gist_index += 1;
                } else {
                    if self.gist_index == 0 {
                        return;
                    }
                    self.gist_index -= 1;
                }
                self.gist_hscroll = 0;
                if self.anchor == FocusPane::Gist {
                    self.reset_ranked_pane();
                }
            }
        }
    }

    /// Cycle the focused pane's sort order (match -> name -> recent -> match) and reset that
    /// pane's selection and horizontal scroll.
    fn cycle_focused_sort(&mut self) {
        match self.focus {
            FocusPane::Gist => {
                self.gist_sort = self.gist_sort.next();
                self.gist_index = 0;
                self.gist_hscroll = 0;
            }
            FocusPane::Local => {
                self.local_sort = self.local_sort.next();
                self.local_index = 0;
                self.local_hscroll = 0;
            }
        }
    }

    /// Open the gist-level manager (`Screen::Gists`), landing on the gist that owns the
    /// selected file row. Resets the manager's own filters first so the target is always
    /// visible. No-op (with a status hint) when there are no gists to manage.
    fn open_gist_manager(&mut self) {
        if self.gists.is_empty() {
            self.status = Some("no gists to manage".into());
            return;
        }
        self.gist_manager.filtering = false;
        self.gist_manager.filter_query.clear();
        self.gist_manager.type_filter = GistTypeFilter::All;
        self.gist_manager.hscroll = 0;
        self.editing_description = false;
        self.description_input.clear();
        let target = self.selected_gist().map(|g| g.file.gist_id);
        let groups = self.visible_gist_groups();
        self.gist_manager.index = target
            .and_then(|id| groups.iter().position(|g| g.id == id))
            .unwrap_or(0);
        self.screen = Screen::Gists;
    }

    /// Open the Pins view (`Screen::Pins`), resetting its selection/scroll so a stale
    /// filtered-in position from a previous visit never lingers.
    fn open_pins(&mut self) {
        self.pins.index = 0;
        self.pins.hscroll = 0;
        self.screen = Screen::Pins;
    }

    /// Pin/unpin the selected local↔gist pair: returns [`KeyOutcome::Unpin`] when the exact
    /// pair is already pinned, otherwise [`KeyOutcome::Pin`]. Requires a selection in both
    /// panes; otherwise it just sets a status hint.
    fn pin_toggle_intent(&mut self) -> KeyOutcome {
        let (Some(local), Some(gist)) = (self.selected_local(), self.selected_gist()) else {
            self.status = Some("select a local file and a gist to pin".into());
            return KeyOutcome::None;
        };
        let already = self.pinned.iter().any(|m| {
            m.local_path == local.path
                && m.gist_id == gist.file.gist_id
                && m.gist_filename == gist.file.filename
        });
        if already {
            KeyOutcome::Unpin
        } else if self.block_if_foreign_gist(&gist.file.gist_id, true) {
            KeyOutcome::None
        } else {
            KeyOutcome::Pin
        }
    }

    fn star_toggle_intent(&mut self) -> KeyOutcome {
        if self.context_gist_id().is_some() {
            KeyOutcome::ToggleGistStar
        } else {
            self.set_status("select a gist first");
            KeyOutcome::None
        }
    }

    fn fork_intent(&mut self) -> KeyOutcome {
        let Some(gist_id) = self.context_gist_id() else {
            self.set_status("select a gist to fork");
            return KeyOutcome::None;
        };
        if self.gist_is_owned(&gist_id) {
            self.set_status("already yours — no fork needed");
            KeyOutcome::None
        } else {
            KeyOutcome::ForkGist
        }
    }

    /// Stage removal of the selected gist file behind a y/n confirm (`Screen::Confirm`). A gist
    /// must keep at least one file, so removing the gist's only file is refused — delete the
    /// whole gist from the gist-level view (`g` then `X`) instead.
    fn remove_gist_file_intent(&mut self) {
        let Some(gist) = self.selected_gist() else {
            self.status = Some("select a gist file to remove".into());
            return;
        };
        let gist_id = gist.file.gist_id.clone();
        if self.block_if_foreign_gist(&gist_id, false) {
            return;
        }
        let filename = gist.file.filename.clone();
        if self.gist_file_count(&gist_id) <= 1 {
            self.status = Some(format!(
                "{filename} is the gist's only file — use g then X to delete the gist"
            ));
            return;
        }
        let label = if gist.file.description.is_empty() {
            gist_id.clone()
        } else {
            gist.file.description.clone()
        };
        self.diff_text = format!(
            "Remove file \"{filename}\" from gist {gist_id} ({label}).\n\nThe other files in this gist are kept. This cannot be undone."
        );
        self.diff_scroll = 0;
        self.diff_hscroll = 0;
        self.pending_action = Some(PendingAction::RemoveFile {
            gist_id,
            filename,
            label,
        });
        self.screen = Screen::Confirm;
    }

    /// Stage creation of a new gist from the selected local file. Create is a two-step confirm:
    /// type an optional description (inline editor, shared with the gist-level view), then
    /// choose visibility. Requires a selected local file.
    fn create_gist_intent(&mut self) {
        let Some(local) = self.selected_local() else {
            self.status = Some("select a local file to create a gist".into());
            return;
        };
        self.diff_text = format!(
            "Create a new gist from {}.\n\nType an optional description, then choose visibility.",
            local.path.display()
        );
        self.diff_scroll = 0;
        self.diff_hscroll = 0;
        self.editing_description = true;
        self.description_input.clear();
        self.pending_action = Some(PendingAction::Create {
            local_path: local.path,
        });
        self.screen = Screen::Confirm;
    }

    fn handle_key_diff(&mut self, code: KeyCode) -> KeyOutcome {
        match code {
            // In the diff, q and Esc return to diff_return (normally List; Pins for pin diffs).
            KeyCode::Char('q') | KeyCode::Esc => {
                let ret = self.diff_return;
                self.back_to_list();
                self.screen = ret;
                self.diff_return = Screen::List;
            }
            // Identical files have nothing to sync, so download/upload are not offered.
            // Revision-history diffs are read-only (no local file pairing).
            KeyCode::Char('d') if self.diff_allows_sync() && !self.diff_identical => {
                if self.download_target.exists() {
                    self.pending_action = Some(PendingAction::Download);
                    self.screen = Screen::Confirm;
                } else {
                    return KeyOutcome::Download;
                }
            }
            KeyCode::Char('u') if self.diff_allows_sync() && !self.diff_identical => {
                return self.upload_intent();
            }
            // Toggle between the configured context radius and the full file; the line
            // count changes, so reset the vertical scroll. The choice is persisted.
            KeyCode::Char('c') => {
                self.diff_show_full = !self.diff_show_full;
                self.diff_scroll = 0;
                return KeyOutcome::PersistDiffContext;
            }
            // Soft-wrap long lines instead of horizontal scrolling; reset the now-meaningless
            // horizontal offset so wrapped lines start at column 0.
            KeyCode::Char('w') => {
                self.diff_wrap = !self.diff_wrap;
                self.diff_hscroll = 0;
            }
            _ => {}
        }
        KeyOutcome::None
    }

    fn handle_key_confirm(&mut self, code: KeyCode) -> KeyOutcome {
        // While typing the create flow's description, arrows drive the text cursor (handled
        // below), not the background diff scroll.
        match self.pending_action.clone() {
            Some(PendingAction::Download) => match code {
                KeyCode::Char('y') => return KeyOutcome::Download,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.pending_action = None;
                    self.screen = Screen::Diff;
                }
                _ => {}
            },
            Some(PendingAction::Upload { ref local_path, .. }) => match code {
                KeyCode::Char('y') if self.upload.watching => {
                    self.set_status("editor still open — finish editing first");
                }
                KeyCode::Char('y') => return KeyOutcome::Upload,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.pending_action = None;
                    // Return to wherever the upload was initiated from (List, or Pins for
                    // a pin push) instead of always snapping back to List.
                    self.screen = self.diff_return;
                    // The background watch thread (if any) is not force-killed — it cleans
                    // itself up once the editor closes. Reset the flag now so a stale
                    // late-arriving event (see AppState::apply_upload_edit_event) doesn't
                    // matter, and so a future upload-edit session isn't blocked by it.
                    self.upload.watching = false;
                }
                KeyCode::Char('e') if self.upload.watching => {
                    self.set_status("editor already open");
                }
                KeyCode::Char('e') => return KeyOutcome::EditUpload,
                KeyCode::Char('p') if is_json_file(local_path) => {
                    self.upload.json_pretty = !self.upload.json_pretty;
                    self.update_upload_diff();
                }
                KeyCode::Char('s') if is_json_file(local_path) => {
                    self.upload.json_sort = !self.upload.json_sort;
                    self.update_upload_diff();
                }
                _ => {}
            },
            Some(PendingAction::Create { .. }) if self.editing_description => match code {
                // Step 1: type the optional description. Enter advances to the
                // visibility choice; Esc cancels the whole create.
                KeyCode::Enter => self.editing_description = false,
                KeyCode::Esc => {
                    self.editing_description = false;
                    self.description_input.clear();
                    self.back_to_list();
                }
                _ => {
                    self.description_input.apply_edit(code);
                }
            },
            Some(PendingAction::Create { .. }) => match code {
                // Step 2: choose visibility (the description is kept in description_input).
                KeyCode::Char('s') => return KeyOutcome::Create(false),
                KeyCode::Char('p') => return KeyOutcome::Create(true),
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.description_input.clear();
                    self.back_to_list();
                }
                _ => {}
            },
            Some(PendingAction::Delete { .. }) => match code {
                KeyCode::Char('y') => return KeyOutcome::ExecuteDelete,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.back_to_list();
                }
                _ => {}
            },
            Some(PendingAction::RemoveFile { .. }) => match code {
                KeyCode::Char('y') => return KeyOutcome::ExecuteRemoveFile,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.back_to_list();
                }
                _ => {}
            },
            Some(PendingAction::CompactGist { .. }) => match code {
                KeyCode::Char('y') => return KeyOutcome::ExecuteCompactGist,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    // Return to whichever screen launched the compaction (Gists or GistDetail).
                    self.pending_action = None;
                    self.screen = self.detail.compact_return_screen;
                }
                _ => {}
            },
            Some(PendingAction::RestoreRevision { .. }) => match code {
                KeyCode::Char('y') => return KeyOutcome::ExecuteRestoreRevision,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.pending_action = None;
                    self.screen = Screen::Revisions;
                }
                _ => {}
            },
            _ => {
                if matches!(code, KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q')) {
                    self.pending_action = None;
                    self.screen = Screen::List;
                }
            }
        }
        KeyOutcome::None
    }
}

/// Whether a column/row position lands inside a `Rect`.
fn point_in(rect: ratatui::layout::Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
}
