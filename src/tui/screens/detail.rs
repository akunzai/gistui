//! `Screen::GistDetail` — key handling, view-model, paint, and palette items colocated in
//! one file (issue #287, Phase 2).

use crate::tui::keys::{point_in, NavAction};
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
    widgets::{Block, BorderType, Borders, Padding, Paragraph, Wrap},
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
            file_cursor: 0,
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
            file_cursor: 0,
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
    let file_cursor = detail.file_cursor.min(files.len().saturating_sub(1));
    let comments = build_comments_pane_vm(state);

    let description_input = state
        .editing_description
        .then(|| state.description_input.clone());

    GistDetailVm {
        missing: false,
        block_title,
        info_line,
        focus: detail.focus,
        files,
        file_cursor,
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
        &state.theme,
        layout,
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
        detail_focus_tabs_line(detail.focus, theme),
    ];
    frame.render_widget(
        Paragraph::new(lines).style(theme.base_style()).block(
            Block::default()
                .title(detail.block_title.clone())
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
        // after the left border (1) + horizontal padding (1). Labels: " Files " (7), " │ " (3),
        // " Comments " (10) — see detail_focus_tabs_line.
        let content_x = area.x + 2;
        let tabs_y = area.y + 2;
        layout.detail_tab_files = Some(ratatui::layout::Rect::new(content_x, tabs_y, 7, 1));
        layout.detail_tab_comments =
            Some(ratatui::layout::Rect::new(content_x + 10, tabs_y, 10, 1));
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
                .title(format!("Files ({})", files.len()))
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
                    CommentLineVm::Author { text } => body.push(Line::from(Span::styled(
                        text.clone(),
                        Style::default().fg(theme.accent),
                    ))),
                    CommentLineVm::Body { text } => body.push(Line::from(text.clone())),
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
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(title)
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
    theme: &crate::tui::Theme,
) -> Line<'static> {
    let active = detail_focus_tab(focus);
    let active_style = Style::default()
        .fg(theme.fg_on_accent)
        .bg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let idle_style = Style::default().fg(theme.dim);
    let mut spans = Vec::new();
    for (i, label) in ["Files", "Comments"].iter().enumerate() {
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

pub(crate) fn detail_palette_items(state: &AppState) -> Vec<crate::tui::palette::PaletteItem> {
    use crate::tui::palette::key_item;
    let g = |code| detail_guard(state, code);
    vec![
        key_item(
            "Enter",
            "Preview selected file",
            KeyCode::Enter,
            g(KeyCode::Enter),
        ),
        key_item(
            "o",
            "Open in browser",
            KeyCode::Char('o'),
            g(KeyCode::Char('o')),
        ),
        key_item(
            "y",
            "Copy gist URL",
            KeyCode::Char('y'),
            g(KeyCode::Char('y')),
        ),
        key_item(
            "H",
            "Revision history",
            KeyCode::Char('H'),
            g(KeyCode::Char('H')),
        ),
        key_item(
            "e",
            "Edit description",
            KeyCode::Char('e'),
            g(KeyCode::Char('e')),
        ),
        key_item(
            "c",
            "Compact revisions",
            KeyCode::Char('c'),
            g(KeyCode::Char('c')),
        ),
        key_item(
            "*",
            "Star / unstar gist",
            KeyCode::Char('*'),
            g(KeyCode::Char('*')),
        ),
        key_item("F", "Fork gist", KeyCode::Char('F'), g(KeyCode::Char('F'))),
        key_item(
            "X",
            "Delete gist",
            KeyCode::Char('X'),
            g(KeyCode::Char('X')),
        ),
        key_item("Tab", "Switch Files / Comments", KeyCode::Tab, true),
        key_item(
            "m",
            "Load older comments",
            KeyCode::Char('m'),
            g(KeyCode::Char('m')),
        ),
        key_item("q", "Back to Gist manager", KeyCode::Char('q'), true),
        key_item("?", "Help", KeyCode::Char('?'), true),
    ]
}
