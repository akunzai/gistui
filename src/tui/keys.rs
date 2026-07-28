use super::bg::revision_version_label;
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

/// Dispatch [`NavAction`] onto a Pins/Gists [`ListCursor`] (step from [`PAGE_SCROLL`]).
fn apply_list_cursor_nav(cursor: &mut ListCursor, action: NavAction, len: usize, hmax: u16) {
    let step = PAGE_SCROLL as usize;
    match action {
        NavAction::Down => cursor.down(len),
        NavAction::Up => cursor.up(),
        NavAction::Right => cursor.right(hmax),
        NavAction::Left => cursor.left(),
        NavAction::PageDown => cursor.page_down(len, step),
        NavAction::PageUp => cursor.page_up(step),
    }
}

/// `T` theme toggle accepts a plain capital key (Caps Lock) or Shift+T; reject Ctrl/Alt combos.
fn theme_toggle_modifiers_ok(modifiers: KeyModifiers) -> bool {
    modifiers.is_empty() || modifiers == KeyModifiers::SHIFT
}

impl AppState {
    pub fn handle_key(&mut self, code: KeyCode) -> KeyOutcome {
        self.handle_key_with(code, KeyModifiers::NONE)
    }

    pub fn handle_key_with(&mut self, code: KeyCode, modifiers: KeyModifiers) -> KeyOutcome {
        if self.screen.is_palette() {
            return self.handle_key_palette(code, modifiers);
        }
        if code == KeyCode::Char(';') && modifiers.is_empty() && !self.palette_blocked() {
            self.open_palette_menu(None);
            return KeyOutcome::None;
        }
        if code == KeyCode::Char('p')
            && modifiers.contains(KeyModifiers::CONTROL)
            && !self.palette_blocked()
        {
            self.open_palette_command();
            return KeyOutcome::None;
        }
        // Global theme toggle: skip while any inline text input is active so `T` can still
        // be typed into filters and description editors.
        if code == KeyCode::Char('T')
            && theme_toggle_modifiers_ok(modifiers)
            && !self.filtering
            && !self.pins().is_some_and(|p| p.filtering)
            && !self.gist_manager().is_some_and(|g| g.filtering)
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
        match &self.screen {
            Screen::List if self.filtering => self.handle_key_filter(code),
            Screen::List => self.handle_key_list(code),
            Screen::Diff(_) => self.handle_key_diff(code),
            Screen::Confirm(_) => self.handle_key_confirm(code),
            Screen::Preview(_) => self.handle_key_preview(code),
            Screen::Help(_) => self.handle_key_help(code),
            Screen::Pins(_) => self.handle_key_pins(code),
            Screen::Gists(_) => self.handle_key_gists(code),
            Screen::GistDetail(_) => self.handle_key_detail(code),
            Screen::Revisions(_) => self.handle_key_revisions(code),
            Screen::Config(_) => self.handle_key_config(code),
            // Exhaustiveness only — palette keys return above via is_palette() early path.
            Screen::Palette(_) => KeyOutcome::None,
        }
    }

    /// Open the Help screen on the topic for the current screen, remembering where to return.
    /// A no-op while already on Help — otherwise the top bar's `(?)Help` click (reachable from
    /// any screen, including Help itself) would push `Screen::Help` onto its own nav_stack,
    /// trapping Esc/`?`/the close button in Help with no keyboard way out.
    pub(crate) fn open_help(&mut self) {
        if self.screen.is_help() {
            return;
        }
        let topic = HelpTopic::for_screen(&self.screen);
        self.enter(Screen::Help(Box::new(HelpState {
            topic,
            index_open: false,
            scroll: 0,
            index_sel: 0,
        })));
    }

    /// Open the flat Settings screen (`C` or palette). Opening alone does not write config.
    pub(crate) fn open_config(&mut self) {
        if self.screen.is_config() {
            return;
        }
        self.enter(Screen::Config(Box::new(ConfigState { index: 0 })));
    }

