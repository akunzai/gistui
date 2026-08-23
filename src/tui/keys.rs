use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

/// Vim-style navigation alias alongside arrow / page keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NavAction {
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
pub(crate) const PAGE_SCROLL: u16 = 10;

/// Dispatch [`NavAction`] onto a Pins/Gists [`ListCursor`] (step from [`PAGE_SCROLL`]).
pub(crate) fn apply_list_cursor_nav(
    cursor: &mut ListCursor,
    action: NavAction,
    len: usize,
    hmax: u16,
) {
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
        // A keystroke means the hand left the mouse. Ending the drag here is what keeps the
        // flag from wedging when a key takes the mouse away before the release lands —
        // turning mouse support off in Settings, or suspending the TUI for $EDITOR (#395).
        self.mouse_session.interrupt();
        if code == KeyCode::Char(';')
            && modifiers.is_empty()
            && !self.screen.is_palette()
            && !self.palette_blocked()
        {
            self.open_palette_menu(None);
            return KeyOutcome::None;
        }
        if code == KeyCode::Char('p')
            && modifiers.contains(KeyModifiers::CONTROL)
            && !self.screen.is_palette()
            && !self.palette_blocked()
        {
            self.open_palette_command();
            return KeyOutcome::None;
        }
        // Global theme toggle: skip while any inline text input is active so `T` can still
        // be typed into filters and description editors.
        if code == KeyCode::Char('T')
            && theme_toggle_modifiers_ok(modifiers)
            && !self.is_any_filtering()
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
        (super::screens::lookup(&self.screen).handle_key)(self, code, modifiers)
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
            ConfigField::DiffShowFull => if self.diff_show_full { "on" } else { "off" }.into(),
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
        if self.is_any_filtering() && !matches!(action, NavAction::PageUp | NavAction::PageDown) {
            return false;
        }
        (super::screens::lookup(&self.screen).apply_navigation)(self, action)
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

    /// Number of navigation steps per mouse wheel tick. List/index screens move one row;
    /// content panes (Diff, Preview, Confirm, GistDetail) scroll three lines for faster
    /// panning. Help body also scrolls three; the Help topic index is a list (one row).
    fn wheel_step(&self) -> usize {
        (super::screens::lookup(&self.screen).wheel_step)(self)
    }

    /// Select the clicked list row on the current screen, focusing its pane/list. Returns
    /// `true` when a row was hit (so a double-click should "open" it). A click in a pane's
    /// blank area or border focuses it but selects nothing (returns `false`); a click off
    /// every list returns `false`.
    fn click_select(&mut self, col: u16, row: u16, layout: &MouseFrame) -> bool {
        (super::screens::lookup(&self.screen).click_select)(self, col, row, layout)
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

    /// Translate a classified mouse intent into a state change, reusing existing keyboard
    /// logic. Pure (no IO, no clock); returns a `KeyOutcome` so `run_loop` can perform any
    /// follow-up IO (e.g. `PreviewDiff` on double-click).
    pub fn handle_mouse(&mut self, input: MouseInput, layout: &MouseFrame) -> KeyOutcome {
        if input == MouseInput::Release {
            self.mouse_session.interrupt();
            return KeyOutcome::None;
        }
        if matches!(input, MouseInput::RightClick { .. }) {
            self.mouse_session.interrupt();
        }
        if self.screen.is_palette() {
            return match input {
                MouseInput::Click { col, row } | MouseInput::DoubleClick { col, row } => {
                    self.palette_click(col, row, layout)
                }
                MouseInput::RightClick { .. } => KeyOutcome::None,
                // The palette overlays the panes, so a drag underneath it resizes nothing.
                // `Release` never reaches here — it returns at the top of the function.
                MouseInput::Drag { .. } | MouseInput::Release => KeyOutcome::None,
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
            // Divider geometry is only registered on the List screen, so this is a no-op
            // everywhere else. `Release` was handled above.
            MouseInput::Drag { col } => {
                self.drag_split_divider(col, layout);
                KeyOutcome::None
            }
            MouseInput::Release => KeyOutcome::None,
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
                if let Some(outcome) = self.activate_global_mouse_target(col, row, layout) {
                    return outcome;
                }
                if self.grab_split_divider(col, row, layout) {
                    return KeyOutcome::None;
                }
                self.click_select(col, row, layout);
                KeyOutcome::None
            }
            MouseInput::DoubleClick { col, row } => {
                if let Some(outcome) = self.activate_global_mouse_target(col, row, layout) {
                    return outcome;
                }
                if self.reset_split_divider(col, row, layout) {
                    return KeyOutcome::None;
                }
                if self.click_select(col, row, layout) {
                    // Selection landed on a row — open/activate it.
                    return self.activate_selected();
                }
                KeyOutcome::None
            }
        }
    }

    fn activate_global_mouse_target(
        &mut self,
        col: u16,
        row: u16,
        layout: &MouseFrame,
    ) -> Option<KeyOutcome> {
        match layout.resolve(col, row)? {
            HitTarget::Close => Some(self.handle_key_with(KeyCode::Esc, KeyModifiers::NONE)),
            HitTarget::Repo => Some(KeyOutcome::OpenRepoUrl {
                url: env!("CARGO_PKG_REPOSITORY").to_string(),
            }),
            HitTarget::TopGists => {
                self.open_gist_manager();
                Some(KeyOutcome::None)
            }
            HitTarget::TopPins => {
                self.open_pins();
                Some(KeyOutcome::None)
            }
            HitTarget::TopConfig => {
                self.open_config();
                Some(KeyOutcome::None)
            }
            HitTarget::TopHelp => {
                self.open_help();
                Some(KeyOutcome::None)
            }
            HitTarget::DetailFilesTab | HitTarget::DetailCommentsTab => {
                self.click_detail_tab(col, row, layout)
            }
            HitTarget::CommentsLoadOlder => self.click_comments_load_older(col, row, layout),
            HitTarget::PaletteClose | HitTarget::Divider(_) | HitTarget::Row(_) => None,
        }
    }
}

/// Outcome of applying one key to a filter query's text (the shared edit transitions
/// for every inline filter input). Nav keys (Up/Down) and Tab are handled by the caller.
pub(crate) enum FilterKey {
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
pub(crate) fn apply_filter_edit(code: KeyCode, query: &mut TextInput) -> FilterKey {
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

/// Common keyboard navigation helper for single-pane inline text filters (`Pins` and `Gists`).
pub(crate) fn handle_inline_filter_key(
    code: KeyCode,
    cursor: &mut crate::tui::list_cursor::ListCursor,
    filter_query: &mut TextInput,
    filtering: &mut bool,
    visible_len: usize,
) {
    match code {
        KeyCode::Up => cursor.up(),
        KeyCode::Down => cursor.down(visible_len),
        _ => match apply_filter_edit(code, filter_query) {
            FilterKey::Edited => cursor.reset(),
            FilterKey::Cleared => {
                *filtering = false;
                cursor.reset();
            }
            FilterKey::Exited => *filtering = false,
            FilterKey::Moved | FilterKey::Pass => {}
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
pub(crate) fn diff_pair_previewable(
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

impl AppState {
    /// Page the focused list-pane selection by [`PAGE_SCROLL`] rows (clamped at bounds).
    pub(crate) fn list_page_focused(&mut self, forward: bool) {
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
    pub(crate) fn list_move_focused(&mut self, forward: bool) {
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
    pub(crate) fn cycle_focused_sort(&mut self) {
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

    pub(crate) fn star_toggle_intent(&mut self) -> KeyOutcome {
        let Some(gist_id) = self.context_gist_id() else {
            self.set_status("select a gist first");
            return KeyOutcome::None;
        };
        let starring = !self.gist_is_starred(&gist_id);
        KeyOutcome::ToggleGistStar { gist_id, starring }
    }
}

/// Whether a column/row position lands inside a `Rect`.
pub(crate) fn point_in(rect: ratatui::layout::Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{
        detail_mut, pins_ref, pins_state_with_long_home_path, set_diff_body, set_diff_scroll,
        set_pending, state_with_gists, state_with_selection, state_with_two_gists,
    };

    use crossterm::event::KeyCode;

    use std::path::PathBuf;

    #[test]
    fn q_in_list_quits_on_second_press() {
        let mut state = initial_state();
        // First press only arms the quit (and surfaces a hint); it must not exit.
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
        assert!(state.quit_armed);
        // Issue #346: the hint must name every key that confirms the quit, not just `q`.
        assert_eq!(
            state.status.as_deref(),
            Some("Press q or Esc again to quit (any other key cancels)")
        );
        // Second press confirms.
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::Quit);
    }

    #[test]
    fn esc_in_list_quits_on_second_press() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::Quit);
    }

    #[test]
    fn any_key_cancels_a_pending_quit() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
        assert!(state.quit_armed);
        // A non-quit key disarms; the next q then needs two presses again.
        state.handle_key(KeyCode::Tab);
        assert!(!state.quit_armed);
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
    }

    #[test]
    fn q_in_confirm_cancels_without_quitting() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        set_pending(&mut state, PendingAction::Download);
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
        assert!(state.screen.is_diff());
    }

    #[test]
    fn y_copies_gist_url_on_list_gists_and_detail() {
        let mut list = state_with_two_gists();
        assert!(matches!(
            list.handle_key(KeyCode::Char('y')),
            KeyOutcome::CopyGistUrl { .. }
        ));

        let mut gists = state_with_two_gists();
        gists.screen = Screen::Gists(Box::default());
        assert!(matches!(
            gists.handle_key(KeyCode::Char('y')),
            KeyOutcome::CopyGistUrl { .. }
        ));

        let mut detail = state_with_gists();
        detail.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut detail).gist_id = Some("g1".into());
        assert!(matches!(
            detail.handle_key(KeyCode::Char('y')),
            KeyOutcome::CopyGistUrl { .. }
        ));
    }

    #[test]
    fn top_bar_pins_click_opens_pins_from_any_screen() {
        let mut state = pins_state_with_long_home_path();
        state.handle_key(KeyCode::Right); // dirty the hscroll so the reset is observable
        assert!(pins_ref(&state).cursor.hscroll > 0);
        state.screen = Screen::Preview(Box::default());
        let mut layout = MouseFrame::default();
        layout.register(HitTarget::TopPins, Rect::new(20, 0, 6, 1));
        let out = state.handle_mouse(MouseInput::Click { col: 22, row: 0 }, &layout);
        assert!(state.screen.is_pins());
        assert_eq!(pins_ref(&state).cursor.hscroll, 0);
        assert_eq!(out, KeyOutcome::None);
    }

    #[test]
    fn double_click_uses_the_same_global_priority_as_click() {
        let mut state = initial_state();
        state.screen = Screen::Preview(Box::default());
        let mut layout = MouseFrame::default();
        let rect = Rect::new(20, 0, 6, 1);
        layout.register_pane(PaneTarget::List, PaneHit { rect, offset: 0 }, 1);
        layout.register(HitTarget::TopPins, rect);

        let out = state.handle_mouse(MouseInput::DoubleClick { col: 22, row: 0 }, &layout);

        assert_eq!(out, KeyOutcome::None);
        assert!(state.screen.is_pins());
    }

    #[test]
    fn top_bar_help_click_while_already_on_help_does_not_trap_keyboard_exit() {
        let mut state = state_with_gists();
        state.screen = Screen::Preview(Box::default());
        let mut layout = MouseFrame::default();
        layout.register(HitTarget::TopHelp, Rect::new(30, 0, 7, 1));
        // First click opens Help from Preview, remembering Preview as the return screen.
        state.handle_mouse(MouseInput::Click { col: 32, row: 0 }, &layout);
        assert!(state.screen.is_help());
        assert!(state.nav_stack.last().is_some_and(Screen::is_preview));

        // A second click on the same top-bar Help hotspot, now that Help is already open, must
        // be a no-op — it must not overwrite return_screen with Screen::Help, which would trap
        // Esc/`?`/the close button in Help with no keyboard way out.
        let out = state.handle_mouse(MouseInput::Click { col: 32, row: 0 }, &layout);
        assert!(state.screen.is_help());
        assert!(state.nav_stack.last().is_some_and(Screen::is_preview));
        assert_eq!(out, KeyOutcome::None);

        // Esc must still return to the real origin screen, not stay stuck on Help.
        state.handle_key(KeyCode::Esc);
        assert!(state.screen.is_preview());
    }

    #[test]
    fn shift_t_toggles_theme() {
        use crossterm::event::KeyModifiers;
        let mut state = initial_state();
        assert_eq!(state.theme_choice, crate::config::ThemeChoice::Dark);
        let outcome = state.handle_key_with(KeyCode::Char('T'), KeyModifiers::SHIFT);
        assert_eq!(outcome, KeyOutcome::ThemeToggle);
        assert_eq!(state.theme_choice, crate::config::ThemeChoice::Light);
    }

    #[test]
    fn repo_link_click_opens_repo_url_regardless_of_which_screen_set_the_rect() {
        let mut state = initial_state();
        let mut layout = MouseFrame::default();
        layout.register(HitTarget::Repo, Rect::new(5, 10, 20, 1));
        let out = state.handle_mouse(MouseInput::Click { col: 10, row: 10 }, &layout);
        assert!(matches!(out, KeyOutcome::OpenRepoUrl { .. }));
    }

    #[test]
    fn scroll_down_moves_content_three_lines() {
        // Set up a Diff screen with enough lines that wheel-down can reach 3.
        let mut state = state_with_selection();
        state.enter_diff(
            "line1\nline2\nline3\nline4\nline5".into(),
            "remote".into(),
            std::path::PathBuf::from("/tmp/x"),
            std::path::PathBuf::from("/tmp/cwd/x"),
        );
        assert!(state.screen.is_diff());
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").scroll, 0);
        state.handle_mouse(MouseInput::ScrollDown, &MouseFrame::default());
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").scroll, 3);
    }

    #[test]
    fn scroll_up_moves_content_three_lines() {
        let mut state = state_with_selection();
        state.enter_diff(
            "line1\nline2\nline3\nline4\nline5".into(),
            "remote".into(),
            std::path::PathBuf::from("/tmp/x"),
            std::path::PathBuf::from("/tmp/cwd/x"),
        );
        set_diff_scroll(&mut state, 3);
        state.handle_mouse(MouseInput::ScrollUp, &MouseFrame::default());
        assert_eq!(state.scroll_body().expect("Diff ScrollBody").scroll, 0);
    }

    #[test]
    fn close_button_click_returns_from_help() {
        let mut state = state_with_gists();
        // Simulate entering Help (mirrors what open_help() does).
        state.screen = Screen::Help(Box::default());
        let mut layout = MouseFrame::default();
        layout.register(HitTarget::Close, Rect::new(36, 0, 5, 1));
        let out = state.handle_mouse(MouseInput::Click { col: 38, row: 0 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn close_button_click_confirm_cancel_clears_pending() {
        // Close button on Screen::Confirm(Box::default()) dispatches Esc, which cancels the pending action.
        // Using PendingAction::Download: Esc sets pending_action = None and screen = Screen::Diff(Box::default()).
        let mut state = state_with_gists();
        set_diff_body(&mut state, "line1\nline2\nline3");
        set_pending(&mut state, PendingAction::Download);
        let mut layout = MouseFrame::default();
        layout.register(HitTarget::Close, Rect::new(36, 0, 5, 1));
        let out = state.handle_mouse(MouseInput::Click { col: 38, row: 0 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert!(state.pending_action().is_none());
        assert!(state.screen.is_diff());
    }

    #[test]
    fn close_button_click_create_description_cancels_not_types() {
        // Regression: close button while editing the create-description sub-state must cancel
        // (Esc), NOT append 'n' to the description field.  This test fails against the old
        // `KeyCode::Char('n')` dispatch and passes with `KeyCode::Esc`.
        let mut state = initial_state();
        state.screen = Screen::Confirm(Box::default());
        set_pending(
            &mut state,
            PendingAction::Create {
                local_path: std::path::PathBuf::from("notes.txt"),
            },
        );
        state.editing_description = true;
        // Pre-fill description so we can assert it was cleared (not grown by a typed 'n').
        state.description_input = "my desc".into();
        let mut layout = MouseFrame::default();
        layout.register(HitTarget::Close, Rect::new(36, 0, 5, 1));
        state.handle_mouse(MouseInput::Click { col: 38, row: 0 }, &layout);
        // Esc on create-description clears description, exits editing, and calls back_to_list.
        assert!(
            !state.editing_description,
            "editing_description must be false after close"
        );
        assert!(
            state.description_input.is_empty(),
            "description must be cleared, not have 'n' appended"
        );
        assert_eq!(state.screen, Screen::List);
        assert!(state.pending_action().is_none());
    }

    #[test]
    fn right_click_opens_menu_palette() {
        let mut state = crate::tui::initial_state();
        let out = state.handle_mouse(
            MouseInput::RightClick { col: 10, row: 5 },
            &MouseFrame::default(),
        );
        assert_eq!(out, KeyOutcome::None);
        assert!(state.screen.is_palette());
        assert_eq!(state.palette().unwrap().anchor, Some((10, 5)));
    }

    /// #395: the palette overlays the panes, so a drag under it must not resize — and the
    /// drag must not outlive the overlay either, or every later press-and-move resizes.
    #[test]
    fn an_overlay_never_leaves_a_divider_drag_running() {
        let mut state = state_with_gists();
        let mut layout = MouseFrame::default();
        let split = crate::tui::SplitHit {
            area: Rect::new(0, 1, 100, 20),
            divider_x: 39,
        };
        layout.register(HitTarget::Divider(split), split.area);
        state.handle_mouse(MouseInput::Click { col: 39, row: 5 }, &layout);
        assert!(state.mouse_session.is_dragging());

        // Opening the context menu ends the resize, so the divider is not left accented
        // under an overlay where dragging does nothing.
        state.handle_mouse(MouseInput::RightClick { col: 39, row: 5 }, &layout);
        assert!(state.screen.is_palette());
        assert!(!state.mouse_session.is_dragging());

        // And a release that arrives while the overlay is up still clears the flag, whatever
        // put it there — otherwise every later press-and-move would resize without a grab.
        state.mouse_session.begin_divider_drag();
        state.handle_mouse(MouseInput::Drag { col: 70 }, &layout);
        assert_eq!(state.list_split_percent, 40, "resized under the palette");
        state.handle_mouse(MouseInput::Release, &layout);
        assert!(
            !state.mouse_session.is_dragging(),
            "release under the palette left the drag stuck"
        );
    }

    #[test]
    fn right_click_interrupts_a_drag_even_when_palette_is_already_open() {
        let mut state = state_with_gists();
        state.open_palette_menu(None);
        state.mouse_session.begin_divider_drag();

        let out = state.handle_mouse(
            MouseInput::RightClick { col: 1, row: 1 },
            &MouseFrame::default(),
        );

        assert_eq!(out, KeyOutcome::None);
        assert!(!state.mouse_session.is_dragging());
    }

    /// #395: a keystroke means the hand left the mouse, and some keys take the mouse away
    /// entirely (turning mouse support off in Settings, suspending the TUI for `$EDITOR`),
    /// swallowing the release that would otherwise end the drag.
    #[test]
    fn a_keystroke_ends_a_divider_drag() {
        let mut state = state_with_gists();
        let mut layout = MouseFrame::default();
        let split = crate::tui::SplitHit {
            area: Rect::new(0, 1, 100, 20),
            divider_x: 39,
        };
        layout.register(HitTarget::Divider(split), split.area);
        state.handle_mouse(MouseInput::Click { col: 39, row: 5 }, &layout);
        assert!(state.mouse_session.is_dragging());
        state.handle_key(KeyCode::Tab);
        assert!(!state.mouse_session.is_dragging());
    }

    /// #395: a background-task overlay takes over the mouse and would swallow the release,
    /// so the drag ends as soon as the overlay appears.
    #[test]
    fn a_background_task_ends_a_divider_drag() {
        let mut state = state_with_gists();
        let mut layout = MouseFrame::default();
        let split = crate::tui::SplitHit {
            area: Rect::new(0, 1, 100, 20),
            divider_x: 39,
        };
        layout.register(HitTarget::Divider(split), split.area);
        state.handle_mouse(MouseInput::Click { col: 39, row: 5 }, &layout);
        assert!(state.mouse_session.is_dragging());
        state.begin_bg_task();
        assert!(!state.mouse_session.is_dragging());
    }

    #[test]
    fn release_without_a_drag_is_a_no_op_on_any_screen() {
        let mut state = state_with_gists();
        state.screen = Screen::Preview(Box::default());
        assert_eq!(
            state.handle_mouse(MouseInput::Release, &MouseFrame::default()),
            KeyOutcome::None
        );
        assert!(!state.mouse_session.is_dragging());
    }
}
