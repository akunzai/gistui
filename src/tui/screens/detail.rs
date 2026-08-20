//! `Screen::GistDetail` — key handling, view-model, paint, and palette items colocated in
//! one file (issue #287, Phase 2).

use crate::domain::GistComment;
use crate::tui::keys::{point_in, NavAction};
use crate::tui::text::comment_lines_count;
use crate::tui::view_model::{
    ChromeVm, CommentLineVm, CommentsAffordance, CommentsPaneVm, GistDetailVm,
};
use crate::tui::{AppState, DetailFocus, HelpTopic, KeyOutcome, MouseLayout, PaneHit, Screen};

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::GistDetail;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step(state: &AppState) -> usize {
    if state
        .detail()
        .is_some_and(|d| d.focus == DetailFocus::Comments)
    {
        3
    } else {
        1
    }
}
use crossterm::event::KeyCode;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph},
    Frame,
};

/// Shared "would this key actually do something" predicate for `Screen::GistDetail`, mirrored
/// by both [`AppState::handle_key_detail`]'s match-arm guards and `detail_palette_items` so the
/// two can never silently drift (issue #288).
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

/// The result of the initial newest-first comment load: the newest page plus the metadata
/// needed to page backwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialComments {
    pub comments: Vec<GistComment>,
    pub total: u32,
    pub oldest_page: u32,
}