    /// Value string shown for a Config field row.
    pub(crate) fn config_field_value(&self, field: ConfigField) -> String {
        match field {
            ConfigField::Theme => match self.theme_choice {
                crate::config::ThemeChoice::Dark => "dark".into(),
                crate::config::ThemeChoice::Light => "light".into(),
            },
            ConfigField::Mouse => {
                if self.config_mouse {
                    "on".into()
                } else {
                    "off".into()
                }
            }
            ConfigField::CheckUpdates => {
                if self.config_check_updates {
                    "on".into()
                } else {
                    "off".into()
                }
            }
            ConfigField::IgnoreTrailingNewline => {
                if self.ignore_trailing_newline {
                    "on".into()
                } else {
                    "off".into()
                }
            }
            ConfigField::ScanDepth => self.scan_depth.to_string(),
            ConfigField::DiffContext => self.diff_context.to_string(),
        }
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
        if (self.filtering
            || self.pins().is_some_and(|p| p.filtering)
            || self.gist_manager().is_some_and(|g| g.filtering))
            && !matches!(action, NavAction::PageUp | NavAction::PageDown)
        {
            return false;
        }
        // Pins/Gists: precompute len/hmax with &self, then mut ListCursor (issue #274).
        // Cannot hold `&mut PinsState` from `match &mut self.screen` while calling helpers.
        if matches!(self.screen, Screen::Pins(_)) {
            let len = self.visible_pin_indices().len();
            let hmax = self.pins_hscroll_max();
            let Some(pins) = self.pins_mut() else {
                return false;
            };
            apply_list_cursor_nav(&mut pins.cursor, action, len, hmax);
            return true;
        }
        if matches!(self.screen, Screen::Gists(_)) {
            let len = self.visible_gist_groups().len();
            let hmax = self.gists_hscroll_max();
            let Some(gm) = self.gist_manager_mut() else {
                return false;
            };
            apply_list_cursor_nav(&mut gm.cursor, action, len, hmax);
            return true;
        }
        match &mut self.screen {
            Screen::Palette(_) => {
                let len = self.palette_visible_items().len();
                if let Some(p) = self.palette_mut() {
                    match action {
                        NavAction::Up => {
                            p.selected = p.selected.saturating_sub(1);
                        }
                        NavAction::Down => {
                            if len > 0 && p.selected + 1 < len {
                                p.selected += 1;
                            }
                        }
                        _ => return false,
                    }
                }
                true
            }
            Screen::Help(help) => {
                let topics = HelpTopic::all();
                if help.index_open {
                    match action {
                        NavAction::Up => help.index_sel = help.index_sel.saturating_sub(1),
                        NavAction::Down => {
                            if help.index_sel + 1 < topics.len() {
                                help.index_sel += 1;
                            }
                        }
                        _ => return false,
                    }
                } else {
                    match action {
                        NavAction::Up => {
                            help.scroll = help.scroll.saturating_sub(1);
                        }
                        NavAction::Down => {
                            help.scroll = help.scroll.saturating_add(1);
                        }
                        NavAction::PageUp => {
                            help.scroll = help.scroll.saturating_sub(PAGE_SCROLL);
                        }
                        NavAction::PageDown => {
                            help.scroll = help.scroll.saturating_add(PAGE_SCROLL);
                        }
                        _ => return false,
                    }
                }
                true
            }
            Screen::GistDetail(_) => {
                match action {
                    NavAction::Down => self.detail_nav(1),
                    NavAction::Up => self.detail_nav(-1),
                    NavAction::PageDown => self.detail_nav(10),
                    NavAction::PageUp => self.detail_nav(-10),
                    _ => return false,
                }
                true
            }
            Screen::Revisions(rev) => {
                let entries_len = rev.entries.as_ref().map(|e| e.len()).unwrap_or(0);
                if entries_len == 0 {
                    return false;
                }
                match action {
                    NavAction::Down => {
                        rev.index = (rev.index + 1).min(entries_len - 1);
                    }
                    NavAction::Up => {
                        rev.index = rev.index.saturating_sub(1);
                    }
                    NavAction::PageDown => {
                        rev.index = (rev.index + PAGE_SCROLL as usize).min(entries_len - 1);
                    }
                    NavAction::PageUp => {
                        rev.index = rev.index.saturating_sub(PAGE_SCROLL as usize);
                    }
                    NavAction::Left => {
                        rev.hscroll = rev.hscroll.saturating_sub(1);
                    }
                    NavAction::Right => {
                        rev.hscroll = rev.hscroll.saturating_add(1);
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
            Screen::Config(cfg) => {
                let n = ConfigField::ALL.len();
                match action {
                    NavAction::Up => {
                        cfg.index = cfg.index.saturating_sub(1);
                    }
                    NavAction::Down => {
                        if cfg.index + 1 < n {
                            cfg.index += 1;
                        }
                    }
                    NavAction::Left | NavAction::Right => {
                        // Adjust is handled in handle_key_config (needs PersistSettings).
                        return false;
                    }
                    _ => return false,
                }
                true
            }
            // Diff, Preview and Confirm all scroll the same diff/preview buffer identically.
            Screen::Diff(_) | Screen::Preview(_) | Screen::Confirm(_) => {
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
            // Exhaustiveness only — Pins/Gists return above before this match.
            Screen::Pins(_) | Screen::Gists(_) => {
                unreachable!("Pins/Gists navigation is handled before the match")
            }
        }
    }

    /// Screens that clear a one-shot status (and the list quit arm) on any key — including
    /// navigation keys handled before the per-screen handler runs.
    fn dismiss_ephemeral_screen_state(&mut self) {
        match &mut self.screen {
            Screen::List => {
                self.status = None;
                self.quit_armed = false;
            }
            Screen::Pins(_)
            | Screen::Gists(_)
            | Screen::GistDetail(_)
            | Screen::Revisions(_)
            | Screen::Preview(_) => self.status = None,
            _ => {}
        }
    }

    fn handle_key_pins(&mut self, code: KeyCode) -> KeyOutcome {
        // One-shot: any key dismisses a lingering sync status; the run_loop IO helper for this
        // key may set a fresh one afterwards (e.g. "already in sync").
        self.status = None;
        // Inline text filter: live-navigate with arrows; Tab is a no-op (single pane).
        if self.pins().is_some_and(|p| p.filtering) {
            let Some(pins) = self.pins_mut() else {
                return KeyOutcome::None;
            };
            match code {
                KeyCode::Up => pins.cursor.up(),
                KeyCode::Down => {
                    // visible length needs self; re-fetch after borrow ends
                }
                _ => match apply_filter_edit(code, &mut pins.filter_query) {
                    FilterKey::Edited => pins.cursor.reset(),
                    FilterKey::Cleared => {
                        pins.filtering = false;
                        pins.cursor.reset();
                    }
                    FilterKey::Exited => pins.filtering = false,
                    FilterKey::Moved | FilterKey::Pass => {}
                },
            }
            if code == KeyCode::Down {
                let len = self.visible_pin_indices().len();
                if let Some(pins) = self.pins_mut() {
                    pins.cursor.down(len);
                }
            }
            return KeyOutcome::None;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.screen = Screen::List,
            KeyCode::Char('/') => {
                if let Some(pins) = self.pins_mut() {
                    pins.filtering = true;
                }
            }
            KeyCode::Enter if pins_guard(self, code) => {
                let Some(index) = self.selected_pin_index() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::PreviewPinDiff { index };
            }
            KeyCode::Char('x') if pins_guard(self, code) => {
                let Some(index) = self.selected_pin_index() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::UnpinAtPin { index };
            }
            KeyCode::Char('s') if pins_guard(self, code) => {
                let Some(index) = self.selected_pin_index() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::SyncPinAuto { index };
            }
            KeyCode::Char('u') if pins_guard(self, code) => {
                let Some(index) = self.selected_pin_index() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::SyncPinPush { index };
            }
            KeyCode::Char('d') if pins_guard(self, code) => {
                let Some(index) = self.selected_pin_index() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::SyncPinPull { index };
            }
            KeyCode::Char('o') => {
                if let Some(pins) = self.pins_mut() {
                    pins.sort = pins.sort.next();
                    pins.cursor.reset();
                }
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }

    fn handle_key_gists(&mut self, code: KeyCode) -> KeyOutcome {
        self.status = None;
        // Inline text filter: live-navigate with arrows; Tab is a no-op (single pane).
        if self.gist_manager().is_some_and(|g| g.filtering) {
            let Some(gm) = self.gist_manager_mut() else {
                return KeyOutcome::None;
            };
            match code {
                KeyCode::Up => gm.cursor.up(),
                KeyCode::Down => {
                    // visible length needs self; re-fetch after borrow ends
                }
                _ => match apply_filter_edit(code, &mut gm.filter_query) {
                    FilterKey::Edited => gm.cursor.reset(),
                    FilterKey::Cleared => {
                        gm.filtering = false;
                        gm.cursor.reset();
                    }
                    FilterKey::Exited => gm.filtering = false,
                    FilterKey::Moved | FilterKey::Pass => {}
                },
            }
            if code == KeyCode::Down {
                let len = self.visible_gist_groups().len();
                if let Some(gm) = self.gist_manager_mut() {
                    gm.cursor.down(len);
                }
            }
            return KeyOutcome::None;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.screen = Screen::List,
            KeyCode::Char('/') => {
                if let Some(gm) = self.gist_manager_mut() {
                    gm.filtering = true;
                }
            }
            KeyCode::Char('s') => {
                if let Some(gm) = self.gist_manager_mut() {
                    gm.sort = gm.sort.next();
                    gm.cursor.reset();
                }
            }
            KeyCode::Char('v') => {
                if let Some(gm) = self.gist_manager_mut() {
                    gm.type_filter = gm.type_filter.next();
                    gm.cursor.reset();
                }
            }
            // Not gated through `gists_guard`: `star_toggle_intent` already has its own
            // complete "select a gist first" message for the no-selection case.
            KeyCode::Char('*') => return self.star_toggle_intent(),
            KeyCode::Enter if gists_guard(self, code) => {
                let Some(group) = self.selected_group() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::OpenGistDetail {
                    gist_id: group.id.clone(),
                };
            }
            KeyCode::Char('o') if gists_guard(self, code) => {
                let Some(gist_id) = self.context_gist_id() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::OpenBrowser { gist_id };
            }
            KeyCode::Char('y') if gists_guard(self, code) => {
                let Some(gist_id) = self.context_gist_id() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::CopyGistUrl { gist_id };
            }
            KeyCode::Char('H') if gists_guard(self, code) => {
                if self.open_revisions() {
                    if let Some(gist_id) = self.revision().and_then(|r| r.gist_id.clone()) {
                        return KeyOutcome::FetchRevisions { gist_id };
                    }
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
                KeyCode::Enter => {
                    let Some(gist_id) = self
                        .detail()
                        .and_then(|d| d.gist_id.clone())
                        .or_else(|| self.selected_group().map(|g| g.id.clone()))
                    else {
                        return KeyOutcome::None;
                    };
                    return KeyOutcome::ApplyDescription {
                        gist_id,
                        description: self.description_input.to_string(),
                    };
                }
                _ => {
                    self.description_input.apply_edit(code);
                }
            }
            return KeyOutcome::None;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.leave();
            }
            KeyCode::Char('o') if detail_guard(self, code) => {
                let Some(gist_id) = self.context_gist_id() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::OpenBrowser { gist_id };
            }
            KeyCode::Char('y') if detail_guard(self, code) => {
                let Some(gist_id) = self.context_gist_id() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::CopyGistUrl { gist_id };
            }
            KeyCode::Char('H') if detail_guard(self, code) => {
                if self.open_revisions() {
                    if let Some(gist_id) = self.revision().and_then(|r| r.gist_id.clone()) {
                        return KeyOutcome::FetchRevisions { gist_id };
                    }
                }
            }
            KeyCode::Char('e') if detail_guard(self, code) => {
                let Some(id) = self.detail().and_then(|d| d.gist_id.clone()) else {
                    return KeyOutcome::None;
                };
                if let Some(group) = self.group_by_id(&id) {
                    self.editing_description = true;
                    self.description_input.set(group.description.clone());
                }
            }
            KeyCode::Char('c') if detail_guard(self, code) => {
                let Some(id) = self.detail().and_then(|d| d.gist_id.clone()) else {
                    return KeyOutcome::None;
                };
                self.pending_return = Some(self.park_gist_detail_screen());
                let label = self
                    .group_by_id(&id)
                    .map(|g| {
                        if g.description.trim().is_empty() {
                            g.id
                        } else {
                            g.description
                        }
                    })
                    .unwrap_or_else(|| id.clone());
                return KeyOutcome::CompactGist { gist_id: id, label };
            }
            // Not gated through `detail_guard`: `star_toggle_intent`/`fork_intent` already have
            // their own complete messages for the disabled cases ("select a gist first",
            // "already yours — no fork needed").
            KeyCode::Char('*') => return self.star_toggle_intent(),
            KeyCode::Char('F') => return self.fork_intent(),
            // 1–9 preview the content of the Nth file in the gist (full-screen preview).
            KeyCode::Char(c @ '1'..='9') => {
                return self.preview_detail_file((c as u8 - b'1') as usize);
            }
            KeyCode::Tab => {
                let Some(d) = self.detail_mut() else {
                    return KeyOutcome::None;
                };
                d.focus = match d.focus {
                    DetailFocus::Comments => DetailFocus::Files,
                    DetailFocus::Files => DetailFocus::Comments,
                };
                let fetch =
                    d.focus == DetailFocus::Comments && d.comments.is_none() && !d.comments_loading;
                if fetch {
                    if let Some(gist_id) = self.detail().and_then(|d| d.gist_id.clone()) {
                        return KeyOutcome::FetchComments { gist_id };
                    }
                }
            }
            // X deletes the whole gist (y/n confirm). Reuses the shared Delete confirm path,
            // which lands on the list once the gist is gone. Owned gists only (no-op otherwise).
            KeyCode::Char('X') if detail_guard(self, code) => {
                if let Some(group) = self
                    .detail()
                    .and_then(|d| d.gist_id.clone())
                    .and_then(|id| self.group_by_id(&id))
                {
                    let label = if group.description.is_empty() {
                        group.id.clone()
                    } else {
                        group.description.clone()
                    };
                    let text = format!(
                        "Delete gist {} ({} file(s)): {label}.\n\nThis permanently removes the entire gist and all its files.",
                        group.id, group.file_count
                    );
                    self.enter_confirm(
                        PendingAction::Delete {
                            gist_id: group.id.clone(),
                            label,
                        },
                        text,
                    );
                }
            }
            KeyCode::Enter if detail_guard(self, code) => {
                if let Some(gist_id) = self.detail().and_then(|d| d.gist_id.clone()) {
                    let cursor = self.detail().map(|d| d.file_cursor).unwrap_or(0);
                    if let Some(filename) = self.gist_filenames(&gist_id).into_iter().nth(cursor) {
                        self.pending_return = Some(self.park_gist_detail_screen());
                        return KeyOutcome::PreviewContent {
                            file: crate::domain::GistFileRef::id_name(gist_id, filename),
                        };
                    }
                }
            }
            KeyCode::Char('m') if detail_guard(self, code) => {
                if let Some(gist_id) = self.detail().and_then(|d| d.gist_id.clone()) {
                    let page = self
                        .detail()
                        .map(|d| d.comments_loaded_oldest_page.saturating_sub(1))
                        .unwrap_or(0);
                    if page > 0 {
                        return KeyOutcome::LoadOlderComments { gist_id, page };
                    }
                }
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }

    fn handle_key_revisions(&mut self, code: KeyCode) -> KeyOutcome {
        self.status = None;
        let entries_len = self
            .revision()
            .and_then(|r| r.entries.as_ref().map(|e| e.len()))
            .unwrap_or(0);
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.leave();
            }
            KeyCode::Enter if revisions_guard(self, code) => {
                return self.revision_diff_incremental_intent();
            }
            KeyCode::Char('D') if revisions_guard(self, code) => {
                return self.revision_diff_intent();
            }
            // Distinct from `revisions_guard`'s `D` case only by omitting the `previewable`
            // check, so a non-previewable file off-head still gets this precise message
            // instead of falling to the (misleading, head-only) fallback below it.
            KeyCode::Char('D')
                if entries_len > 0 && self.revision().is_some_and(|r| r.index > 0) =>
            {
                return self.revision_diff_intent();
            }
            KeyCode::Char('D') if entries_len > 0 => {
                self.set_status("already at current revision");
            }
            KeyCode::Char('r') if revisions_guard(self, code) => {
                return self.restore_revision_preview_intent();
            }
            KeyCode::Char('r') if entries_len <= 1 => {
                self.set_status("only one revision — nothing to restore");
            }
            KeyCode::Char('r') if self.revision().is_some_and(|r| r.index == 0) => {
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
        let focus = self.detail().map(|d| d.focus).unwrap_or_default();
        match focus {
            DetailFocus::Comments => {
                if let Some(d) = self.detail_mut() {
                    d.scroll = if delta < 0 {
                        d.scroll.saturating_sub((-delta) as u16)
                    } else {
                        d.scroll.saturating_add(delta as u16)
                    };
                }
            }
            DetailFocus::Files => {
                let count = self
                    .detail()
                    .and_then(|d| d.gist_id.as_deref())
                    .map(|id| self.gist_filenames(id).len())
                    .unwrap_or(0);
                if count == 0 {
                    return;
                }
                let max = count - 1;
                if let Some(d) = self.detail_mut() {
                    let next = d.file_cursor as i64 + delta as i64;
                    d.file_cursor = next.clamp(0, max as i64) as usize;
                }
            }
        }
    }

    /// Number of navigation steps per mouse wheel tick. List/index screens move one row;
    /// content panes (Diff, Preview, Confirm, GistDetail) scroll three lines for faster
    /// panning. Help body also scrolls three; the Help topic index is a list (one row).
    fn wheel_step(&self) -> usize {
        match &self.screen {
            Screen::Config(_) => super::screens::config::wheel_step(),
            Screen::Preview(_) => super::screens::preview::wheel_step(),
            Screen::Diff(_) => super::screens::diff::wheel_step(),
            Screen::Confirm(_) => 3,
            // GistDetail: the comments body scrolls like content (3 lines); the file list
            // steps one file at a time.
            Screen::GistDetail(_)
                if self
                    .detail()
                    .is_some_and(|d| d.focus == DetailFocus::Comments) =>
            {
                3
            }
            Screen::Help(h) => super::screens::help::wheel_step(h),
            Screen::Palette(_) => 1,
            _ => 1, // List/Pins/Gists/Revisions/Help index/GistDetail Files
        }
    }

    /// Select the clicked list row on the current screen, focusing its pane/list. Returns
    /// `true` when a row was hit (so a double-click should "open" it). A click in a pane's
    /// blank area or border focuses it but selects nothing (returns `false`); a click off
    /// every list returns `false`.
    fn click_select(&mut self, col: u16, row: u16, layout: &MouseLayout) -> bool {
        match &mut self.screen {
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
            Screen::Gists(_) => {
                if let Some(hit) = layout.list {
                    if point_in(hit.rect, col, row) {
                        let count = self.visible_gist_groups().len();
                        if let Some(idx) = hit.index_at(row, count) {
                            if let Some(gm) = self.gist_manager_mut() {
                                gm.cursor.select(idx);
                                return true;
                            }
                        }
                    }
                }
                false
            }
            Screen::Pins(_) => {
                if let Some(hit) = layout.list {
                    if point_in(hit.rect, col, row) {
                        let count = self.visible_pin_indices().len();
                        if let Some(idx) = hit.index_at(row, count) {
                            if let Some(pins) = self.pins_mut() {
                                pins.cursor.select(idx);
                                return true;
                            }
                        }
                    }
                }
                false
            }
            Screen::Revisions(rev) => {
                if let Some(hit) = layout.list {
                    if point_in(hit.rect, col, row) {
                        let count = rev.entries.as_ref().map_or(0, |e| e.len());
                        if let Some(idx) = hit.index_at(row, count) {
                            rev.index = idx;
                            rev.hscroll = 0;
                            return true;
                        }
                    }
                }
                false
            }
            // Only set when the topic index is open (render_help), so this is a no-op while
            // viewing a topic's body.
            Screen::Help(help) => {
                if let Some(hit) = layout.list {
                    if point_in(hit.rect, col, row) {
                        if let Some(idx) = hit.index_at(row, HelpTopic::all().len()) {
                            help.index_sel = idx;
                            return true;
                        }
                    }
                }
                false
            }
            Screen::Config(cfg) => {
                if let Some(hit) = layout.list {
                    if point_in(hit.rect, col, row) {
                        if let Some(idx) = hit.index_at(row, ConfigField::ALL.len()) {
                            cfg.index = idx;
                            return true;
                        }
                    }
                }
                false
            }
            Screen::GistDetail(_) => {
                if let Some(hit) = layout.detail_files {
                    if point_in(hit.rect, col, row) {
                        // Clicking the file list focuses the Files tab; a row also moves the cursor.
                        let count = self
                            .detail()
                            .and_then(|d| d.gist_id.as_deref())
                            .map_or(0, |id| self.gist_filenames(id).len());
                        if let Some(d) = self.detail_mut() {
                            d.focus = DetailFocus::Files;
                        }
                        if let Some(idx) = hit.index_at(row, count) {
                            if let Some(d) = self.detail_mut() {
                                d.file_cursor = idx;
                            }
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
        match &self.screen {
            // GistDetail files have no `Enter`; they preview via number keys, so a
            // double-click previews the file under the cursor.
            Screen::GistDetail(_) => {
                let cursor = self.detail().map(|d| d.file_cursor).unwrap_or(0);
                self.preview_detail_file(cursor)
            }
            _ => self.handle_key_with(KeyCode::Enter, KeyModifiers::NONE),
        }
    }

    /// Preview the `index`-th file of the gist shown on `Screen::GistDetail` (full-screen),
    /// the action behind the `1`–`9` keys and a file double-click.
    fn preview_detail_file(&mut self, index: usize) -> KeyOutcome {
        if let Some(gist_id) = self.detail().and_then(|d| d.gist_id.clone()) {
            if let Some(filename) = self.gist_filenames(&gist_id).into_iter().nth(index) {
                if self.block_if_non_previewable_gist_file(&gist_id, &filename) {
                    return KeyOutcome::None;
                }
                self.pending_return = Some(self.park_gist_detail_screen());
                return KeyOutcome::PreviewContent {
                    file: crate::domain::GistFileRef::id_name(gist_id, filename),
                };
            }
        }
        KeyOutcome::None
    }

    fn revision_diff_incremental_intent(&mut self) -> KeyOutcome {
        let Some(rev) = self.revision() else {
            return KeyOutcome::None;
        };
        let Some(gist_id) = rev.gist_id.clone() else {
            return KeyOutcome::None;
        };
        let filename = rev.target_file.clone();
        let index = rev.index;
        let parent = rev
            .entries
            .as_ref()
            .and_then(|entries| entries.get(index + 1).cloned());
        let Some(child) = self.selected_revision().cloned() else {
            return KeyOutcome::None;
        };
        if self.block_if_non_previewable_gist_file(&gist_id, &filename) {
            return KeyOutcome::None;
        }
        let child_version = child.version.clone();
        let child_label = revision_version_label(&child);
        let (parent_version, old_label) = match parent {
            Some(parent) => {
                let label = revision_version_label(&parent);
                (Some(parent.version), format!("revision {label}"))
            }
            None => (None, "(initial)".into()),
        };
        let new_label = format!("revision {child_label}");
        let owner_login = self.gist_owner_login(&gist_id);
        KeyOutcome::RevisionDiffIncremental {
            gist_id,
            filename,
            child_version,
            parent_version,
            old_label,
            new_label,
            owner_login,
        }
    }

    fn revision_diff_intent(&mut self) -> KeyOutcome {
        let Some(rev) = self.revision() else {
            return KeyOutcome::None;
        };
        let Some(gist_id) = rev.gist_id.clone() else {
            return KeyOutcome::None;
        };
        let filename = rev.target_file.clone();
        let Some(revision) = self.selected_revision().cloned() else {
            return KeyOutcome::None;
        };
        if self.block_if_non_previewable_gist_file(&gist_id, &filename) {
            return KeyOutcome::None;
        }
        let version = revision.version.clone();
        let version_label = revision_version_label(&revision);
        let old_label = format!("revision {version_label}");
        let new_label = format!("current {filename}");
        let raw_url = self.gist_file_raw_url(&gist_id, &filename);
        let owner_login = self.gist_owner_login(&gist_id);
        KeyOutcome::RevisionDiff {
            gist_id,
            filename,
            version,
            old_label,
            new_label,
            raw_url,
            owner_login,
        }
    }

    fn restore_revision_preview_intent(&mut self) -> KeyOutcome {
        let Some(rev) = self.revision() else {
            return KeyOutcome::None;
        };
        let Some(gist_id) = rev.gist_id.clone() else {
            return KeyOutcome::None;
        };
        if !self.gist_is_owned(&gist_id) {
            return KeyOutcome::None;
        }
        let filename = rev.target_file.clone();
        let Some(revision) = self.selected_revision().cloned() else {
            return KeyOutcome::None;
        };
        let version = revision.version.clone();
        let version_label = revision_version_label(&revision);
        let raw_url = self.gist_file_raw_url(&gist_id, &filename);
        let owner_login = self.gist_owner_login(&gist_id);
        KeyOutcome::RestoreRevisionPreview {
            gist_id,
            filename,
            version,
            version_label,
            raw_url,
            owner_login,
        }
    }

    /// Switch the GistDetail tab if `col`/`row` lands on a tab header. Returns the outcome
    /// (possibly `FetchComments`) when a tab was clicked, else `None` to fall through.
    fn click_detail_tab(&mut self, col: u16, row: u16, layout: &MouseLayout) -> Option<KeyOutcome> {
        if !self.screen.is_gist_detail() {
            return None;
        }
        if let Some(rect) = layout.detail_tab_files {
            if point_in(rect, col, row) {
                if let Some(d) = self.detail_mut() {
                    d.focus = DetailFocus::Files;
                }
                return Some(KeyOutcome::None);
            }
        }
        if let Some(rect) = layout.detail_tab_comments {
            if point_in(rect, col, row) {
                let fetch = if let Some(d) = self.detail_mut() {
                    d.focus = DetailFocus::Comments;
                    d.comments.is_none() && !d.comments_loading
                } else {
                    false
                };
                if fetch {
                    if let Some(gist_id) = self.detail().and_then(|d| d.gist_id.clone()) {
                        return Some(KeyOutcome::FetchComments { gist_id });
                    }
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
        if !self.screen.is_gist_detail()
            || !self
                .detail()
                .is_some_and(|d| d.focus == DetailFocus::Comments)
        {
            return None;
        }
        let rect = layout.comments_load_older?;
        if point_in(rect, col, row) && self.can_load_older_comments() {
            if let Some(gist_id) = self.detail().and_then(|d| d.gist_id.clone()) {
                let page = self
                    .detail()
                    .map(|d| d.comments_loaded_oldest_page.saturating_sub(1))
                    .unwrap_or(0);
                if page > 0 {
                    return Some(KeyOutcome::LoadOlderComments { gist_id, page });
                }
            }
        }
        None
    }

    /// Translate a classified mouse intent into a state change, reusing existing keyboard
    /// logic. Pure (no IO, no clock); returns a `KeyOutcome` so `run_loop` can perform any
    /// follow-up IO (e.g. `PreviewDiff` on double-click).
    pub fn handle_mouse(&mut self, input: MouseInput, layout: &MouseLayout) -> KeyOutcome {
        if self.screen.is_palette() {
            return match input {
                MouseInput::Click { col, row } | MouseInput::DoubleClick { col, row } => {
                    self.palette_click(col, row, layout)
                }
                MouseInput::RightClick { .. } => KeyOutcome::None,
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
            };
        }
        match input {
            MouseInput::RightClick { col, row } => {
                if self.palette_blocked() {
                    return KeyOutcome::None;
                }
                self.click_select(col, row, layout);
                self.open_palette_menu(Some((col, row)));
                KeyOutcome::None
            }
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
                        return KeyOutcome::OpenRepoUrl {
                            url: env!("CARGO_PKG_REPOSITORY").to_string(),
                        };
                    }
                }
                // Top-bar (G)ists / (P)ins / (C)onfig / (?)Help — same effect as pressing
                // the key, from any screen (not just wherever that key happens to be bound).
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
                if let Some(rect) = layout.top_bar_config {
                    if point_in(rect, col, row) {
                        self.open_config();
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

// ── Palette/handler shared guards (issue #288) ──────────────────────────────────────────
//
// One `<screen>_guard(state, code) -> bool` per screen with non-trivial (data-dependent) key
// enablement. Each is the single "would this key actually do something" predicate, called
// both by that screen's `handle_key_*` match-arm guards below and by the matching
// `*_palette_items` builder in `palette.rs` — so the two can never silently drift again.
// Screens where every action is unconditionally enabled (Preview, Config, Help) have nothing
// to share and are left as-is.

/// Whether a diff between `gist_id`/`filename` and the (optional) local file at `local_path`
/// is previewable — the gist file is text and, when a local pairing exists, so is it.
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

pub(crate) fn pins_guard(state: &AppState, code: KeyCode) -> bool {
    let has_pin = !state.pinned.is_empty() && state.selected_pin_index().is_some();
    match code {
        KeyCode::Enter => {
            has_pin
                && state.selected_pin_index().is_some_and(|idx| {
                    let pin = &state.pinned[idx];
                    diff_pair_previewable(
                        state,
                        &pin.gist_id,
                        &pin.gist_filename,
                        Some(pin.local_path.as_path()),
                    )
                })
        }
        KeyCode::Char('x' | 's' | 'u' | 'd') => has_pin,
        _ => false,
    }
}

pub(crate) fn gists_guard(state: &AppState, code: KeyCode) -> bool {
    let has_sel = state.gist_manager().map(|g| g.cursor.index).unwrap_or(0)
        < state.visible_gist_groups().len();
    match code {
        KeyCode::Enter | KeyCode::Char('o' | 'y' | 'H' | '*') => has_sel,
        _ => false,
    }
}

pub(crate) fn detail_guard(state: &AppState, code: KeyCode) -> bool {
    let d = state.detail();
    let gist_id = d.and_then(|d| d.gist_id.clone());
    let owned = gist_id
        .as_deref()
        .map(|id| state.gist_is_owned(id))
        .unwrap_or(false);
    match code {
        KeyCode::Enter => {
            d.is_some_and(|d| d.focus == DetailFocus::Files)
                && gist_id.as_deref().is_some_and(|id| {
                    state
                        .gist_filenames(id)
                        .into_iter()
                        .nth(d.map(|d| d.file_cursor).unwrap_or(0))
                        .is_some_and(|name| state.gist_file_is_text_previewable(id, &name))
                })
        }
        KeyCode::Char('o' | 'y' | 'H' | '*') => gist_id.is_some(),
        KeyCode::Char('e' | 'c' | 'X') => owned,
        KeyCode::Char('F') => gist_id.is_some() && !owned,
        // Load older comments: needs both a page to load AND the Comments tab focused —
        // `can_load_older_comments` only checks the former (issue #288: previously the
        // palette enabled this even while the Files tab was focused, where `m` is a no-op).
        KeyCode::Char('m') => {
            d.is_some_and(|d| d.focus == DetailFocus::Comments) && state.can_load_older_comments()
        }
        _ => false,
    }
}

pub(crate) fn revisions_guard(state: &AppState, code: KeyCode) -> bool {
    let rev = state.revision();
    let entries_len = rev
        .and_then(|r| r.entries.as_ref().map(|e| e.len()))
        .unwrap_or(0);
    let has_entries = entries_len > 0;
    let not_head = rev.is_some_and(|r| r.index > 0);
    let gist_id = rev.and_then(|r| r.gist_id.clone());
    let owned = gist_id
        .as_deref()
        .map(|id| state.gist_is_owned(id))
        .unwrap_or(false);
    let file = rev.map(|r| r.target_file.clone()).unwrap_or_default();
    let previewable = gist_id
        .as_ref()
        .is_some_and(|id| state.gist_file_is_text_previewable(id, &file));
    match code {
        KeyCode::Enter => has_entries && previewable,
        KeyCode::Char('D') => has_entries && not_head && previewable,
        KeyCode::Char('r') => entries_len > 1 && not_head && owned,
        // Cycling the target file only needs the gist to have more than one file — it does
        // not depend on the revision list having loaded (`cycle_revision_target_file` never
        // checks `entries`). Issue #288: previously the palette gated this on `has_entries`
        // with no functional reason; unified on the handler's broader condition instead of
        // narrowing the handler to match the palette.
        KeyCode::Char('F') => gist_id
            .as_deref()
            .is_some_and(|id| state.gist_filenames(id).len() > 1),
        _ => false,
    }
}

pub(crate) fn list_guard(state: &AppState, code: KeyCode) -> bool {
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
    match code {
        KeyCode::Enter => gist_file.as_ref().is_some_and(|f| {
            let local_path = visible_locals
                .get(state.local_index)
                .map(|r| r.candidate.path.as_path());
            diff_pair_previewable(state, &f.gist_id, &f.filename, local_path)
        }),
        KeyCode::Char(' ') => gist_file
            .as_ref()
            .is_some_and(|f| state.gist_file_is_text_previewable(&f.gist_id, &f.filename)),
        KeyCode::Char('d') => has_gist && state.focus == FocusPane::Gist,
        KeyCode::Char('u') => has_local && has_gist && owned,
        KeyCode::Char('n') => has_local,
        // Pinning a *new* pair needs ownership (can't create a pin on a foreign gist);
        // toggling an already-pinned pair off never did. Issue #288: previously the palette
        // allowed 'p' on any local+gist pair regardless of ownership; `pin_toggle_intent`
        // silently no-ops (via `block_if_foreign_gist`) for a foreign, not-yet-pinned pair.
        KeyCode::Char('p') => has_local && has_gist && (pinned_pair || owned),
        // Issue #288: previously the palette additionally required `pinned_pair` here, but
        // `handle_key_list`'s real 'S' arm never has — a non-pinned pair is caught one layer
        // down in the IO dispatcher (`dispatch.rs`) with a "pair is not pinned" status
        // message, the same way pressing 'S' directly already behaves. Unified on the
        // handler's (looser, tested) condition instead of narrowing the handler to match the
        // palette, since the palette's extra restriction wasn't guarding against a real bug.
        KeyCode::Char('S') => has_local && has_gist,
        KeyCode::Char('g') => !state.gists.is_empty(),
        KeyCode::Char('X') => {
            has_gist
                && owned
                && gist_id
                    .as_deref()
                    .is_some_and(|id| state.gist_file_count(id) > 1)
        }
        KeyCode::Char('e') => has_local,
        KeyCode::Char('y' | '*') => state.context_gist_id().is_some(),
        KeyCode::Char('H') => has_gist,
        _ => false,
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
            // Not gated through `list_guard`: `star_toggle_intent` already has its own
            // complete "select a gist first" message for the no-selection case.
            KeyCode::Char('*') => return self.star_toggle_intent(),
            KeyCode::Char('s') => self.cycle_focused_sort(),
            KeyCode::Char('r') => {
                self.local_recursive = !self.local_recursive;
                self.local_index = 0;
                self.local_hscroll = 0;
                return KeyOutcome::RefreshLocals;
            }
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('y') => {
                let Some(gist_id) = self.context_gist_id() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::CopyGistUrl { gist_id };
            }
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char('P') => self.open_pins(),
            KeyCode::Char('C') => self.open_config(),
            // Not gated through `list_guard`: unlike the palette's "Smart-sync pinned pair"
            // item, this key isn't restricted to an already-pinned pair — the IO dispatcher
            // (`dispatch.rs`) checks pin membership downstream and reports "pair is not
            // pinned" there. `list_guard`'s `S` case (used by the palette) is stricter.
            KeyCode::Char('S') => {
                let (Some(local), Some(gist)) = (self.selected_local(), self.selected_gist())
                else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::SyncSelectedPair {
                    local_path: local.path.clone(),
                    gist_id: gist.file.gist_id.clone(),
                    filename: gist.file.filename.clone(),
                };
            }
            KeyCode::Char('g') => self.open_gist_manager(),
            KeyCode::Char('H') if list_guard(self, code) => {
                if self.open_revisions() {
                    if let Some(gist_id) = self.revision().and_then(|r| r.gist_id.clone()) {
                        return KeyOutcome::FetchRevisions { gist_id };
                    }
                }
            }
            KeyCode::Char('H') => {
                self.status = Some("select a gist file to view revision history".into());
            }
            KeyCode::Char('e') if list_guard(self, code) => {
                let (locals, _) = self.list_pane_snapshots();
                if let Some(local) = locals.get(self.local_index) {
                    return KeyOutcome::EditLocal {
                        path: local.candidate.path.clone(),
                    };
                }
            }
            KeyCode::Char('e') => {
                self.status = Some("select a local file to edit".into());
            }
            KeyCode::Char(' ') if list_guard(self, code) => {
                let (_, ranked) = self.list_pane_snapshots();
                let Some(gist) = ranked.get(self.gist_index) else {
                    return KeyOutcome::None;
                };
                self.pending_return = Some(Screen::List);
                return KeyOutcome::PreviewContent {
                    file: crate::domain::GistFileRef::new(
                        gist.file.gist_id.clone(),
                        gist.file.filename.clone(),
                        gist.file.raw_url.clone(),
                    ),
                };
            }
            // has_gist but non-previewable (`list_guard` above didn't match) — replay the
            // same check `PreviewContent` would use, so the user gets the precise
            // "cannot preview: …" message instead of a silent no-op.
            KeyCode::Char(' ') => {
                let (_, ranked) = self.list_pane_snapshots();
                if let Some(gist) = ranked.get(self.gist_index) {
                    self.block_if_non_previewable_gist_file(
                        &gist.file.gist_id,
                        &gist.file.filename,
                    );
                }
            }
            KeyCode::Char('d') if list_guard(self, code) => {
                let (_, ranked) = self.list_pane_snapshots();
                if let Some(gist) = ranked.get(self.gist_index) {
                    let filename = gist.file.filename.clone();
                    return KeyOutcome::DownloadGist {
                        file: crate::domain::GistFileRef::new(
                            gist.file.gist_id.clone(),
                            filename.clone(),
                            gist.file.raw_url.clone(),
                        ),
                        target: self.cwd.join(&filename),
                    };
                }
            }
            // Enter works from either pane: it diffs the selected local file against the
            // selected gist (the top match when focus is on the local pane). Snapshot both
            // ranked lists once (issue #224 / #154 shape #1).
            KeyCode::Enter if list_guard(self, code) => {
                let (locals, ranked) = self.list_pane_snapshots();
                let Some(gist) = ranked.get(self.gist_index) else {
                    return KeyOutcome::None;
                };
                let local_path = locals
                    .get(self.local_index)
                    .map(|r| r.candidate.path.clone());
                let filename = gist.file.filename.clone();
                return KeyOutcome::PreviewDiff {
                    local_path,
                    file: crate::domain::GistFileRef::new(
                        gist.file.gist_id.clone(),
                        filename.clone(),
                        gist.file.raw_url.clone(),
                    ),
                    target: self.cwd.join(&filename),
                    upload_orientation: self.focus == FocusPane::Local,
                };
            }
            // has_gist but non-diffable (`list_guard` above didn't match) — replay the same
            // check `PreviewDiff` would use, so the user gets the precise "cannot preview: …"
            // message instead of a silent no-op.
            KeyCode::Enter => {
                let (locals, ranked) = self.list_pane_snapshots();
                if let Some(gist) = ranked.get(self.gist_index) {
                    let local_path = locals
                        .get(self.local_index)
                        .map(|r| r.candidate.path.clone());
                    self.block_if_non_previewable_diff(
                        &gist.file.gist_id,
                        &gist.file.filename,
                        local_path.as_deref(),
                    );
                }
            }
            // Not gated through `list_guard`: `pin_toggle_intent` / `upload_intent` /
            // `remove_gist_file_intent` / `create_gist_intent` already have their own complete
            // messages for every disabled case (no selection, foreign gist, single-file gist…).
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
        // One snapshot for the focused pane length (issue #224) — selection-index
        // changes do not alter list length, so this is safe.
        let (locals, ranked) = self.list_pane_snapshots();
        match self.focus {
            FocusPane::Local => {
                let len = locals.len();
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
                let len = ranked.len();
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
        // Length-only snapshot once per move (issue #224).
        let (locals, ranked) = self.list_pane_snapshots();
        match self.focus {
            FocusPane::Local => {
                let len = locals.len();
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
                let len = ranked.len();
                if forward {
                    if self.gist_index + 1 >= len {
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
    pub(crate) fn open_gist_manager(&mut self) {
        if self.gists.is_empty() {
            self.status = Some("no gists to manage".into());
            return;
        }
        self.editing_description = false;
        self.description_input.clear();
        let target = self.selected_gist().map(|g| g.file.gist_id);
        // Clean payload (default filters) so the target is always visible.
        self.screen = Screen::Gists(Box::default());
        let groups = self.visible_gist_groups();
        if let Some(gm) = self.gist_manager_mut() {
            let idx = target
                .and_then(|id| groups.iter().position(|g| g.id == id))
                .unwrap_or(0);
            gm.cursor.select(idx);
        }
    }

    /// Open the Pins view (`Screen::Pins`), resetting its selection/scroll so a stale
    /// filtered-in position from a previous visit never lingers.
    pub(crate) fn open_pins(&mut self) {
        self.screen = Screen::Pins(Box::new(PinsState {
            cursor: ListCursor::default(),
            ..PinsState::default()
        }));
    }

    /// Pin/unpin the selected local↔gist pair: returns [`KeyOutcome::Unpin`] when the exact
    /// pair is already pinned, otherwise [`KeyOutcome::Pin`]. Requires a selection in both
    /// panes; otherwise it just sets a status hint.
    fn pin_toggle_intent(&mut self) -> KeyOutcome {
        let (locals, ranked) = self.list_pane_snapshots();
        let (Some(local), Some(gist)) = (
            locals.get(self.local_index).map(|r| &r.candidate),
            ranked.get(self.gist_index),
        ) else {
            self.status = Some("select a local file and a gist to pin".into());
            return KeyOutcome::None;
        };
        let local_path = local.path.clone();
        let gist_id = gist.file.gist_id.clone();
        let filename = gist.file.filename.clone();
        let already = self.pinned.iter().any(|m| {
            m.local_path == local_path && m.gist_id == gist_id && m.gist_filename == filename
        });
        if already {
            KeyOutcome::Unpin {
                local_path,
                gist_id,
                filename,
            }
        } else if self.block_if_foreign_gist(&gist_id, true) {
            KeyOutcome::None
        } else {
            KeyOutcome::Pin {
                local_path,
                gist_id,
                filename,
            }
        }
    }

    fn star_toggle_intent(&mut self) -> KeyOutcome {
        let Some(gist_id) = self.context_gist_id() else {
            self.set_status("select a gist first");
            return KeyOutcome::None;
        };
        let starring = !self.gist_is_starred(&gist_id);
        KeyOutcome::ToggleGistStar { gist_id, starring }
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
            KeyOutcome::ForkGist { gist_id }
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
        self.pending_return = Some(Screen::List);
        let text = format!(
            "Remove file \"{filename}\" from gist {gist_id} ({label}).\n\nThe other files in this gist are kept. This cannot be undone."
        );
        self.enter_confirm(
            PendingAction::RemoveFile {
                gist_id,
                filename,
                label,
            },
            text,
        );
    }

    /// Stage creation of a new gist from the selected local file. Create is a two-step confirm:
    /// type an optional description (inline editor, shared with the gist-level view), then
    /// choose visibility. Requires a selected local file.
    fn create_gist_intent(&mut self) {
        let Some(local) = self.selected_local() else {
            self.status = Some("select a local file to create a gist".into());
            return;
        };
        self.editing_description = true;
        self.description_input.clear();
        self.pending_return = Some(Screen::List);
        self.enter_confirm(
            PendingAction::Create {
                local_path: local.path.clone(),
            },
            format!(
                "Create a new gist from {}.\n\nType an optional description, then choose visibility.",
                local.path.display()
            ),
        );
    }

    fn handle_key_confirm(&mut self, code: KeyCode) -> KeyOutcome {
        // While typing the create flow's description, arrows drive the text cursor (handled
        // below), not the background diff scroll.
        match self.pending_action().cloned() {
            Some(PendingAction::Download) => match code {
                KeyCode::Char('y') => {
                    return KeyOutcome::Download {
                        mode: crate::actions::DownloadMode::overwrite_after_user_confirm(),
                    };
                }
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.cancel_confirm_to_diff();
                }
                _ => {}
            },
            Some(PendingAction::Upload { ref local_path, .. }) => match code {
                KeyCode::Char('y') if self.upload.watching => {
                    self.set_status("editor still open — finish editing first");
                }
                KeyCode::Char('y') => return KeyOutcome::Upload,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    // Return to wherever the upload was initiated from (List, or Pins for
                    // a pin push) instead of always snapping back to List.
                    self.cancel_confirm();
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
                    self.cancel_confirm();
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
                    self.cancel_confirm();
                }
                _ => {}
            },
            Some(PendingAction::RestoreRevision { .. }) => match code {
                KeyCode::Char('y') => return KeyOutcome::ExecuteRestoreRevision,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.cancel_confirm();
                    if !self.screen.is_revisions() {
                        self.screen = Screen::Revisions(Box::default());
                    }
                }
                _ => {}
            },
            _ => {
                if matches!(code, KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q')) {
                    self.cancel_confirm();
                }
            }
        }
        KeyOutcome::None
    }
}

/// Whether a column/row position lands inside a `Rect`.
pub(crate) fn point_in(rect: ratatui::layout::Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
}
