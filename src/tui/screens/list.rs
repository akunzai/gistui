//! `Screen::List` — key handling, view-model, paint, and palette items colocated in one
//! file (issue #287, Phase 2).

use crate::tui::keys::{apply_filter_edit, diff_pair_previewable, point_in, FilterKey, NavAction};
use crate::tui::view_model::{
    ChromeVm, ListFooterVm, ListPaneEmpty, ListPaneVm, ListRowVm, ListVm, PaneTitleVm,
};
use crate::tui::{
    AppState, FocusPane, GistView, HelpTopic, KeyOutcome, MouseLayout, PaneHit, PendingAction,
    Screen,
};
use crossterm::event::KeyCode;

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::List;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    1
}
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Modifier, Style},
    widgets::{
        Block, BorderType, Borders, List, ListItem, ListState, Padding, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};

/// Shared "would this key actually do something" predicate for `Screen::List`, mirrored by
/// both [`AppState::handle_key_list`]'s match-arm guards and `list_palette_items` so the two
/// can never silently drift (issue #288).
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
    pub(crate) fn handle_key_filter(&mut self, code: KeyCode) -> KeyOutcome {
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

    pub(crate) fn handle_key_list(&mut self, code: KeyCode) -> KeyOutcome {
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
                let (Some(local), Some(gist)) = self.selected_pair() else {
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
                let target = local_path
                    .as_deref()
                    .and_then(std::path::Path::parent)
                    .unwrap_or(&self.cwd)
                    .join(&filename);
                return KeyOutcome::PreviewDiff {
                    local_path,
                    file: crate::domain::GistFileRef::new(
                        gist.file.gist_id.clone(),
                        filename.clone(),
                        gist.file.raw_url.clone(),
                    ),
                    target,
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

    /// Arrow / hjkl / page-key navigation for `Screen::List`: moves the focused pane's
    /// selection, or scrolls it horizontally.
    pub(crate) fn apply_navigation_list(&mut self, action: NavAction) -> bool {
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

    /// Select the clicked row on `Screen::List`, focusing its pane. Returns `true` when a row
    /// was hit (so a double-click should "open" it). A click in a pane's blank area or border
    /// focuses it but selects nothing (returns `false`); a click off every list returns `false`.
    pub(crate) fn click_select_list(&mut self, col: u16, row: u16, layout: &MouseLayout) -> bool {
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
}

/// The ` ⚓` suffix for whichever pane drives the match ranking, empty for the other one.
fn anchor_marker(state: &AppState, pane: FocusPane) -> &'static str {
    if state.anchor == pane {
        " ⚓"
    } else {
        ""
    }
}

/// List body only — usable while `state.screen` is List **or** Palette-over-List (#250).
pub(crate) fn build_list_vm(state: &AppState) -> ListVm {
    let (visible_locals, ranked) = state.list_pane_snapshots();

    let local_empty;
    let local_empty_message;
    let local_rows;
    if state.local_scanning && state.locals.is_empty() {
        local_empty = ListPaneEmpty::Loading;
        local_empty_message = Some(format!(
            "  {} Scanning files…",
            crate::tui::render::spinner_glyph(state.spinner_frame)
        ));
        local_rows = Vec::new();
    } else if state.locals.is_empty() {
        local_empty = ListPaneEmpty::NoItems;
        local_empty_message = Some("  📭 No local files found".into());
        local_rows = Vec::new();
    } else if visible_locals.is_empty() {
        local_empty = ListPaneEmpty::NoFilterMatch;
        local_empty_message = Some("  🔍 No files match the filter".into());
        local_rows = Vec::new();
    } else {
        local_empty = ListPaneEmpty::HasRows;
        local_empty_message = None;
        local_rows = visible_locals
            .iter()
            .map(|r| {
                let base = crate::tui::text::local_row_label(&r.candidate.path, &state.cwd);
                ListRowVm {
                    label: crate::tui::render::marked_row_text(base, r.mark),
                    mark: r.mark,
                }
            })
            .collect();
    }

    let recursive_marker = if state.local_recursive { " [↓]" } else { "" };
    let scanning_marker = if state.local_scanning { " …" } else { "" };
    // Order per `PaneTitleVm` (#338): the anchor rides in the head so pressing `a` always shows,
    // the state a keypress just changed follows, and the cwd is the context that gives way.
    // The pane name is the one part of the head a reader can re-derive from `[1]` and the
    // layout, so it is what a narrow pane gives up to keep `sort:` visible.
    let local_head = |name: &str| {
        format!(
            "[1] {name}{}{}{}{}",
            crate::tui::render::count_label(visible_locals.len(), state.locals.len()),
            anchor_marker(state, FocusPane::Local),
            recursive_marker,
            scanning_marker
        )
    };
    let mut local_title = PaneTitleVm::new(local_head("Local "));
    local_title.short_head = Some(local_head(""));
    local_title.push(format!("sort:{}", state.local_sort.label()));
    local_title.push_filter(&state.local_filter_query);
    local_title.context = Some(crate::config::display_path(&state.cwd));

    let gist_empty;
    let gist_empty_message;
    let gist_rows;
    if state.loading && ranked.is_empty() {
        gist_empty = ListPaneEmpty::Loading;
        gist_empty_message = Some(format!(
            "  {} Loading gists…",
            crate::tui::render::spinner_glyph(state.spinner_frame)
        ));
        gist_rows = Vec::new();
    } else if ranked.is_empty() {
        if !state.filter_query.is_empty() {
            gist_empty = ListPaneEmpty::NoFilterMatch;
            gist_empty_message = Some("  🔍 No gists match the filter".into());
        } else {
            gist_empty = ListPaneEmpty::NoItems;
            gist_empty_message = Some("  📭 No gists found".into());
        }
        gist_rows = Vec::new();
    } else {
        gist_empty = ListPaneEmpty::HasRows;
        gist_empty_message = None;
        gist_rows = ranked
            .iter()
            .map(|g| {
                let base = crate::tui::gist_row_display(g, state.gist_view, state);
                ListRowVm {
                    label: crate::tui::render::marked_row_text(base, g.mark),
                    mark: g.mark,
                }
            })
            .collect();
    }

    // Same order as the Local pane (#338). No re-derivable context here, so nothing is ever
    // shortened — a segment either fits whole or is dropped.
    let gist_head = |name: &str| {
        format!(
            "[2] {name}{}{}",
            crate::tui::render::count_label(ranked.len(), state.gists.len()),
            anchor_marker(state, FocusPane::Gist)
        )
    };
    let mut gist_title = PaneTitleVm::new(gist_head("Gists "));
    gist_title.short_head = Some(gist_head(""));
    gist_title.push(state.gist_type_filter.label());
    gist_title.push(state.gist_sort.label());
    gist_title.push_filter(&state.filter_query);

    let footer = if state.filtering {
        let query = match state.focus {
            FocusPane::Local => state.local_filter_query.clone(),
            FocusPane::Gist => state.filter_query.clone(),
        };
        ListFooterVm::Filtering {
            focus: state.focus,
            query,
        }
    } else if let Some(message) = &state.status {
        ListFooterVm::Status {
            text: message.clone(),
        }
    } else {
        ListFooterVm::Hints {
            text: crate::tui::MINIMAL_HINT.to_string(),
        }
    };

    ListVm {
        local: ListPaneVm {
            title: local_title,
            focused: state.focus == FocusPane::Local,
            selected: (local_empty == ListPaneEmpty::HasRows).then_some(state.local_index),
            empty: local_empty,
            empty_message: local_empty_message,
            rows: local_rows,
        },
        gist: ListPaneVm {
            title: gist_title,
            focused: state.focus == FocusPane::Gist,
            selected: (gist_empty == ListPaneEmpty::HasRows).then_some(state.gist_index),
            empty: gist_empty,
            empty_message: gist_empty_message,
            rows: gist_rows,
        },
        local_hscroll: state.local_hscroll,
        gist_hscroll: state.gist_hscroll,
        footer,
    }
}

fn list_pane_items(
    pane: &ListPaneVm,
    hscroll: u16,
    theme: &crate::tui::Theme,
) -> Vec<ListItem<'static>> {
    match pane.empty {
        ListPaneEmpty::HasRows => pane
            .rows
            .iter()
            .map(|row| {
                let item = ListItem::new(crate::tui::render::hscroll_str(&row.label, hscroll));
                if matches!(row.mark, crate::ranking::MatchMark::ExactFilename) {
                    item.style(Style::default().add_modifier(Modifier::BOLD))
                } else {
                    item
                }
            })
            .collect(),
        _ => {
            let msg = pane.empty_message.clone().unwrap_or_else(|| "  ".into());
            vec![ListItem::new(msg).style(Style::default().fg(theme.dim))]
        }
    }
}

fn render_pane(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    title: &PaneTitleVm,
    items: Vec<ListItem>,
    focused: bool,
    selected: Option<usize>,
    theme: &crate::tui::Theme,
) -> usize {
    let item_count = items.len();
    // Titles sit between the two border corners; segments that do not fit are dropped here
    // rather than clipped mid-word by the block (#338).
    let title = crate::tui::render::fit_title(title, area.width.saturating_sub(2) as usize);
    let border_style = if focused {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.dim)
    };
    // The border colour alone signals which pane is active; row text stays at full
    // brightness in both panes so it is always legible.
    // Focused selection is a solid bar (whole row); unfocused just bolds the row.
    let highlight_style = if focused {
        Style::default()
            .bg(theme.accent)
            .fg(theme.fg_on_accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title(title.as_str())
                // Pin title to theme fg so it stays legible in both dark and light modes.
                .title_style(Style::default().fg(theme.fg))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style)
                .style(theme.base_style())
                .padding(Padding::horizontal(1)),
        )
        .style(theme.base_style())
        .highlight_style(highlight_style)
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, area, &mut list_state);
    let offset = list_state.offset();

    // Show a scrollbar when the list overflows its viewport.
    let viewport = area.height.saturating_sub(2) as usize;
    if viewport > 0 && item_count > viewport {
        let mut scrollbar_state = ScrollbarState::new(item_count).position(selected.unwrap_or(0));
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
    offset
}

pub(crate) fn render_list_vm(
    frame: &mut Frame,
    state: &AppState,
    list: &ListVm,
    chrome: &ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    let footer_body = match &list.footer {
        ListFooterVm::Hints { text } | ListFooterVm::Status { text } => text.clone(),
        ListFooterVm::Filtering { focus, query } => {
            let pane = match focus {
                FocusPane::Local => "local",
                FocusPane::Gist => "gist",
            };
            // Height sizing uses a plain-text approximation; the painted footer uses `input_line`.
            format!("filter {pane}: {query}_   (Tab next pane · Enter apply · Esc clear)")
        }
    };
    let footer_is_command = matches!(list.footer, ListFooterVm::Hints { .. });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(crate::tui::render::footer_height(
                &footer_body,
                area.width,
                "",
            )),
        ])
        .split(area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    let local_items = list_pane_items(&list.local, list.local_hscroll, &state.theme);
    let local_offset = render_pane(
        frame,
        columns[0],
        &list.local.title,
        local_items,
        list.local.focused,
        list.local.selected,
        &state.theme,
    );
    if chrome.mouse_enabled {
        layout.local = Some(PaneHit {
            rect: columns[0],
            offset: local_offset,
        });
    }

    let gist_items = list_pane_items(&list.gist, list.gist_hscroll, &state.theme);
    let gist_offset = render_pane(
        frame,
        columns[1],
        &list.gist.title,
        gist_items,
        list.gist.focused,
        list.gist.selected,
        &state.theme,
    );
    if chrome.mouse_enabled {
        layout.gist = Some(PaneHit {
            rect: columns[1],
            offset: gist_offset,
        });
    }

    match &list.footer {
        ListFooterVm::Filtering { focus, query } => {
            let pane = match focus {
                FocusPane::Local => "local",
                FocusPane::Gist => "gist",
            };
            let line = crate::tui::render::input_line(
                &format!("filter {pane}: "),
                query,
                "   (Tab next pane · Enter apply · Esc clear)",
            );
            crate::tui::render::render_footer_line(
                frame,
                chunks[1],
                "",
                line,
                &state.theme,
                layout,
            );
        }
        _ => {
            crate::tui::render_footer(
                frame,
                chunks[1],
                "",
                &footer_body,
                footer_is_command,
                &state.theme,
                layout,
            );
        }
    }
}

pub(crate) fn list_palette_items(state: &AppState) -> Vec<crate::tui::palette::PaletteItem> {
    use crate::tui::palette::key_item;
    let g = |code| list_guard(state, code);
    vec![
        key_item(
            "Enter",
            "Diff local ↔ gist",
            KeyCode::Enter,
            g(KeyCode::Enter),
        ),
        key_item(
            "Space",
            "Preview gist content",
            KeyCode::Char(' '),
            g(KeyCode::Char(' ')),
        ),
        key_item(
            "d",
            "Download gist → cwd",
            KeyCode::Char('d'),
            g(KeyCode::Char('d')),
        ),
        key_item(
            "u",
            "Upload local → gist",
            KeyCode::Char('u'),
            g(KeyCode::Char('u')),
        ),
        key_item(
            "n",
            "Create gist from local",
            KeyCode::Char('n'),
            g(KeyCode::Char('n')),
        ),
        key_item(
            "p",
            "Pin / unpin pair",
            KeyCode::Char('p'),
            g(KeyCode::Char('p')),
        ),
        key_item("P", "Open Pins view", KeyCode::Char('P'), true),
        key_item(
            "g",
            "Open Gist manager",
            KeyCode::Char('g'),
            g(KeyCode::Char('g')),
        ),
        key_item(
            "S",
            "Smart-sync pinned pair",
            KeyCode::Char('S'),
            g(KeyCode::Char('S')),
        ),
        key_item(
            "X",
            "Remove file from gist",
            KeyCode::Char('X'),
            g(KeyCode::Char('X')),
        ),
        key_item(
            "e",
            "Edit local file",
            KeyCode::Char('e'),
            g(KeyCode::Char('e')),
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
            "*",
            "Star / unstar gist",
            KeyCode::Char('*'),
            g(KeyCode::Char('*')),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::*;
    use crossterm::event::KeyModifiers;

    use crate::tui::tests::{
        list_state_with_matches, set_pending, state_ready_to_create, state_with_gists,
        state_with_local_paths, state_with_two_gists,
    };

    fn clear_pending(state: &mut AppState) {
        if state.screen.is_confirm() {
            state.screen = Screen::List;
        }
    }

    /// #338: the Local pane title keeps the anchor in its head, puts the sort and filter next,
    /// and hands the cwd to `context` — the only part a narrow pane shortens.
    #[test]
    fn local_title_puts_the_cwd_in_the_shrinkable_context() {
        let mut state = state_with_local_paths(&["/cwd/notes.md", "/cwd/a.txt"]);
        state.cwd = std::path::PathBuf::from("/cwd/some-org/some-project");
        state.local_filter_query = "md".into();
        state.anchor = FocusPane::Local;

        let title = build_list_vm(&state).local.title;
        assert_eq!(
            title.segments,
            vec![
                "[1] Local (1/2) ⚓".to_string(),
                "sort:match".to_string(),
                "/md".to_string(),
            ]
        );
        assert_eq!(
            title.context.as_deref(),
            Some(crate::config::display_path(&state.cwd).as_str())
        );
    }

    /// #338: flipping the anchor must change the pane title. The marker rides in the head, so it
    /// survives at every width the head does — `render::fit_title` owns the width side of that.
    #[test]
    fn anchor_key_puts_the_marker_in_the_title_head() {
        let mut state = state_with_local_paths(&["/cwd/notes.md", "/cwd/a.txt"]);
        state.cwd = std::path::PathBuf::from("/cwd/some-org/some-project");
        state.local_filter_query = "md".into();
        state.anchor = FocusPane::Gist;

        let before = build_list_vm(&state).local.title;
        state.handle_key(KeyCode::Char('a'));
        let after = build_list_vm(&state).local.title;

        assert_eq!(state.anchor, FocusPane::Local);
        assert!(!before.segments[0].contains('⚓'), "{:?}", before.segments);
        assert!(after.segments[0].contains('⚓'), "{:?}", after.segments);
    }

    /// #338: the Gist pane title is built from the same ordered segments, with no context.
    #[test]
    fn gist_title_segments_follow_the_same_order() {
        let mut state = state_with_gists();
        state.filter_query = "a".into();
        state.anchor = FocusPane::Gist;

        let title = build_list_vm(&state).gist.title;
        assert_eq!(
            title.segments,
            vec![
                "[2] Gists (1/2) ⚓".to_string(),
                "all".to_string(),
                "match".to_string(),
                "/a".to_string(),
            ]
        );
        assert_eq!(title.context, None);
    }

    #[test]
    fn esc_in_preview_returns_to_list_and_clears() {
        let mut state = initial_state();
        state.enter_preview(
            "Preview: a / x".into(),
            "raw content".into(),
            Some(("a".into(), "x".into())),
        );
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert!(state.preview().is_none());
    }

    #[test]
    fn back_to_list_clears_preview() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        state.back_to_list();
        assert_eq!(state.screen, Screen::List);
        assert!(!state.diff_previewed());
        assert!(state.diff_body_text().is_empty());
        assert!(state.preview_remote().is_empty());
        assert_eq!(state.preview_local(), PathBuf::new());
        assert_eq!(state.download_target(), PathBuf::new());
    }

    #[test]
    fn identical_diff_disables_download_and_upload() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        if let Some(d) = state.diff_mut() {
            d.identical = true;
        }
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
        assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::None);
        // Scrolling and leaving still work.
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn esc_in_diff_returns_to_list() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert!(!state.diff_previewed());
    }

    #[test]
    fn q_in_diff_returns_to_list() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn o_on_main_list_is_noop_now_that_browser_moved_to_gist_view() {
        let mut state = state_with_two_gists();
        assert_eq!(state.handle_key(KeyCode::Char('o')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn confirm_upload_n_cancels_and_resets_watching() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Upload {
                gist_id: "a".into(),
                filename: "settings.json".into(),
                local_path: PathBuf::from("/tmp/settings.json"),
            },
        );
        state.upload.watching = true;

        assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
        assert!(state.pending_action().is_none());
        assert_eq!(state.screen, Screen::List);
        assert!(
            !state.upload.watching,
            "cancelling must reset watching so a future upload-edit session isn't blocked forever \
             by a stale flag (the background thread is not force-killed and cleans up on its own)"
        );
    }

    #[test]
    fn apply_upload_edit_event_discards_when_context_is_stale() {
        let mut state = initial_state();
        // The user already left Confirm (e.g. cancelled) before this late event arrived.
        state.screen = Screen::List;
        clear_pending(&mut state);
        state.upload.watching = false;
        state.upload.edited_content = None;

        state.apply_upload_edit_event(crate::tui::bg::UploadEditWatchEvent::ContentChanged {
            gist_id: "a".into(),
            filename: "notes.txt".into(),
            content: "should be ignored".into(),
        });

        assert_eq!(state.upload.edited_content, None);
    }

    #[test]
    fn n_without_local_is_noop() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn x_without_gist_is_noop() {
        let mut state = initial_state();
        state.focus = FocusPane::Gist;
        assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn x_on_a_gists_only_file_is_blocked() {
        let mut state = initial_state();
        state.focus = FocusPane::Gist;
        state.gists = vec![GistFile {
            gist_id: "abc123".into(),
            description: String::new(),
            filename: "notes.md".into(),
            public: false,
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        }];
        // Removing the only file would leave a fileless gist, which GitHub forbids.
        assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert!(state.pending_action().is_none());
        assert!(state.status.as_deref().unwrap().contains("only file"));
    }

    #[test]
    fn delete_confirm_n_returns_to_list() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Delete {
                gist_id: "abc123".into(),
                label: "my notes".into(),
            },
        );
        assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert!(state.pending_action().is_none());
    }

    #[test]
    fn g_with_no_gists_is_blocked() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char('g')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn create_confirm_esc_cancels() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Create {
                local_path: PathBuf::from("/tmp/config.toml"),
            },
        );
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert_eq!(state.pending_action().cloned(), None);
    }

    #[test]
    fn create_esc_while_editing_description_cancels() {
        let mut state = state_ready_to_create();
        state.handle_key(KeyCode::Char('n'));
        state.handle_key(KeyCode::Char('x'));
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert_eq!(state.pending_action().cloned(), None);
        assert!(!state.editing_description);
        assert!(state.description_input.is_empty());
    }

    #[test]
    fn lowercase_h_does_not_open_revision_history() {
        let mut state = list_state_with_matches();
        state.focus = FocusPane::Gist;
        state.gist_index = 0;
        assert_eq!(state.handle_key(KeyCode::Char('h')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn scroll_down_moves_focused_list_by_one() {
        let mut state = state_with_local_paths(&["a.rs", "b.rs", "c.rs"]);
        state.screen = Screen::List;
        state.focus = FocusPane::Local;
        state.local_index = 0;
        let out = state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.local_index, 1);
    }

    #[test]
    fn scroll_up_moves_focused_list_by_one() {
        let mut state = state_with_local_paths(&["a.rs", "b.rs", "c.rs"]);
        state.screen = Screen::List;
        state.focus = FocusPane::Local;
        state.local_index = 2;
        let out = state.handle_mouse(MouseInput::ScrollUp, &MouseLayout::default());
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.local_index, 1);
    }

    #[test]
    fn list_click_selects_and_focuses_gist_pane() {
        let mut state = state_with_gists();
        state.screen = Screen::List;
        state.focus = FocusPane::Local;
        state.gist_hscroll = 5;
        let hit = PaneHit {
            rect: Rect::new(20, 0, 20, 10),
            offset: 0,
        };
        let layout = MouseLayout {
            gist: Some(hit),
            ..Default::default()
        };
        // row 2 -> content idx 1 (top border is row 0, row 1 = idx 0, row 2 = idx 1)
        let out = state.handle_mouse(MouseInput::Click { col: 25, row: 2 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.focus, FocusPane::Gist);
        assert_eq!(state.gist_index, 1);
        assert_eq!(state.gist_hscroll, 0);
    }

    #[test]
    fn list_click_selects_and_focuses_local_pane() {
        let mut state = state_with_local_paths(&["a.rs", "b.rs", "c.rs"]);
        state.gists = vec![];
        state.screen = Screen::List;
        state.focus = FocusPane::Gist;
        state.local_hscroll = 5;
        let hit = PaneHit {
            rect: Rect::new(0, 0, 20, 10),
            offset: 0,
        };
        let layout = MouseLayout {
            local: Some(hit),
            ..Default::default()
        };
        // row 1 -> idx 0 (first content row after top border)
        let out = state.handle_mouse(MouseInput::Click { col: 5, row: 1 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.focus, FocusPane::Local);
        assert_eq!(state.local_index, 0);
        assert_eq!(state.local_hscroll, 0);
    }

    #[test]
    fn list_double_click_opens_diff() {
        let mut state = state_with_gists();
        state.screen = Screen::List;
        let hit = PaneHit {
            rect: Rect::new(20, 0, 20, 10),
            offset: 0,
        };
        let layout = MouseLayout {
            gist: Some(hit),
            ..Default::default()
        };
        // row 1 -> idx 0 (first gist)
        let out = state.handle_mouse(MouseInput::DoubleClick { col: 25, row: 1 }, &layout);
        assert_eq!(state.focus, FocusPane::Gist);
        assert_eq!(state.gist_index, 0);
        assert!(matches!(out, KeyOutcome::PreviewDiff { .. }));
    }

    #[test]
    fn click_in_pane_blank_focuses_without_selecting() {
        let mut state = state_with_gists();
        state.screen = Screen::List;
        state.focus = FocusPane::Local;
        state.gist_index = 0;
        let hit = PaneHit {
            rect: Rect::new(20, 0, 20, 4),
            offset: 0,
        };
        let layout = MouseLayout {
            gist: Some(hit),
            ..Default::default()
        };
        // row 0 is the top border (no row there): clicking the gist pane's blank/border area
        // switches focus to it but selects nothing.
        let out = state.handle_mouse(MouseInput::Click { col: 25, row: 0 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.focus, FocusPane::Gist);
        assert_eq!(state.gist_index, 0);
    }

    #[test]
    fn scroll_down_clamps_at_list_end() {
        // Only 1 item in local; scrolling down should clamp (no panic, no index change).
        let mut state = state_with_local_paths(&["a.rs"]);
        state.screen = Screen::List;
        state.focus = FocusPane::Local;
        state.local_index = 0;
        state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
        assert_eq!(state.local_index, 0);
    }

    #[test]
    fn semicolon_opens_menu_palette() {
        let mut state = crate::tui::initial_state();
        state.handle_key(KeyCode::Char(';'));
        assert!(state.screen.is_palette());
        assert_eq!(
            state.palette().unwrap().mode,
            crate::tui::palette::PaletteMode::Menu
        );
        assert_eq!(state.palette().unwrap().origin_screen, Screen::List);
    }

    #[test]
    fn palette_global_openers_do_not_replace_the_active_palette() {
        let mut state = crate::tui::initial_state();
        state.open_palette_menu(None);
        state.handle_key_with(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert!(state.screen.is_palette());
        state.handle_key(KeyCode::Char(';'));
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn open_config_does_not_write_config_file() {
        // Point config_path() at a throwaway XDG dir and assert the *real* path stays absent
        // after open_config() — not an unrelated tempfile the app never uses.
        let _guard = crate::config::tests::ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", dir.path());
        let path = crate::config::config_path().unwrap();
        assert_eq!(path, dir.path().join("gistui").join("config.toml"));
        assert!(!path.exists());

        let mut state = initial_state();
        state.screen = Screen::List;
        state.open_config();
        assert!(state.screen.is_config());
        // Opening alone must not create the file — persist only after a field change.
        assert!(
            !path.exists(),
            "open_config must not create {}",
            path.display()
        );

        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn config_c_key_opens_settings_from_list() {
        let mut state = initial_state();
        state.screen = Screen::List;
        state.handle_key(KeyCode::Char('C'));
        assert!(state.screen.is_config());
        state.handle_key(KeyCode::Esc);
        assert_eq!(state.screen, Screen::List);
    }
}