impl AppState {
    pub(crate) fn handle_key_detail(&mut self, code: KeyCode) -> KeyOutcome {
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
            // which returns to whichever screen opened this detail view once the gist is gone.
            // Owned gists only (no-op otherwise).
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
                        crate::tui::PendingAction::Delete {
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

    /// Preview the `index`-th file of the gist shown on `Screen::GistDetail` (full-screen),
    /// the action behind the `1`–`9` keys and a file double-click.
    pub(crate) fn preview_detail_file(&mut self, index: usize) -> KeyOutcome {
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

    /// Snapshot the live GistDetail payload as a `Screen`, to stage into
    /// [`AppState::pending_return`] before an async fetch (preview/revisions/compact) that will
    /// eventually call [`AppState::enter`] once its result lands. Every caller is itself part of
    /// GistDetail's own key handling, so `self.screen` is always `Screen::GistDetail` here.
    fn park_gist_detail_screen(&self) -> Screen {
        debug_assert!(self.screen.is_gist_detail());
        self.screen.clone()
    }

    /// Arrow / hjkl / page-key navigation for `Screen::GistDetail`: moves within the focused
    /// pane (comment scroll or file cursor).
    pub(crate) fn apply_navigation_detail(&mut self, action: NavAction) -> bool {
        match action {
            NavAction::Down => self.detail_nav(1),
            NavAction::Up => self.detail_nav(-1),
            NavAction::PageDown => self.detail_nav(10),
            NavAction::PageUp => self.detail_nav(-10),
            _ => return false,
        }
        true
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

    /// Select the clicked row on `Screen::GistDetail`'s Files tab, focusing it. Returns `true`
    /// when a row was hit (so a double-click should "open"/preview it).
    pub(crate) fn click_select_detail(&mut self, col: u16, row: u16, layout: &MouseLayout) -> bool {
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

    /// Switch the GistDetail tab if `col`/`row` lands on a tab header. Returns the outcome
    /// (possibly `FetchComments`) when a tab was clicked, else `None` to fall through.
    pub(crate) fn click_detail_tab(
        &mut self,
        col: u16,
        row: u16,
        layout: &MouseLayout,
    ) -> Option<KeyOutcome> {
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
    pub(crate) fn click_comments_load_older(
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

    /// Reset comment-pagination state (called when (re)opening a gist detail or switching
    /// the loaded gist), so a fresh Tab re-fetches from the newest page.
    pub fn reset_comment_pagination(&mut self) {
        let Some(d) = self.detail_mut() else {
            return;
        };
        d.comments = None;
        d.comments_loading = false;
        d.comments_error = None;
        d.comments_total = None;
        d.comments_loaded_oldest_page = 0;
        d.comments_loading_more = false;
        d.comments_scroll_to_bottom = false;
    }

    /// Apply the initial newest-page load. Ignored if the user navigated to another gist
    /// (stale response). On success, requests a one-shot scroll-to-bottom so the newest
    /// comment is visible.
    pub fn apply_initial_comments(
        &mut self,
        gist_id: &str,
        result: Result<InitialComments, String>,
    ) {
        let Some(d) = self.detail_mut() else {
            return;
        };
        if d.gist_id.as_deref() != Some(gist_id) {
            return;
        }
        d.comments_loading = false;
        match result {
            Ok(init) => {
                d.comments_total = Some(init.total);
                d.comments_loaded_oldest_page = init.oldest_page;
                d.comments = Some(init.comments);
                d.comments_scroll_to_bottom = true;
            }
            Err(error) => {
                d.comments_error = Some(error);
            }
        }
    }

    /// Apply a "load older" page: prepend it (older comments sort first) and bump
    /// `detail_scroll` by the prepended line count so the viewport stays put. Ignored on
    /// stale gist.
    pub fn apply_older_comments(
        &mut self,
        gist_id: &str,
        result: Result<Vec<GistComment>, String>,
    ) {
        let Some(d) = self.detail_mut() else {
            return;
        };
        if d.gist_id.as_deref() != Some(gist_id) {
            return;
        }
        d.comments_loading_more = false;
        match result {
            Ok(mut older) => {
                let added = comment_lines_count(&older);
                if let Some(existing) = d.comments.as_mut() {
                    older.append(existing);
                    *existing = older;
                } else {
                    d.comments = Some(older);
                }
                d.comments_loaded_oldest_page =
                    d.comments_loaded_oldest_page.saturating_sub(1).max(1);
                d.scroll = d.scroll.saturating_add(added);
            }
            Err(error) => {
                d.comments_error = Some(error);
            }
        }
    }

    /// Whether a "load older" action should be offered: comments are loaded, an older page
    /// exists, and no load is already in flight.
    pub fn can_load_older_comments(&self) -> bool {
        let Some(d) = self.detail() else {
            return false;
        };
        d.comments.is_some()
            && d.comments_loaded_oldest_page > 1
            && !d.comments_loading_more
            && !d.comments_loading
    }
}

/// Gist detail body — usable under Palette-over-GistDetail as well.
pub(crate) fn build_gist_detail_vm(state: &AppState) -> GistDetailVm {
    let (footer, footer_colored) =
        crate::tui::footer_with_status(state.status.as_deref(), crate::tui::MINIMAL_HINT);
    let detail = state.detail().cloned().unwrap_or_default();
    let Some(gist_id) = detail.gist_id.as_deref() else {
        return GistDetailVm {
            missing: true,
            block_title: String::new(),
            info_line: String::new(),
            focus: detail.focus,
            files: Vec::new(),
            files_title: String::new(),
            file_cursor: 0,
            comments_count: 0,
            comments: CommentsPaneVm::PromptLoad,
            footer,
            footer_colored,
            description_input: state
                .editing_description
                .then(|| state.description_input.clone()),
        };
    };
    let Some(group) = state.group_by_id(gist_id) else {
        return GistDetailVm {
            missing: true,
            block_title: String::new(),
            info_line: String::new(),
            focus: detail.focus,
            files: Vec::new(),
            files_title: String::new(),
            file_cursor: 0,
            comments_count: 0,
            comments: CommentsPaneVm::PromptLoad,
            footer,
            footer_colored,
            description_input: state
                .editing_description
                .then(|| state.description_input.clone()),
        };
    };

    let block_title = if group.description.trim().is_empty() {
        format!("Gist {}", group.id)
    } else {
        format!("Gist: {}", group.description)
    };
    let info_line = crate::tui::render::gist_info_line(
        &group,
        crate::tui::render::unix_now(),
        state.current_user_login.as_deref(),
        state.gist_is_starred(gist_id),
        state.gist_counts(gist_id),
    );
    let files = state.gist_file_display_names(gist_id);
    let total_size: u64 = state
        .all_gist_files()
        .filter(|file| file.gist_id == gist_id)
        .map(|file| file.size)
        .sum();
    let files_title = format!(
        "Files ({}): {} total",
        files.len(),
        crate::tui::format_file_size(total_size)
    );
    let file_cursor = detail.file_cursor.min(files.len().saturating_sub(1));
    let comments = build_comments_pane_vm(state);
    let comments_count = state.gist_counts(gist_id).0;

    let description_input = state
        .editing_description
        .then(|| state.description_input.clone());

    GistDetailVm {
        missing: false,
        block_title,
        info_line,
        focus: detail.focus,
        files,
        files_title,
        file_cursor,
        comments_count,
        comments,
        footer,
        footer_colored,
        description_input,
    }
}

fn build_comments_pane_vm(state: &AppState) -> CommentsPaneVm {
    let now = crate::tui::render::unix_now() as i64;
    let detail = state.detail().cloned().unwrap_or_default();
    match (
        &detail.comments,
        detail.comments_loading,
        &detail.comments_error,
    ) {
        (None, true, _) => CommentsPaneVm::Loading,
        (None, false, _) => CommentsPaneVm::PromptLoad,
        (Some(_), _, Some(err)) => CommentsPaneVm::Error {
            message: format!("comments error: {err}"),
        },
        (Some(comments), _, None) if comments.is_empty() => CommentsPaneVm::Empty,
        (Some(comments), _, None) => {
            let affordance = if detail.comments_loading_more {
                CommentsAffordance::LoadingMore
            } else if detail.comments_loaded_oldest_page > 1 {
                CommentsAffordance::LoadOlder
            } else {
                CommentsAffordance::StartOfThread
            };
            let mut lines = Vec::new();
            for c in comments {
                let age = crate::domain::parse_rfc3339_to_unix(&c.created_at)
                    .map(|t| crate::domain::humanize_age(now - t as i64))
                    .unwrap_or_else(|| "?".into());
                lines.push(CommentLineVm::Author {
                    text: format!("{} · {age}", c.author),
                });
                for raw in c.body.lines() {
                    lines.push(CommentLineVm::Body {
                        text: format!("  {raw}"),
                    });
                }
                lines.push(CommentLineVm::Blank);
            }
            CommentsPaneVm::Thread {
                title: comments_title_text(state),
                affordance,
                lines,
                scroll: detail.scroll,
            }
        }
    }
}

/// Mirror of render-side comments title (pure; used by the view model).
fn comments_title_text(state: &AppState) -> String {
    let detail = state.detail().cloned().unwrap_or_default();
    match (&detail.comments, detail.comments_total) {
        (Some(c), _) if detail.comments_error.is_some() => format!("Comments ({})", c.len()),
        (Some(c), Some(total)) if !c.is_empty() => {
            let loaded = c.len() as u32;
            let first = total.saturating_sub(loaded) + 1;
            format!("Comments ({first}–{total} / {total})")
        }
        (Some(c), None) if !c.is_empty() => format!("Comments (newest {})", c.len()),
        _ => "Comments".to_string(),
    }
}

pub(crate) fn render_gist_detail_vm(
    frame: &mut Frame,
    state: &AppState,
    detail: &GistDetailVm,
    chrome: &ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    // Fixed 4-row header (borders + basic-info line + focus tabs); the active tab — the file
    // list or the comments, never both — fills the rest above the footer.
    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(4),
            ratatui::layout::Constraint::Min(3),
            ratatui::layout::Constraint::Length(crate::tui::render::footer_height(
                &detail.footer,
                area.width,
                "",
                detail.footer_colored,
            )),
        ])
        .split(area);
    if !detail.missing {
        render_detail_header_vm(frame, chunks[0], detail, chrome, &state.theme, layout);
        match detail.focus {
            DetailFocus::Files => {
                render_gist_file_list_vm(frame, chunks[1], detail, chrome, &state.theme, layout)
            }
            DetailFocus::Comments => {
                render_gist_comments_vm(frame, chunks[1], &detail.comments, &state.theme, layout)
            }
        }
    }
    crate::tui::render_footer(
        frame,
        chunks[2],
        "",
        &detail.footer,
        detail.footer_colored,
        crate::tui::keymap::for_screen(&state.screen),
        &state.theme,
    );

    let edit_modal = if let Some(input) = &detail.description_input {
        // The modal covers the file list and tabs; drop their hit regions so a click
        // behind the modal doesn't move the cursor or switch tabs.
        layout.detail_files = None;
        layout.detail_tab_files = None;
        layout.detail_tab_comments = None;
        layout.comments_load_older = None;
        Some(crate::tui::render::render_centered_modal_input(
            frame,
            "Edit description (Enter apply · Esc cancel)",
            "",
            input,
            "",
            state.theme.accent,
            &state.theme,
        ))
    } else {
        None
    };
    if chrome.mouse_enabled {
        // When the edit-description modal is open, the close button belongs on it;
        // otherwise it sits on the full-screen detail view's top-right corner.
        layout.close_button = Some(crate::tui::render_close_button(
            frame,
            edit_modal.unwrap_or(area),
            &state.theme,
        ));
    }
}

fn render_detail_header_vm(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    detail: &GistDetailVm,
    chrome: &ChromeVm,
    theme: &crate::tui::Theme,
    layout: &mut MouseLayout,
) {
    let lines = vec![
        Line::from(detail.info_line.clone()),
        detail_focus_tabs_line(detail.focus, detail.comments_count, theme),
    ];
    frame.render_widget(
        Paragraph::new(lines).style(theme.base_style()).block(
            Block::default()
                .title(crate::tui::render::fit_block_title(
                    &detail.block_title,
                    area.width,
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.accent))
                .style(theme.base_style())
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
    if chrome.mouse_enabled {
        // Tab line is the 2nd content row (border + gist-info line above it); content starts
        // after the left border (1) + horizontal padding (1). The comments label includes its
        // count, so derive its click target from the same formatted label as the renderer.
        let content_x = area.x + 2;
        let tabs_y = area.y + 2;
        layout.detail_tab_files = Some(ratatui::layout::Rect::new(content_x, tabs_y, 7, 1));
        let comments_width = format!(" Comments ({}) ", detail.comments_count).len() as u16;
        layout.detail_tab_comments = Some(ratatui::layout::Rect::new(
            content_x + 10,
            tabs_y,
            comments_width,
            1,
        ));
    }
}

/// Files tab from the view model (full file list; paint windows to the area height).
fn render_gist_file_list_vm(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    detail: &GistDetailVm,
    chrome: &ChromeVm,
    theme: &crate::tui::Theme,
    layout: &mut MouseLayout,
) {
    let file_list_height = u16::try_from(detail.files.len().saturating_add(2))
        .unwrap_or(u16::MAX)
        .clamp(3, area.height.max(3));
    let area = ratatui::layout::Rect::new(area.x, area.y, area.width, file_list_height);
    let files = &detail.files;
    let cursor = detail.file_cursor.min(files.len().saturating_sub(1));
    let visible_rows = (area.height as usize).saturating_sub(2);
    let offset = crate::tui::render::file_list_scroll(cursor, visible_rows, files.len());
    if chrome.mouse_enabled {
        layout.detail_files = Some(PaneHit { rect: area, offset });
    }
    let lines = crate::tui::render::file_rows(files, cursor, offset, visible_rows, true, theme);
    frame.render_widget(
        Paragraph::new(lines).style(theme.base_style()).block(
            Block::default()
                .title(crate::tui::render::fit_block_title(
                    &detail.files_title,
                    area.width,
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.accent))
                .style(theme.base_style())
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

/// Comments pane from the view model: styles plain presentation facts and fills hit/scroll layout.
fn render_gist_comments_vm(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    comments: &CommentsPaneVm,
    theme: &crate::tui::Theme,
    layout: &mut MouseLayout,
) {
    let mut body: Vec<Line> = Vec::new();
    let mut affordance_present = false;
    let mut title = "Comments".to_string();
    let mut scroll = 0u16;
    // Borders only (no padding): wrap body lines to the inner width so a
    // continuation keeps the source line's indent (#342).
    let inner_width = area.width.saturating_sub(2) as usize;

    match comments {
        CommentsPaneVm::Loading => body.push(Line::from(Span::styled(
            "Loading comments…",
            Style::default().fg(theme.dim),
        ))),
        CommentsPaneVm::PromptLoad => body.push(Line::from(Span::styled(
            "Tab here to load comments",
            Style::default().fg(theme.dim),
        ))),
        CommentsPaneVm::Error { message } => body.push(Line::from(Span::styled(
            message.clone(),
            Style::default().fg(theme.del_color),
        ))),
        CommentsPaneVm::Empty => body.push(Line::from(Span::styled(
            "No comments",
            Style::default().fg(theme.dim),
        ))),
        CommentsPaneVm::Thread {
            title: t,
            affordance,
            lines,
            scroll: s,
        } => {
            title = t.clone();
            scroll = *s;
            let label = match affordance {
                CommentsAffordance::LoadingMore => "Loading…",
                CommentsAffordance::LoadOlder => {
                    affordance_present = true;
                    "↑ Load 30 older comments"
                }
                CommentsAffordance::StartOfThread => "— Start of thread —",
            };
            body.push(Line::from(Span::styled(
                label,
                Style::default().fg(theme.dim),
            )));
            body.push(Line::from(""));
            for line in lines {
                match line {
                    CommentLineVm::Author { text } => {
                        for part in crate::tui::render::wrap_hanging(text, inner_width) {
                            body.push(Line::from(Span::styled(
                                part,
                                Style::default().fg(theme.accent),
                            )));
                        }
                    }
                    CommentLineVm::Body { text } => {
                        for part in crate::tui::render::wrap_hanging(text, inner_width) {
                            body.push(Line::from(part));
                        }
                    }
                    CommentLineVm::Blank => body.push(Line::from("")),
                }
            }
        }
    }

    layout.comments_load_older = if affordance_present {
        Some(ratatui::layout::Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            1,
        ))
    } else {
        None
    };

    let total_lines = body.len();
    let inner_rows = area.height.saturating_sub(2);
    layout.comments_max_scroll = Some((total_lines as u16).saturating_sub(inner_rows));

    frame.render_widget(
        Paragraph::new(body)
            .style(theme.base_style())
            .scroll((scroll, 0))
            .block(
                Block::default()
                    .title(crate::tui::render::fit_block_title(&title, area.width))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .style(theme.base_style()),
            ),
        area,
    );
    crate::tui::render::render_text_scrollbar(frame, area, total_lines, scroll as usize);
}

/// The Files|Comments tab index, mirroring `detail_focus`. Pure so the tab selection is
/// trivially testable and stays in sync with the navigation handler. Files is the default
/// tab, so it comes first.
pub(crate) fn detail_focus_tab(focus: DetailFocus) -> usize {
    match focus {
        DetailFocus::Files => 0,
        DetailFocus::Comments => 1,
    }
}

/// A `Files │ Comments` focus indicator line, with the pane Tab currently drives highlighted.
/// Rendered just under the gist's basic info (inside the info box) rather than as a floating
/// strip, so the active focus is visible without a disconnected top row.
pub(crate) fn detail_focus_tabs_line(
    focus: DetailFocus,
    comments_count: u32,
    theme: &crate::tui::Theme,
) -> Line<'static> {
    let active = detail_focus_tab(focus);
    let active_style = Style::default()
        .fg(theme.fg_on_accent)
        .bg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let idle_style = Style::default().fg(theme.dim);
    let mut spans = Vec::new();
    let comments = format!("Comments ({comments_count})");
    for (i, label) in ["Files".to_string(), comments].iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", idle_style));
        }
        let style = if i == active {
            active_style
        } else {
            idle_style
        };
        spans.push(Span::styled(format!(" {label} "), style));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::*;

    use crate::tui::tests::{detail_mut, state_with_gists, state_with_two_gists};

    fn detail_ref(state: &AppState) -> &DetailState {
        state.detail().expect("expected Screen::GistDetail")
    }

    fn state_with_many_files(n: usize) -> AppState {
        let mut state = initial_state();
        state.gists = (0..n)
            .map(|i| GistFile {
                gist_id: "g1".into(),
                description: "demo".into(),
                filename: format!("f{i}.txt"),
                public: false,
                updated_at: "2026-06-10T00:00:00Z".into(),
                created_at: "2026-06-01T00:00:00Z".into(),
                owner_login: String::new(),
                fork_of_id: None,

                raw_url: None,

                content_type: None,

                size: 0,

                node_id: None,
            })
            .collect();
        state
    }

    #[test]
    fn detail_focus_and_cursor_default_to_files_and_zero() {
        let mut state = initial_state();
        state.screen = Screen::GistDetail(Box::default());
        assert_eq!(detail_ref(&state).focus, DetailFocus::Files);
        assert_eq!(detail_ref(&state).file_cursor, 0);
    }

    #[test]
    fn detail_tab_toggles_focus() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        assert_eq!(detail_ref(&state).focus, DetailFocus::Files);
        let outcome = state.handle_key(KeyCode::Tab);
        assert!(matches!(outcome, KeyOutcome::FetchComments { .. }));
        assert_eq!(detail_ref(&state).focus, DetailFocus::Comments);
        let outcome = state.handle_key(KeyCode::Tab);
        assert!(matches!(outcome, KeyOutcome::None));
        assert_eq!(detail_ref(&state).focus, DetailFocus::Files);
    }

    #[test]
    fn detail_tab_to_comments_skips_fetch_when_already_loaded() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        detail_mut(&mut state).comments = Some(Vec::new());
        let outcome = state.handle_key(KeyCode::Tab);
        assert!(matches!(outcome, KeyOutcome::None));
        assert_eq!(detail_ref(&state).focus, DetailFocus::Comments);
    }

    #[test]
    fn detail_files_focus_arrows_move_cursor_and_clamp() {
        let mut state = state_with_gists(); // g1 has 2 files: a.txt, b.txt
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        detail_mut(&mut state).focus = DetailFocus::Files;

        state.handle_key(KeyCode::Up); // already at 0, clamps
        assert_eq!(detail_ref(&state).file_cursor, 0);
        state.handle_key(KeyCode::Down);
        assert_eq!(detail_ref(&state).file_cursor, 1);
        state.handle_key(KeyCode::Down); // only 2 files, clamps at index 1
        assert_eq!(detail_ref(&state).file_cursor, 1);
        state.handle_key(KeyCode::PageUp); // jumps to 0
        assert_eq!(detail_ref(&state).file_cursor, 0);
        state.handle_key(KeyCode::PageDown); // +10 clamps to last (1)
        assert_eq!(detail_ref(&state).file_cursor, 1);
        // Comment scroll is untouched while files-focused.
        assert_eq!(detail_ref(&state).scroll, 0);
    }

    #[test]
    fn detail_comments_focus_arrows_still_scroll_comments() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).focus = DetailFocus::Comments;
        state.handle_key(KeyCode::Down);
        assert_eq!(detail_ref(&state).scroll, 1);
        assert_eq!(detail_ref(&state).file_cursor, 0); // cursor untouched
    }

    #[test]
    fn detail_enter_previews_cursor_file_including_tenth() {
        let mut state = state_with_many_files(12);
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        detail_mut(&mut state).focus = DetailFocus::Files;
        detail_mut(&mut state).file_cursor = 9; // the 10th file — unreachable via 1-9
        let outcome = state.handle_key(KeyCode::Enter);
        assert!(matches!(
            outcome,
            KeyOutcome::PreviewContent {
                file: ref f,
                ..
            } if f.gist_id == "g1" && f.filename == "f9.txt"
        ));
        assert!(state
            .pending_return
            .as_ref()
            .is_some_and(Screen::is_gist_detail));
    }

    #[test]
    fn detail_enter_in_comments_focus_is_noop() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        detail_mut(&mut state).focus = DetailFocus::Comments;
        let outcome = state.handle_key(KeyCode::Enter);
        assert!(matches!(outcome, KeyOutcome::None));
    }

    #[test]
    fn detail_scroll_saturates_at_zero() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).focus = DetailFocus::Comments;
        detail_mut(&mut state).scroll = 0;
        state.handle_key(KeyCode::Up);
        assert_eq!(detail_ref(&state).scroll, 0);
        state.handle_key(KeyCode::Down);
        assert_eq!(detail_ref(&state).scroll, 1);
    }

