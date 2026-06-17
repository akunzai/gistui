use super::*;
use crossterm::event::KeyCode;

/// Lines moved per PageUp/PageDown in the scrollable views. Matches the gist-detail paging step
/// (`detail_nav(10)`); `handle_key` is pure and cannot read the viewport height, so a fixed step
/// keeps paging predictable without threading terminal size into the key logic.
const PAGE_SCROLL: u16 = 10;

impl AppState {
    pub fn handle_key(&mut self, code: KeyCode) -> KeyOutcome {
        // Global theme toggle: skip while any inline text input is active so `T` can still
        // be typed into filters and description editors.
        if code == KeyCode::Char('T')
            && !self.filtering
            && !self.pins_filtering
            && !self.gists_filtering
            && !self.editing_description
        {
            self.theme_choice = match self.theme_choice {
                crate::config::ThemeChoice::Dark => crate::config::ThemeChoice::Light,
                crate::config::ThemeChoice::Light => crate::config::ThemeChoice::Dark,
            };
            self.theme = Theme::for_choice(self.theme_choice);
            return KeyOutcome::ThemeToggle;
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
        }
    }

    /// Open the Help screen on the topic for the current screen, remembering where to return.
    fn open_help(&mut self) {
        self.help_return = self.screen;
        self.help_topic = HelpTopic::for_screen(self.screen);
        self.help_index_open = false;
        self.help_scroll = 0;
        self.screen = Screen::Help;
    }