    #[test]
    fn detail_c_triggers_compaction_and_records_origin() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        let outcome = state.handle_key(KeyCode::Char('c'));
        assert!(matches!(outcome, KeyOutcome::CompactGist { .. }));
        assert!(state
            .pending_return
            .as_ref()
            .is_some_and(Screen::is_gist_detail));
    }

    #[test]
    fn detail_number_key_requests_file_preview() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        let outcome = state.handle_key(KeyCode::Char('1'));
        assert!(matches!(
            outcome,
            KeyOutcome::PreviewContent {
                file: ref f,
                ..
            } if f.gist_id == "g1" && f.filename == "a.txt"
        ));
        assert!(state
            .pending_return
            .as_ref()
            .is_some_and(Screen::is_gist_detail));
    }

    #[test]
    fn detail_number_key_out_of_range_is_ignored() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        // Only two files exist; pressing 5 must do nothing (no fetch requested).
        let outcome = state.handle_key(KeyCode::Char('5'));
        assert!(matches!(outcome, KeyOutcome::None));
    }

    #[test]
    fn detail_x_requests_gist_delete_confirm() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        let outcome = state.handle_key(KeyCode::Char('X'));
        assert!(matches!(outcome, KeyOutcome::None));
        assert!(state.screen.is_confirm());
        assert!(matches!(
            state.pending_action(),
            Some(PendingAction::Delete { gist_id, .. }) if gist_id == "g1"
        ));
    }

    #[test]
    fn star_key_in_detail_returns_toggle_intent() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        assert!(matches!(
            state.handle_key(KeyCode::Char('*')),
            KeyOutcome::ToggleGistStar { .. }
        ));
    }

    #[test]
    fn context_gist_id_uses_detail_id_on_detail_screen() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        assert_eq!(state.context_gist_id().as_deref(), Some("g1"));
    }

    #[test]
    fn detail_e_edits_description_with_prefill_and_enter_applies() {
        let mut state = state_with_two_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("a".into());
        state.handle_key(KeyCode::Char('e'));
        assert!(state.editing_description);
        // Prefilled with the current description.
        assert_eq!(state.description_input, "My Ghostty config");
        state.handle_key(KeyCode::Char('!'));
        assert_eq!(state.description_input, "My Ghostty config!");
        assert!(matches!(
            state.handle_key(KeyCode::Enter),
            KeyOutcome::ApplyDescription { .. }
        ));
    }

    #[test]
    fn detail_description_edits_mid_string_with_cursor_keys() {
        let mut state = state_with_two_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("a".into());
        state.handle_key(KeyCode::Char('e'));
        assert_eq!(state.description_input, "My Ghostty config");
        // Jump to the start, step right past "My", and insert without retyping the rest.
        state.handle_key(KeyCode::Home);
        state.handle_key(KeyCode::Right);
        state.handle_key(KeyCode::Right);
        state.handle_key(KeyCode::Char(' '));
        state.handle_key(KeyCode::Char('o'));
        state.handle_key(KeyCode::Char('w'));
        state.handle_key(KeyCode::Char('n'));
        assert_eq!(state.description_input, "My own Ghostty config");
        // Delete removes the char at the cursor (the space before "Ghostty").
        state.handle_key(KeyCode::Delete);
        assert_eq!(state.description_input, "My ownGhostty config");
    }

    #[test]
    fn detail_esc_cancels_description_edit() {
        let mut state = state_with_two_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("a".into());
        state.handle_key(KeyCode::Char('e'));
        assert!(state.editing_description);
        state.handle_key(KeyCode::Esc);
        assert!(!state.editing_description);
        assert!(state.description_input.is_empty());
    }

    #[test]
    fn detail_x_stages_whole_gist_delete() {
        let mut state = state_with_two_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("b".into());
        assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
        assert!(state.screen.is_confirm());
        assert_eq!(
            state.pending_action().cloned(),
            Some(PendingAction::Delete {
                gist_id: "b".into(),
                label: "SSH config".into(),
            })
        );
    }

    #[test]
    fn ctrl_f_pages_gist_detail_files() {
        use crossterm::event::KeyModifiers;
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        detail_mut(&mut state).file_cursor = 0;
        state.handle_key_with(KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert_eq!(detail_ref(&state).file_cursor, 1);
    }

    #[test]
    fn fork_key_returns_fork_intent_for_foreign_gist_in_detail() {
        let mut state = initial_state();
        state.current_user_login = Some("me".into());
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("foreign".into());
        state.starred_gists = vec![GistFile {
            gist_id: "foreign".into(),
            description: "x".into(),
            filename: "a.txt".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: "other".into(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            size: 0,

            node_id: None,
        }];
        assert!(matches!(
            state.handle_key(KeyCode::Char('F')),
            KeyOutcome::ForkGist { .. }
        ));
    }

    #[test]
    fn fork_key_blocked_for_owned_gist_in_detail() {
        let mut state = initial_state();
        state.current_user_login = Some("me".into());
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("mine".into());
        state.gists = vec![GistFile {
            gist_id: "mine".into(),
            description: "x".into(),
            filename: "a.txt".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: "me".into(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            size: 0,

            node_id: None,
        }];
        assert_eq!(state.handle_key(KeyCode::Char('F')), KeyOutcome::None);
        assert!(state.status.as_ref().unwrap().contains("already yours"));
    }

    #[test]
    fn foreign_detail_mutate_keys_are_silent_noop() {
        let mut state = initial_state();
        state.current_user_login = Some("me".into());
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("foreign".into());
        state.starred_gists = vec![GistFile {
            gist_id: "foreign".into(),
            description: "x".into(),
            filename: "a.txt".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: "other".into(),
            fork_of_id: None,
            raw_url: None,
            content_type: None,
            size: 0,
            node_id: None,
        }];
        assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::None);
        assert_eq!(state.handle_key(KeyCode::Char('c')), KeyOutcome::None);
        assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
        assert!(state.status.is_none());
    }

    #[test]
    fn wheel_step_gist_detail_moves_three() {
        // GistDetail content pane: one scroll-down tick must advance detail_scroll by 3.
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        // Use Comments focus so detail_nav moves detail_scroll (not the file cursor).
        detail_mut(&mut state).focus = DetailFocus::Comments;
        detail_mut(&mut state).comments = Some(Vec::new());
        assert_eq!(detail_ref(&state).scroll, 0);
        state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
        assert_eq!(detail_ref(&state).scroll, 3);
    }

    #[test]
    fn gist_detail_file_click_selects_and_double_previews() {
        let mut state = state_with_gists(); // g1: a.txt (0), b.txt (1)
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        detail_mut(&mut state).focus = DetailFocus::Comments; // start elsewhere to prove the focus switch
        let hit = PaneHit {
            rect: Rect::new(0, 0, 40, 10),
            offset: 0,
        };
        let layout = MouseLayout {
            detail_files: Some(hit),
            ..Default::default()
        };
        // Click the 2nd file row -> Files focus + cursor 1, but no open yet.
        let out = state.handle_mouse(MouseInput::Click { col: 5, row: 2 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(detail_ref(&state).focus, DetailFocus::Files);
        assert_eq!(detail_ref(&state).file_cursor, 1);
        // Double-click previews that file (there is no Enter for files).
        let out = state.handle_mouse(MouseInput::DoubleClick { col: 5, row: 2 }, &layout);
        assert!(matches!(
            out,
            KeyOutcome::PreviewContent {
                file: ref f,
                ..
            } if f.gist_id == "g1" && f.filename == "b.txt"
        ));
    }

    #[test]
    fn gist_detail_tab_click_switches_focus() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        detail_mut(&mut state).focus = DetailFocus::Files;
        // Header at chunks[0] y=0: content_x = 2, tabs_y = 2; " Files " (7), " Comments " (10 @ +10).
        let layout = MouseLayout {
            detail_tab_files: Some(Rect::new(2, 2, 7, 1)),
            detail_tab_comments: Some(Rect::new(12, 2, 10, 1)),
            ..Default::default()
        };
        // Click the Comments tab: switches focus and (comments unloaded) requests a fetch.
        let out = state.handle_mouse(MouseInput::Click { col: 14, row: 2 }, &layout);
        assert_eq!(detail_ref(&state).focus, DetailFocus::Comments);
        assert!(matches!(out, KeyOutcome::FetchComments { .. }));
        // Click the Files tab back.
        let out = state.handle_mouse(MouseInput::Click { col: 4, row: 2 }, &layout);
        assert_eq!(detail_ref(&state).focus, DetailFocus::Files);
        assert_eq!(out, KeyOutcome::None);
    }

    #[test]
    fn wheel_step_gist_detail_files_moves_one() {
        // The file list (Files tab) steps one file per wheel tick, not 3.
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        detail_mut(&mut state).focus = DetailFocus::Files;
        detail_mut(&mut state).file_cursor = 0;
        state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
        assert_eq!(detail_ref(&state).file_cursor, 1);
    }

    #[test]
    fn comment_lines_count_matches_view_model_thread_lines() {
        use crate::domain::GistComment;
        use crate::tui::screens::detail::build_gist_detail_vm;
        use crate::tui::text::comment_lines_count;
        use crate::tui::view_model::{CommentLineVm, CommentsPaneVm};
        let comments = vec![
            GistComment {
                author: "alice".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                body: "one line".into(),
            },
            GistComment {
                author: "bob".into(),
                created_at: "2026-01-02T00:00:00Z".into(),
                body: "two\nlines".into(),
            },
        ];
        // Each comment: 1 header + body.lines() + 1 blank.
        // alice: 1 + 1 + 1 = 3 ; bob: 1 + 2 + 1 = 4 ; total 7.
        assert_eq!(comment_lines_count(&comments), 7);

        let mut state = initial_state();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        state.gists = vec![GistFile::for_sync("g1".into(), "a.txt".into(), None)];
        detail_mut(&mut state).comments = Some(comments);
        detail_mut(&mut state).comments_loaded_oldest_page = 1;
        let detail = build_gist_detail_vm(&state);
        match detail.comments {
            CommentsPaneVm::Thread { lines, .. } => {
                assert_eq!(lines.len(), 7);
                assert!(matches!(lines[0], CommentLineVm::Author { .. }));
            }
            other => panic!("expected Thread, got {other:?}"),
        }
    }

    fn sample_comment(author: &str, body: &str) -> crate::domain::GistComment {
        crate::domain::GistComment {
            author: author.into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            body: body.into(),
        }
    }

    #[test]
    fn apply_initial_comments_sets_window_and_requests_bottom_scroll() {
        use crate::tui::InitialComments;
        let mut s = crate::tui::initial_state();
        detail_mut(&mut s).gist_id = Some("g1".into());
        s.apply_initial_comments(
            "g1",
            Ok(InitialComments {
                comments: vec![sample_comment("a", "x")],
                total: 910,
                oldest_page: 31,
            }),
        );
        assert_eq!(detail_ref(&s).comments_total, Some(910));
        assert_eq!(detail_ref(&s).comments_loaded_oldest_page, 31);
        assert!(detail_ref(&s).comments_scroll_to_bottom);
        assert!(s.can_load_older_comments()); // page 31 > 1
        assert_eq!(detail_ref(&s).comments.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn apply_initial_comments_ignored_when_gist_changed() {
        use crate::tui::InitialComments;
        let mut s = crate::tui::initial_state();
        detail_mut(&mut s).gist_id = Some("g2".into());
        s.apply_initial_comments(
            "g1",
            Ok(InitialComments {
                comments: vec![],
                total: 0,
                oldest_page: 1,
            }),
        );
        assert!(detail_ref(&s).comments.is_none()); // stale response dropped
    }

    #[test]
    fn apply_older_comments_prepends_and_compensates_scroll() {
        use crate::tui::InitialComments;
        let mut s = crate::tui::initial_state();
        detail_mut(&mut s).gist_id = Some("g1".into());
        s.apply_initial_comments(
            "g1",
            Ok(InitialComments {
                comments: vec![sample_comment("newer", "n")],
                total: 60,
                oldest_page: 2,
            }),
        );
        detail_mut(&mut s).scroll = 5;
        // One older comment = 1 header + 1 body + 1 blank = 3 lines prepended.
        s.apply_older_comments("g1", Ok(vec![sample_comment("older", "o")]));
        assert_eq!(detail_ref(&s).comments_loaded_oldest_page, 1);
        assert!(!s.can_load_older_comments()); // reached page 1
        assert_eq!(detail_ref(&s).comments.as_ref().unwrap()[0].author, "older"); // prepended
        assert_eq!(detail_ref(&s).scroll, 5 + 3); // viewport held in place
        assert!(!detail_ref(&s).comments_loading_more);
    }

    #[test]
    fn can_load_older_false_while_loading_more() {
        use crate::tui::InitialComments;
        let mut s = crate::tui::initial_state();
        detail_mut(&mut s).gist_id = Some("g1".into());
        s.apply_initial_comments(
            "g1",
            Ok(InitialComments {
                comments: vec![sample_comment("a", "x")],
                total: 90,
                oldest_page: 3,
            }),
        );
        detail_mut(&mut s).comments_loading_more = true;
        assert!(!s.can_load_older_comments());
    }

    #[test]
    fn m_key_loads_older_when_available() {
        use crate::tui::InitialComments;
        let mut s = crate::tui::initial_state();
        s.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut s).focus = DetailFocus::Comments;
        detail_mut(&mut s).gist_id = Some("g1".into());
        s.apply_initial_comments(
            "g1",
            Ok(InitialComments {
                comments: vec![sample_comment("a", "x")],
                total: 90,
                oldest_page: 3,
            }),
        );
        let out = s.handle_key(KeyCode::Char('m'));
        assert!(matches!(out, KeyOutcome::LoadOlderComments { .. }));
    }

    #[test]
    fn m_key_noop_when_at_oldest_page() {
        use crate::tui::InitialComments;
        let mut s = crate::tui::initial_state();
        s.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut s).focus = DetailFocus::Comments;
        detail_mut(&mut s).gist_id = Some("g1".into());
        s.apply_initial_comments(
            "g1",
            Ok(InitialComments {
                comments: vec![sample_comment("a", "x")],
                total: 10,
                oldest_page: 1,
            }),
        );
        let out = s.handle_key(KeyCode::Char('m'));
        assert!(matches!(out, KeyOutcome::None));
    }
}