    fn handle_key_help(&mut self, code: KeyCode) -> KeyOutcome {
        let topics = HelpTopic::all();
        if self.help_index_open {
            match code {
                KeyCode::Up => self.help_index_sel = self.help_index_sel.saturating_sub(1),
                KeyCode::Down => {
                    if self.help_index_sel + 1 < topics.len() {
                        self.help_index_sel += 1;
                    }
                }
                KeyCode::Enter => {
                    self.help_topic = topics[self.help_index_sel];
                    self.help_index_open = false;
                    self.help_scroll = 0;
                }
                KeyCode::Char(c @ '1'..='8') => {
                    self.help_topic = topics[(c as u8 - b'1') as usize];
                    self.help_index_open = false;
                    self.help_scroll = 0;
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.screen = self.help_return;
                    self.help_index_open = false;
                    self.help_scroll = 0;
                }
                _ => {}
            }
        } else {
            match code {
                KeyCode::Down => self.help_scroll = self.help_scroll.saturating_add(1),
                KeyCode::Up => self.help_scroll = self.help_scroll.saturating_sub(1),
                KeyCode::Tab => {
                    self.help_index_sel = topics
                        .iter()
                        .position(|&t| t == self.help_topic)
                        .unwrap_or(0);
                    self.help_index_open = true;
                }
                KeyCode::Char(c @ '1'..='8') => {
                    self.help_topic = topics[(c as u8 - b'1') as usize];
                    self.help_scroll = 0;
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.screen = self.help_return;
                    self.help_scroll = 0;
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
        if self.pins_filtering {
            match code {
                KeyCode::Up if self.pins_index > 0 => {
                    self.pins_index -= 1;
                    self.pins_hscroll = 0;
                }
                KeyCode::Up => {}
                KeyCode::Down => {
                    if self.pins_index + 1 < self.visible_pin_indices().len() {
                        self.pins_index += 1;
                        self.pins_hscroll = 0;
                    }
                }
                _ => match apply_filter_edit(code, &mut self.pins_filter_query) {
                    FilterKey::Edited => {
                        self.pins_index = 0;
                        self.pins_hscroll = 0;
                    }
                    FilterKey::Cleared => {
                        self.pins_filtering = false;
                        self.pins_index = 0;
                        self.pins_hscroll = 0;
                    }
                    FilterKey::Exited => self.pins_filtering = false,
                    FilterKey::Moved | FilterKey::Pass => {}
                },
            }
            return KeyOutcome::None;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.screen = Screen::List,
            KeyCode::Down if self.pins_index + 1 < self.visible_pin_indices().len() => {
                self.pins_index += 1;
                self.pins_hscroll = 0;
            }
            KeyCode::Up if self.pins_index > 0 => {
                self.pins_index -= 1;
                self.pins_hscroll = 0;
            }
            KeyCode::Right => {
                self.pins_hscroll = (self.pins_hscroll + 1).min(self.pins_hscroll_max());
            }
            KeyCode::Left => {
                self.pins_hscroll = self.pins_hscroll.saturating_sub(1);
            }
            KeyCode::Char('/') => self.pins_filtering = true,
            KeyCode::Enter if !self.pinned.is_empty() => return KeyOutcome::PreviewPinDiff,
            KeyCode::Char('x') if !self.pinned.is_empty() => return KeyOutcome::UnpinAtPin,
            KeyCode::Char('s') if !self.pinned.is_empty() => return KeyOutcome::SyncPinAuto,
            KeyCode::Char('u') if !self.pinned.is_empty() => return KeyOutcome::SyncPinPush,
            KeyCode::Char('d') if !self.pinned.is_empty() => return KeyOutcome::SyncPinPull,
            KeyCode::Char('o') => {
                self.pins_sort = self.pins_sort.next();
                self.pins_index = 0;
                self.pins_hscroll = 0;
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }

    fn handle_key_gists(&mut self, code: KeyCode) -> KeyOutcome {
        self.status = None;
        // Inline description editor: capture text until Enter (apply) or Esc (cancel).
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
        // Inline text filter: live-navigate with arrows; Tab is a no-op (single pane).
        if self.gists_filtering {
            match code {
                KeyCode::Up if self.gists_index > 0 => {
                    self.gists_index -= 1;
                    self.gists_hscroll = 0;
                }
                KeyCode::Up => {}
                KeyCode::Down => {
                    if self.gists_index + 1 < self.visible_gist_groups().len() {
                        self.gists_index += 1;
                        self.gists_hscroll = 0;
                    }
                }
                _ => match apply_filter_edit(code, &mut self.gists_filter_query) {
                    FilterKey::Edited => {
                        self.gists_index = 0;
                        self.gists_hscroll = 0;
                    }
                    FilterKey::Cleared => {
                        self.gists_filtering = false;
                        self.gists_index = 0;
                        self.gists_hscroll = 0;
                    }
                    FilterKey::Exited => self.gists_filtering = false,
                    FilterKey::Moved | FilterKey::Pass => {}
                },
            }
            return KeyOutcome::None;
        }
        let groups = self.visible_gist_groups();
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.screen = Screen::List,
            KeyCode::Down if self.gists_index + 1 < groups.len() => {
                self.gists_index += 1;
                self.gists_hscroll = 0;
            }
            KeyCode::Up if self.gists_index > 0 => {
                self.gists_index -= 1;
                self.gists_hscroll = 0;
            }
            KeyCode::Right => {
                let max = self.gists_hscroll_max();
                if self.gists_hscroll < max {
                    self.gists_hscroll += 1;
                }
            }
            KeyCode::Left => self.gists_hscroll = self.gists_hscroll.saturating_sub(1),
            KeyCode::Char('/') => self.gists_filtering = true,
            KeyCode::Char('s') => {
                self.gists_sort = self.gists_sort.next();
                self.gists_index = 0;
                self.gists_hscroll = 0;
            }
            KeyCode::Char('v') => {
                self.gists_type_filter = self.gists_type_filter.next();
                self.gists_index = 0;
                self.gists_hscroll = 0;
            }
            KeyCode::Char('e') => {
                if let Some(group) = groups.get(self.gists_index) {
                    self.editing_description = true;
                    self.description_input.set(group.description.clone());
                }
            }
            KeyCode::Enter if self.gists_index < groups.len() => {
                return KeyOutcome::OpenGistDetail;
            }
            KeyCode::Char('o') if self.gists_index < groups.len() => {
                return KeyOutcome::OpenBrowser
            }
            KeyCode::Char('y') if self.gists_index < groups.len() => {
                return KeyOutcome::CopyGistUrl
            }
            KeyCode::Char('c') if self.gists_index < groups.len() => {
                // The revision count needs a network call, so analysis happens in run_loop;
                // the confirm prompt is raised once the count is back.
                self.compact_return_screen = Screen::Gists;
                return KeyOutcome::CompactGist;
            }
            KeyCode::Char('X') => {
                if let Some(group) = groups.get(self.gists_index) {
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
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }

    /// Pure key handling for `Screen::GistDetail`: scroll comments, compact, browser, back.
    fn handle_key_detail(&mut self, code: KeyCode) -> KeyOutcome {
        self.status = None;
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.screen = Screen::Gists;
            }
            KeyCode::Down => self.detail_nav(1),
            KeyCode::Up => self.detail_nav(-1),
            KeyCode::PageDown => self.detail_nav(10),
            KeyCode::PageUp => self.detail_nav(-10),
            KeyCode::Char('o') => return KeyOutcome::OpenBrowser,
            KeyCode::Char('y') => return KeyOutcome::CopyGistUrl,
            KeyCode::Char('c') => {
                self.compact_return_screen = Screen::GistDetail;
                return KeyOutcome::CompactGist;
            }
            // 1–9 preview the content of the Nth file in the gist (full-screen preview).
            KeyCode::Char(c @ '1'..='9') => {
                if let Some(gist_id) = self.detail_gist_id.clone() {
                    let index = (c as u8 - b'1') as usize;
                    if let Some(filename) = self.gist_filenames(&gist_id).into_iter().nth(index) {
                        self.preview_request = Some((gist_id, filename));
                        self.preview_return = Screen::GistDetail;
                        return KeyOutcome::PreviewContent;
                    }
                }
            }
            KeyCode::Tab => {
                self.detail_focus = match self.detail_focus {
                    DetailFocus::Comments => DetailFocus::Files,
                    DetailFocus::Files => DetailFocus::Comments,
                };
            }
            // X deletes the whole gist (y/n confirm), mirroring the gist manager. Reuses the
            // shared Delete confirm path, which lands on the list once the gist is gone.
            KeyCode::Char('X') => {
                if let Some(group) = self
                    .detail_gist_id
                    .clone()
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
            KeyCode::Enter if self.detail_focus == DetailFocus::Files => {
                if let Some(gist_id) = self.detail_gist_id.clone() {
                    let cursor = self.detail_file_cursor;
                    if let Some(filename) = self.gist_filenames(&gist_id).into_iter().nth(cursor) {
                        self.preview_request = Some((gist_id, filename));
                        self.preview_return = Screen::GistDetail;
                        return KeyOutcome::PreviewContent;
                    }
                }
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }

    /// Move within the focused detail pane: scroll comments, or move the file cursor
    /// (clamped to the gist's file count). `delta` is signed rows.
    fn detail_nav(&mut self, delta: i32) {
        match self.detail_focus {
            DetailFocus::Comments => {
                self.detail_scroll = if delta < 0 {
                    self.detail_scroll.saturating_sub((-delta) as u16)
                } else {
                    self.detail_scroll.saturating_add(delta as u16)
                };
            }
            DetailFocus::Files => {
                let count = self
                    .detail_gist_id
                    .as_deref()
                    .map(|id| self.gist_filenames(id).len())
                    .unwrap_or(0);
                if count == 0 {
                    return;
                }
                let max = count - 1;
                let next = self.detail_file_cursor as i64 + delta as i64;
                self.detail_file_cursor = next.clamp(0, max as i64) as usize;
            }
        }
    }

    /// Apply a finished comment fetch, ignoring it if the user has since navigated to a
    /// different gist (stale response). On error, comments become an empty list and the
    /// error message is retained so the detail view can surface it.
    pub fn apply_fetched_comments(
        &mut self,
        gist_id: &str,
        result: Result<Vec<GistComment>, String>,
    ) {
        if self.detail_gist_id.as_deref() != Some(gist_id) {
            return;
        }
        match result {
            Ok(comments) => {
                self.detail_comments = Some(comments);
                self.detail_comments_error = None;
            }
            Err(error) => {
                self.detail_comments = Some(Vec::new());
                self.detail_comments_error = Some(error);
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
            KeyCode::Down => self.scroll_diff_down(),
            KeyCode::Up => self.scroll_diff_up(),
            KeyCode::PageDown => self.scroll_diff_page_down(PAGE_SCROLL),
            KeyCode::PageUp => self.scroll_diff_page_up(PAGE_SCROLL),
            KeyCode::Right => self.scroll_diff_right(),
            KeyCode::Left => self.scroll_diff_left(),
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
            KeyCode::Down => self.list_move_focused(true),
            KeyCode::Up => self.list_move_focused(false),
            KeyCode::Right => self.scroll_focused_right(),
            KeyCode::Left => self.scroll_focused_left(),
            KeyCode::Char('t') => {
                self.gist_view = match self.gist_view {
                    GistView::Description => GistView::Id,
                    GistView::Id => GistView::Description,
                };
            }
            KeyCode::Char('v') => {
                // Cycle the gist visibility filter: all -> public -> secret -> all.
                self.gist_type_filter = self.gist_type_filter.next();
                self.gist_index = 0;
                self.gist_hscroll = 0;
            }
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
            KeyCode::Char('P') => {
                self.pins_index = 0;
                self.pins_hscroll = 0;
                self.screen = Screen::Pins;
            }
            KeyCode::Char('S') => return KeyOutcome::SyncSelectedPair,
            KeyCode::Char('g') => self.open_gist_manager(),
            KeyCode::Char('e') => {
                if self.selected_local().is_some() {
                    return KeyOutcome::EditLocal;
                }
                self.status = Some("select a local file to edit".into());
            }
            KeyCode::Char(' ') if self.selected_gist().is_some() => {
                self.preview_return = Screen::List;
                return KeyOutcome::PreviewContent;
            }
            KeyCode::Char('d')
                if self.focus == FocusPane::Gist && self.gist_index < self.ranked_gists().len() =>
            {
                return KeyOutcome::DownloadGist;
            }
            // Enter works from either pane: it diffs the selected local file against the
            // selected gist (the top match when focus is on the local pane).
            KeyCode::Enter if self.gist_index < self.ranked_gists().len() => {
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
        self.gists_filtering = false;
        self.gists_filter_query.clear();
        self.gists_type_filter = GistTypeFilter::All;
        self.gists_hscroll = 0;
        self.editing_description = false;
        self.description_input.clear();
        let target = self.selected_gist().map(|g| g.file.gist_id);
        let groups = self.visible_gist_groups();
        self.gists_index = target
            .and_then(|id| groups.iter().position(|g| g.id == id))
            .unwrap_or(0);
        self.screen = Screen::Gists;
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
        } else {
            KeyOutcome::Pin
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
            KeyCode::Down => self.scroll_diff_down(),
            KeyCode::Up => self.scroll_diff_up(),
            KeyCode::PageDown => self.scroll_diff_page_down(PAGE_SCROLL),
            KeyCode::PageUp => self.scroll_diff_page_up(PAGE_SCROLL),
            KeyCode::Right => self.scroll_diff_right(),
            KeyCode::Left => self.scroll_diff_left(),
            // Identical files have nothing to sync, so download/upload are not offered.
            KeyCode::Char('d') if !self.diff_identical => {
                if self.download_target.exists() {
                    self.pending_action = Some(PendingAction::Download);
                    self.screen = Screen::Confirm;
                } else {
                    return KeyOutcome::Download;
                }
            }
            KeyCode::Char('u') if !self.diff_identical => return self.upload_intent(),
            // Toggle between the configured context radius and the full file; the line
            // count changes, so reset the vertical scroll. The choice is persisted.
            KeyCode::Char('c') => {
                self.diff_show_full = !self.diff_show_full;
                self.diff_scroll = 0;
                return KeyOutcome::PersistDiffContext;
            }
            _ => {}
        }
        KeyOutcome::None
    }

    fn handle_key_confirm(&mut self, code: KeyCode) -> KeyOutcome {
        // While typing the create flow's description, arrows drive the text cursor (handled
        // below), not the background diff scroll.
        if !self.editing_description {
            match code {
                KeyCode::Down => {
                    self.scroll_diff_down();
                    return KeyOutcome::None;
                }
                KeyCode::Up => {
                    self.scroll_diff_up();
                    return KeyOutcome::None;
                }
                KeyCode::PageDown => {
                    self.scroll_diff_page_down(PAGE_SCROLL);
                    return KeyOutcome::None;
                }
                KeyCode::PageUp => {
                    self.scroll_diff_page_up(PAGE_SCROLL);
                    return KeyOutcome::None;
                }
                KeyCode::Right => {
                    self.scroll_diff_right();
                    return KeyOutcome::None;
                }
                KeyCode::Left => {
                    self.scroll_diff_left();
                    return KeyOutcome::None;
                }
                _ => {}
            }
        }
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
                KeyCode::Char('y') => return KeyOutcome::Upload,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.pending_action = None;
                    self.screen = Screen::List;
                }
                KeyCode::Char('e') => return KeyOutcome::EditUpload,
                KeyCode::Char('p') if is_json_file(local_path) => {
                    self.upload_json_pretty = !self.upload_json_pretty;
                    self.update_upload_diff();
                }
                KeyCode::Char('s') if is_json_file(local_path) => {
                    self.upload_json_sort = !self.upload_json_sort;
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
                    self.screen = self.compact_return_screen;
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
