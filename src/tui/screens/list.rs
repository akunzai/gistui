//! `Screen::List` — key handling, view-model, paint, and palette items colocated in one
//! file (issue #287, Phase 2).

use crate::ranking::MatchMark;
use crate::tui::keys::{apply_filter_edit, diff_pair_previewable, FilterKey, NavAction};
use crate::tui::render::list_pane::{
    highlight_pane_divider, render_list_pane, ListPaneEmpty, ListPaneVm, RowEmphasis, RowVm,
    MIN_PANE_CELLS,
};
use crate::tui::render::text_fit::PaneTitleVm;
use crate::tui::view_model::ChromeVm;
use crate::tui::{
    AppState, FocusPane, GistView, HelpTopic, HitTarget, KeyOutcome, MouseFrame, PaneTarget,
    PendingAction, RowTarget, Screen, SplitHit,
};
use crossterm::event::KeyCode;

/// Main dual-pane List screen presentation (#250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListVm {
    pub local: ListPaneVm,
    pub gist: ListPaneVm,
    pub footer: ListFooterVm,
    /// Share of the width the local pane gets; paint turns it into cells (issue #395).
    pub split_percent: u16,
    /// The divider is being dragged, so paint highlights it.
    pub split_dragging: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListFooterVm {
    /// Idle command hints (colourised keys).
    Hints { text: String },
    /// One-shot status message (plain).
    Status { text: String },
    /// Inline filter on the focused pane; carries live query text and focus.
    Filtering {
        focus: FocusPane,
        query: crate::tui::text_input::TextInput,
    },
}

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::List;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    1
}

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

/// Share of the List screen's width the local pane starts with, and what a double-click on
/// the divider restores.
pub(crate) const DEFAULT_SPLIT_PERCENT: u16 = 40;

/// Percent band the divider drag is confined to, before the absolute width floor below.
const MIN_SPLIT_PERCENT: u16 = 15;
const MAX_SPLIT_PERCENT: u16 = 85;

/// Cells the local pane gets for `percent` of `width`. The single source of the split
/// geometry: paint sizes the panes with it and the drag clamp reasons in terms of it.
/// Rounds to nearest, matching what ratatui's own percentage constraint used to resolve to.
fn split_cells(percent: u16, width: u16) -> u16 {
    ((u32::from(width) * u32::from(percent) + 50) / 100) as u16
}

/// Clamp a requested split to the percent band and to [`MIN_PANE_CELLS`] on both sides.
/// `None` when `width` cannot hold two readable panes at any split — the caller then leaves
/// the layout alone rather than painting two slits.
fn clamp_split_percent(percent: u16, width: u16) -> Option<u16> {
    if width < MIN_PANE_CELLS * 2 {
        return None;
    }
    let width = u32::from(width);
    let min_cells = u32::from(MIN_PANE_CELLS);
    // Inverting `split_cells`, which rounds at the half cell: `cells >= min` holds from
    // `ceil((min * 100 - 50) / width)` up, and `width - cells >= min` holds up to
    // `floor(((width - min) * 100 + 49) / width)`.
    let low = (min_cells * 100 - 50).div_ceil(width) as u16;
    let high = ((width - min_cells) * 100 + 49).div_euclid(width) as u16;
    let low = low.max(MIN_SPLIT_PERCENT);
    let high = high.min(MAX_SPLIT_PERCENT);
    (low <= high).then(|| percent.clamp(low, high))
}

/// Percent that puts the local pane's right border under `col`. Rounds to the nearest
/// percent: above 100 columns one percent is worth more than one cell, so the divider
/// snaps to the closest reachable column instead of drifting behind the pointer.
fn percent_for_col(area: Rect, col: u16) -> u16 {
    let cells = u32::from(col.saturating_sub(area.x)) + 1;
    let width = u32::from(area.width.max(1));
    ((cells * 100 + width / 2) / width).min(100) as u16
}

/// Shared "would this key actually do something" predicate for `Screen::List`, mirrored by
/// both [`AppState::handle_key_list`]'s match-arm guards and `list_palette_items` so the two
/// can never silently drift (issue #288).
pub(crate) fn list_guard(state: &AppState, code: KeyCode) -> bool {
    let (visible_locals, ranked) = state.list_pane_snapshots();
    let has_gist = ranked.get(state.gist_cursor.index).is_some();
    let has_local = visible_locals.get(state.local_cursor.index).is_some();
    let gist = ranked.get(state.gist_cursor.index);
    let gist_id = gist.map(|g| g.file.gist_id.clone());
    let owned = gist_id
        .as_deref()
        .map(|id| state.gist_is_owned(id))
        .unwrap_or(false);
    let gist_file = gist.map(|g| g.file.clone());
    let pinned_pair = visible_locals
        .get(state.local_cursor.index)
        .zip(gist)
        .is_some_and(|(local, gist)| {
            crate::pins::is_pinned(
                &state.pinned,
                crate::pins::PinKey::new(
                    &local.candidate.path,
                    &gist.file.gist_id,
                    &gist.file.filename,
                ),
            )
        });
    match code {
        KeyCode::Enter => gist_file.as_ref().is_some_and(|f| {
            let local_path = visible_locals
                .get(state.local_cursor.index)
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
        KeyCode::Char('g') => !state.gist_catalog.owned.is_empty(),
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
                self.status = Some("Press q or Esc again to quit (any other key cancels)".into());
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
                self.gist_cursor.reset();
            }
            // Not gated through `list_guard`: `star_toggle_intent` already has its own
            // complete "select a gist first" message for the no-selection case.
            KeyCode::Char('*') => return self.star_toggle_intent(),
            KeyCode::Char('s') => self.cycle_focused_sort(),
            KeyCode::Char('r') => {
                self.local_recursive = !self.local_recursive;
                self.local_cursor.reset();
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
                    entry: self.defer_entry(),
                    local_path: local.path.clone(),
                    gist_id: gist.file.gist_id.clone(),
                    filename: gist.file.filename.clone(),
                };
            }
            KeyCode::Char('g') => self.open_gist_manager(),
            KeyCode::Char('H') if list_guard(self, code) => {
                if self.open_revisions() {
                    if let Some(gist_id) = self.revision().and_then(|r| r.gist_id.clone()) {
                        return KeyOutcome::Revision(
                            crate::tui::gist_revision::RevisionRequest::FetchHistory { gist_id },
                        );
                    }
                }
            }
            KeyCode::Char('H') => {
                self.status = Some("select a gist file to view revision history".into());
            }
            KeyCode::Char('e') if list_guard(self, code) => {
                let (locals, _) = self.list_pane_snapshots();
                if let Some(local) = locals.get(self.local_cursor.index) {
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
                let Some(gist) = ranked.get(self.gist_cursor.index) else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::PreviewContent {
                    entry: self.defer_entry(),
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
                if let Some(gist) = ranked.get(self.gist_cursor.index) {
                    self.block_if_non_previewable_gist_file(
                        &gist.file.gist_id,
                        &gist.file.filename,
                    );
                }
            }
            KeyCode::Char('d') if list_guard(self, code) => {
                let (_, ranked) = self.list_pane_snapshots();
                if let Some(gist) = ranked.get(self.gist_cursor.index) {
                    let filename = gist.file.filename.clone();
                    return KeyOutcome::DownloadGist {
                        entry: self.defer_entry(),
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
                let Some(gist) = ranked.get(self.gist_cursor.index) else {
                    return KeyOutcome::None;
                };
                let local_path = locals
                    .get(self.local_cursor.index)
                    .map(|r| r.candidate.path.clone());
                let filename = gist.file.filename.clone();
                let target = local_path
                    .as_deref()
                    .and_then(std::path::Path::parent)
                    .unwrap_or(&self.cwd)
                    .join(&filename);
                return KeyOutcome::PreviewDiff {
                    entry: self.defer_entry(),
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
                if let Some(gist) = ranked.get(self.gist_cursor.index) {
                    let local_path = locals
                        .get(self.local_cursor.index)
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
        self.focused_cursor_mut().reset();
    }

    /// Pin/unpin the selected local↔gist pair: returns [`KeyOutcome::Unpin`] when the exact
    /// pair is already pinned, otherwise [`KeyOutcome::Pin`]. Requires a selection in both
    /// panes; otherwise it just sets a status hint.
    fn pin_toggle_intent(&mut self) -> KeyOutcome {
        let (locals, ranked) = self.list_pane_snapshots();
        let (Some(local), Some(gist)) = (
            locals.get(self.local_cursor.index).map(|r| &r.candidate),
            ranked.get(self.gist_cursor.index),
        ) else {
            self.status = Some("select a local file and a gist to pin".into());
            return KeyOutcome::None;
        };
        let local_path = local.path.clone();
        let gist_id = gist.file.gist_id.clone();
        let filename = gist.file.filename.clone();
        let already = crate::pins::is_pinned(
            &self.pinned,
            crate::pins::PinKey::new(&local_path, &gist_id, &filename),
        );
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
            NavAction::Left => self.focused_cursor_mut().left(),
            NavAction::Right => {
                // Bound computed before the cursor borrow (issue #274); it is the painted
                // width of the *selected* row only (issue #341).
                let hmax = self.focused_hscroll_max();
                self.focused_cursor_mut().right(hmax);
            }
        }
        true
    }

    /// The divider under (`col`, `row`), if it is there *and* the terminal is wide enough for
    /// a resize to mean anything. Both the grab and the double-click reset go through it, so
    /// a too-narrow terminal swallows neither press — they fall through to the normal
    /// focus/select handling instead of being eaten by a gesture that can only be clamped
    /// away. It lives here rather than on `MouseFrame` because the width policy is the List
    /// screen's (`clamp_split_percent`), not the hit map's.
    fn grabbable_divider(&self, col: u16, row: u16, layout: &MouseFrame) -> bool {
        layout.split().is_some_and(|hit| {
            hit.grabbed(col, row)
                && clamp_split_percent(self.list_split_percent, hit.area.width).is_some()
        })
    }

    /// Grab the divider if the press landed on it, starting a drag. Returns `true` when
    /// grabbed, so the caller can skip the click's usual focus/select handling.
    pub(crate) fn grab_split_divider(&mut self, col: u16, row: u16, layout: &MouseFrame) -> bool {
        let grabbed = self.grabbable_divider(col, row, layout);
        if grabbed {
            self.mouse_session.begin_divider_drag();
        }
        grabbed
    }

    /// Move the divider under `col` while a drag is in progress. Only `col` is consulted:
    /// letting the pointer wander above or below the panes must not break the drag.
    pub(crate) fn drag_split_divider(&mut self, col: u16, layout: &MouseFrame) {
        if !self.mouse_session.is_dragging() {
            return;
        }
        let Some(hit) = layout.split() else {
            return;
        };
        if let Some(percent) = clamp_split_percent(percent_for_col(hit.area, col), hit.area.width) {
            self.list_split_percent = percent;
        }
    }

    /// End any divider drag. Called on mouse-up and whenever a background-task overlay
    /// takes over the mouse, which would otherwise swallow the release and wedge the drag.
    /// Restore the default split — the double-click action on the divider. Returns `true`
    /// when the double-click landed on it.
    pub(crate) fn reset_split_divider(&mut self, col: u16, row: u16, layout: &MouseFrame) -> bool {
        let grabbed = self.grabbable_divider(col, row, layout);
        if grabbed {
            self.list_split_percent = DEFAULT_SPLIT_PERCENT;
            self.mouse_session.interrupt();
        }
        grabbed
    }

    /// Select the clicked row on `Screen::List`, focusing its pane. Returns `true` when a row
    /// was hit (so a double-click should "open" it). A click in a pane's blank area or border
    /// focuses it but selects nothing (returns `false`); a click off every list returns `false`.
    pub(crate) fn click_select_list(&mut self, target: RowTarget) -> bool {
        let RowTarget::Pane { pane, index } = target else {
            return false;
        };
        // A click anywhere in a pane (incl. blank/border) focuses it; a click on a row
        // also selects it.
        match pane {
            PaneTarget::Local => {
                self.focus = FocusPane::Local;
                let Some(idx) = index else {
                    return false;
                };
                self.local_cursor.select(idx);
                if self.anchor == FocusPane::Local {
                    self.reset_ranked_pane();
                }
                true
            }
            PaneTarget::Gist => {
                self.focus = FocusPane::Gist;
                let Some(idx) = index else {
                    return false;
                };
                self.gist_cursor.select(idx);
                if self.anchor == FocusPane::Gist {
                    self.reset_ranked_pane();
                }
                true
            }
            PaneTarget::List | PaneTarget::DetailFiles => false,
        }
    }
}

/// The ` ⚑` suffix for whichever pane drives the match ranking, empty for the other one.
/// Single-width on purpose — see the mark vocabulary in `docs/design.md`.
fn anchor_marker(state: &AppState, pane: FocusPane) -> &'static str {
    if state.anchor == pane {
        " ⚑"
    } else {
        ""
    }
}

/// List body only — usable while `state.screen` is List **or** Palette-over-List (#250).
pub(crate) fn build_list_vm(state: &AppState) -> ListVm {
    let (visible_locals, ranked) = state.list_pane_snapshots();

    let (local_empty, local_empty_message, local_rows) =
        if state.local_scanning() && state.locals.is_empty() {
            (
                ListPaneEmpty::Loading,
                Some(format!(
                    "  {} Scanning files…",
                    crate::tui::render::spinner_glyph(state.spinner_frame)
                )),
                Vec::new(),
            )
        } else if state.locals.is_empty() {
            (
                ListPaneEmpty::NoItems,
                Some("  No local files found".into()),
                Vec::new(),
            )
        } else if visible_locals.is_empty() {
            (
                ListPaneEmpty::NoFilterMatch,
                Some("  No files match the filter".into()),
                Vec::new(),
            )
        } else {
            (
                ListPaneEmpty::HasRows,
                None,
                visible_locals
                    .iter()
                    .map(|r| {
                        let base = crate::tui::text::local_row_label(&r.candidate.path, &state.cwd);
                        RowVm {
                            label: crate::tui::render::marked_row_text(base, r.mark),
                            emphasis: row_emphasis(r.mark),
                        }
                    })
                    .collect(),
            )
        };

    let recursive_marker = if state.local_recursive { " [↓]" } else { "" };
    let scanning_marker = if state.local_scanning() { " …" } else { "" };
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
            gist_empty_message = Some("  No gists match the filter".into());
        } else {
            gist_empty = ListPaneEmpty::NoItems;
            gist_empty_message = Some("  No gists found".into());
        }
        gist_rows = Vec::new();
    } else {
        gist_empty = ListPaneEmpty::HasRows;
        gist_empty_message = None;
        gist_rows = ranked
            .iter()
            .map(|g| {
                let base = crate::tui::gist_row_display(g, state.gist_view, state);
                RowVm {
                    label: crate::tui::render::marked_row_text(base, g.mark),
                    emphasis: row_emphasis(g.mark),
                }
            })
            .collect();
    }

    // Same order as the Local pane (#338). No re-derivable context here, so nothing is ever
    // shortened — a segment either fits whole or is dropped.
    let gist_head = |name: &str| {
        format!(
            "[2] {name}{}{}",
            crate::tui::render::count_label(ranked.len(), state.gist_catalog.owned.len()),
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
            text: crate::tui::keymap::footer_hints(&Screen::List),
        }
    };

    ListVm {
        local: ListPaneVm {
            title: local_title,
            focused: state.focus == FocusPane::Local,
            selected: (local_empty == ListPaneEmpty::HasRows).then_some(state.local_cursor.index),
            empty: local_empty,
            empty_message: local_empty_message,
            rows: local_rows,
            hscroll: state.local_cursor.hscroll,
            scrollbar: true,
        },
        gist: ListPaneVm {
            title: gist_title,
            focused: state.focus == FocusPane::Gist,
            selected: (gist_empty == ListPaneEmpty::HasRows).then_some(state.gist_cursor.index),
            empty: gist_empty,
            empty_message: gist_empty_message,
            rows: gist_rows,
            hscroll: state.gist_cursor.hscroll,
            scrollbar: true,
        },
        footer,
        split_percent: state.list_split_percent,
        split_dragging: state.mouse_session.is_dragging(),
    }
}

/// A matched filename is the only List row that stands out on its own; the pin mark rides in
/// the label instead (see `render::marked_row_text`).
fn row_emphasis(mark: MatchMark) -> RowEmphasis {
    if matches!(mark, MatchMark::ExactFilename) {
        RowEmphasis::Strong
    } else {
        RowEmphasis::None
    }
}

pub(crate) fn render_list_vm(
    frame: &mut Frame,
    state: &AppState,
    list: &ListVm,
    chrome: &ChromeVm,
    layout: &mut MouseFrame,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(
        frame,
        area,
        &state.settings.theme(),
        chrome.mouse_enabled,
        layout,
    );
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
                footer_is_command,
            )),
        ])
        .split(area);
    // Percent is the stored fact, but the panes are sized in cells: letting ratatui round a
    // second percentage makes the divider lag a cell behind the pointer mid-drag (#395).
    // A terminal too narrow for two readable panes keeps the default split.
    let split_percent =
        clamp_split_percent(list.split_percent, chunks[0].width).unwrap_or(DEFAULT_SPLIT_PERCENT);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(split_cells(split_percent, chunks[0].width)),
            Constraint::Min(0),
        ])
        .split(chunks[0]);

    render_list_pane(
        frame,
        columns[0],
        &list.local,
        &state.settings.theme(),
        chrome.mouse_enabled,
        layout,
        PaneTarget::Local,
    );
    render_list_pane(
        frame,
        columns[1],
        &list.gist,
        &state.settings.theme(),
        chrome.mouse_enabled,
        layout,
        PaneTarget::Gist,
    );

    let divider_x = columns[0].right().saturating_sub(1);
    if list.split_dragging {
        highlight_pane_divider(frame, chunks[0], divider_x, state.settings.theme().accent);
    }
    if chrome.mouse_enabled {
        let split = SplitHit {
            area: chunks[0],
            divider_x,
        };
        layout.register(HitTarget::Divider(split), split.area);
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
                &state.settings.theme(),
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
                crate::tui::keymap::for_screen(&state.screen),
                &state.settings.theme(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::screens::ScreenVm;
    use crate::tui::test_support::{
        list_state_with_matches, set_pending, state_ready_to_create, state_with_gists,
        state_with_local_paths, state_with_selection, state_with_two_gists,
    };
    use crate::tui::text::{hscroll_max_for_text, local_row_label, text_len};
    use crate::tui::*;
    use crossterm::event::KeyCode;
    use crossterm::event::KeyModifiers;
    use std::path::PathBuf;

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
                "[1] Local (1/2) ⚑".to_string(),
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
        assert!(!before.segments[0].contains('⚑'), "{:?}", before.segments);
        assert!(after.segments[0].contains('⚑'), "{:?}", after.segments);
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
                "[2] Gists (1/2) ⚑".to_string(),
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
            Some(crate::domain::GistFileRef::id_name("a", "x")),
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
        assert!(state.scroll_body().is_none());
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
        state.gist_catalog.owned = vec![GistFile {
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            ..GistFile::fixture("abc123", "notes.md")
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
        state.gist_cursor.index = 0;
        assert_eq!(state.handle_key(KeyCode::Char('h')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn scroll_down_moves_focused_list_by_one() {
        let mut state = state_with_local_paths(&["a.rs", "b.rs", "c.rs"]);
        state.screen = Screen::List;
        state.focus = FocusPane::Local;
        state.local_cursor.index = 0;
        let out = state.handle_mouse(MouseInput::ScrollDown, &MouseFrame::default());
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.local_cursor.index, 1);
    }

    #[test]
    fn scroll_up_moves_focused_list_by_one() {
        let mut state = state_with_local_paths(&["a.rs", "b.rs", "c.rs"]);
        state.screen = Screen::List;
        state.focus = FocusPane::Local;
        state.local_cursor.index = 2;
        let out = state.handle_mouse(MouseInput::ScrollUp, &MouseFrame::default());
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.local_cursor.index, 1);
    }

    #[test]
    fn list_click_selects_and_focuses_gist_pane() {
        let mut state = state_with_gists();
        state.screen = Screen::List;
        state.focus = FocusPane::Local;
        state.gist_cursor.hscroll = 5;
        let hit = PaneHit {
            rect: Rect::new(20, 0, 20, 10),
            offset: 0,
        };
        let mut layout = MouseFrame::default();
        layout.register_pane(PaneTarget::Gist, hit, 2);
        // row 2 -> content idx 1 (top border is row 0, row 1 = idx 0, row 2 = idx 1)
        let out = state.handle_mouse(MouseInput::Click { col: 25, row: 2 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.focus, FocusPane::Gist);
        assert_eq!(
            state.gist_cursor,
            ListCursor {
                index: 1,
                hscroll: 0
            }
        );
    }

    #[test]
    fn list_click_selects_and_focuses_local_pane() {
        let mut state = state_with_local_paths(&["a.rs", "b.rs", "c.rs"]);
        state.gist_catalog.owned = vec![];
        state.screen = Screen::List;
        state.focus = FocusPane::Gist;
        state.local_cursor.hscroll = 5;
        let hit = PaneHit {
            rect: Rect::new(0, 0, 20, 10),
            offset: 0,
        };
        let mut layout = MouseFrame::default();
        layout.register_pane(PaneTarget::Local, hit, 3);
        // row 1 -> idx 0 (first content row after top border)
        let out = state.handle_mouse(MouseInput::Click { col: 5, row: 1 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.focus, FocusPane::Local);
        // A click is a selection: it clears the offset that belonged to the old row.
        assert_eq!(state.local_cursor, ListCursor::default());
    }

    #[test]
    fn list_double_click_opens_diff() {
        let mut state = state_with_gists();
        state.screen = Screen::List;
        let hit = PaneHit {
            rect: Rect::new(20, 0, 20, 10),
            offset: 0,
        };
        let mut layout = MouseFrame::default();
        layout.register_pane(PaneTarget::Gist, hit, 2);
        // row 1 -> idx 0 (first gist)
        let out = state.handle_mouse(MouseInput::DoubleClick { col: 25, row: 1 }, &layout);
        assert_eq!(state.focus, FocusPane::Gist);
        assert_eq!(state.gist_cursor.index, 0);
        assert!(matches!(out, KeyOutcome::PreviewDiff { .. }));
    }

    #[test]
    fn click_in_pane_blank_focuses_without_selecting() {
        let mut state = state_with_gists();
        state.screen = Screen::List;
        state.focus = FocusPane::Local;
        state.gist_cursor.index = 0;
        let hit = PaneHit {
            rect: Rect::new(20, 0, 20, 4),
            offset: 0,
        };
        let mut layout = MouseFrame::default();
        layout.register_pane(PaneTarget::Gist, hit, 2);
        // row 0 is the top border (no row there): clicking the gist pane's blank/border area
        // switches focus to it but selects nothing.
        let out = state.handle_mouse(MouseInput::Click { col: 25, row: 0 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.focus, FocusPane::Gist);
        assert_eq!(state.gist_cursor.index, 0);
    }

    #[test]
    fn scroll_down_clamps_at_list_end() {
        // Only 1 item in local; scrolling down should clamp (no panic, no index change).
        let mut state = state_with_local_paths(&["a.rs"]);
        state.screen = Screen::List;
        state.focus = FocusPane::Local;
        state.local_cursor.index = 0;
        state.handle_mouse(MouseInput::ScrollDown, &MouseFrame::default());
        assert_eq!(state.local_cursor.index, 0);
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

    #[test]
    fn local_filter_matches_filename_and_relative_path() {
        let mut state =
            state_with_local_paths(&["/cwd/settings.json", "/cwd/src/main.rs", "/cwd/notes.txt"]);

        assert_eq!(state.visible_locals().len(), 3);

        state.local_filter_query = "json".into();
        let visible: Vec<_> = state
            .visible_locals()
            .iter()
            .map(|r| r.candidate.path.clone())
            .collect();
        assert_eq!(visible, vec![PathBuf::from("/cwd/settings.json")]);

        state.local_filter_query = "src/".into();
        let visible: Vec<_> = state
            .visible_locals()
            .iter()
            .map(|r| r.candidate.path.clone())
            .collect();
        assert_eq!(visible, vec![PathBuf::from("/cwd/src/main.rs")]);

        state.local_filter_query = "NOTES".into();
        assert_eq!(state.visible_locals().len(), 1);
    }

    #[test]
    fn local_down_clamps_to_filtered_count() {
        let mut state = state_with_local_paths(&["/cwd/a.json", "/cwd/b.txt", "/cwd/c.txt"]);
        state.focus = FocusPane::Local;
        state.local_filter_query = "json".into(); // only 1 match

        state.handle_key(KeyCode::Down); // would move to index 1 if clamped on raw len
        assert_eq!(state.local_cursor.index, 0); // clamped: only one visible row
    }

    #[test]
    fn anchor_defaults_to_local() {
        assert_eq!(initial_state().anchor, FocusPane::Local);
    }

    #[test]
    fn a_key_toggles_anchor_and_resets_ranked_pane() {
        let mut state = list_state_with_matches();
        assert_eq!(state.anchor, FocusPane::Local);
        state.local_cursor.index = 1;
        state.local_cursor.hscroll = 3;
        state.handle_key(KeyCode::Char('a'));
        assert_eq!(state.anchor, FocusPane::Gist);
        // anchor now Gist → local is the newly-ranked (non-driver) pane → reset to top.
        assert_eq!(state.local_cursor, ListCursor::default());
    }

    #[test]
    fn a_key_toggle_reverse_direction_resets_gist() {
        let mut state = list_state_with_matches();
        state.anchor = FocusPane::Gist;
        state.gist_cursor.index = 1;
        state.gist_cursor.hscroll = 4;
        state.handle_key(KeyCode::Char('a'));
        assert_eq!(state.anchor, FocusPane::Local);
        assert_eq!(state.gist_cursor, ListCursor::default());
    }

    #[test]
    fn moving_driver_pane_up_resets_ranked_pane() {
        let mut state = list_state_with_matches();
        state.anchor = FocusPane::Local;
        state.focus = FocusPane::Local;
        state.local_cursor.index = 1; // >0 so Up fires
        state.gist_cursor.index = 1;
        state.handle_key(KeyCode::Up);
        assert_eq!(state.local_cursor.index, 0);
        assert_eq!(state.gist_cursor.index, 0);
    }

    #[test]
    fn moving_ranked_pane_does_not_reset_driver() {
        let mut state = list_state_with_matches();
        state.anchor = FocusPane::Local; // Local drives
        state.local_cursor.index = 0;
        state.focus = FocusPane::Gist; // picking in the ranked gist pane
        state.handle_key(KeyCode::Down);
        assert_eq!(state.gist_cursor.index, 1);
        assert_eq!(state.local_cursor.index, 0); // driver NOT reset
    }

    #[test]
    fn moving_driver_pane_resets_ranked_pane() {
        let mut state = list_state_with_matches();
        state.anchor = FocusPane::Local;
        state.focus = FocusPane::Local; // moving the driver
        state.gist_cursor.index = 1;
        state.handle_key(KeyCode::Down);
        assert_eq!(state.local_cursor.index, 1);
        assert_eq!(state.gist_cursor.index, 0); // ranked pane reset to top
    }

    #[test]
    fn tab_switches_focus() {
        let mut state = initial_state();
        assert_eq!(state.focus, FocusPane::Local);
        state.handle_key(KeyCode::Tab);
        assert_eq!(state.focus, FocusPane::Gist);
    }

    #[test]
    fn digit_keys_jump_to_a_pane() {
        let mut state = initial_state();
        state.handle_key(KeyCode::Char('2'));
        assert_eq!(state.focus, FocusPane::Gist);
        state.handle_key(KeyCode::Char('1'));
        assert_eq!(state.focus, FocusPane::Local);
    }

    #[test]
    fn t_toggles_gist_view() {
        let mut state = initial_state();
        assert_eq!(state.gist_view, GistView::Description);
        state.handle_key(KeyCode::Char('t'));
        assert_eq!(state.gist_view, GistView::Id);
        state.handle_key(KeyCode::Char('t'));
        assert_eq!(state.gist_view, GistView::Description);
    }

    #[test]
    fn v_cycles_gist_type_filter() {
        let mut state = initial_state();
        assert_eq!(state.gist_type_filter, GistTypeFilter::All);
        state.handle_key(KeyCode::Char('v'));
        assert_eq!(state.gist_type_filter, GistTypeFilter::Public);
        state.handle_key(KeyCode::Char('v'));
        assert_eq!(state.gist_type_filter, GistTypeFilter::Secret);
        state.handle_key(KeyCode::Char('v'));
        assert_eq!(state.gist_type_filter, GistTypeFilter::Starred);
        state.handle_key(KeyCode::Char('v'));
        assert_eq!(state.gist_type_filter, GistTypeFilter::Forked);
        state.handle_key(KeyCode::Char('v'));
        assert_eq!(state.gist_type_filter, GistTypeFilter::All);
    }

    #[test]
    fn s_cycles_gist_sort_when_gist_pane_focused() {
        let mut state = initial_state();
        state.focus = FocusPane::Gist;
        assert_eq!(state.gist_sort, GistSort::Match);
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(state.gist_sort, GistSort::Name);
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(state.gist_sort, GistSort::Recent);
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(state.gist_sort, GistSort::Match);
        // The local sort is untouched while the gist pane is focused.
        assert_eq!(state.local_sort, LocalSort::Match);
    }

    #[test]
    fn s_cycles_local_sort_when_local_pane_focused() {
        let mut state = initial_state();
        assert_eq!(state.focus, FocusPane::Local);
        assert_eq!(state.local_sort, LocalSort::Match);
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(state.local_sort, LocalSort::Name);
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(state.local_sort, LocalSort::Recent);
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(state.local_sort, LocalSort::Match);
        // The gist sort is untouched while the local pane is focused.
        assert_eq!(state.gist_sort, GistSort::Match);
    }

    #[test]
    fn slash_enters_filter_mode_and_typing_filters() {
        let mut state = state_with_two_gists();
        assert!(!state.filtering);
        state.handle_key(KeyCode::Char('/'));
        assert!(state.filtering);
        // Type "ghostty" -> matches only the first gist (by filename + description).
        for c in "ghostty".chars() {
            state.handle_key(KeyCode::Char(c));
        }
        let ranked = state.ranked_gists();
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].file.gist_id, "a");
    }

    #[test]
    fn filter_matches_description_case_insensitively() {
        let mut state = state_with_two_gists();
        state.filter_query = "SSH".into();
        let ranked = state.ranked_gists();
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].file.gist_id, "b");
    }

    #[test]
    fn filter_enter_keeps_query_esc_clears() {
        let mut state = state_with_two_gists();
        state.handle_key(KeyCode::Char('/'));
        state.handle_key(KeyCode::Char('s'));
        state.handle_key(KeyCode::Char('s'));
        state.handle_key(KeyCode::Char('h'));
        state.handle_key(KeyCode::Enter);
        assert!(!state.filtering);
        assert_eq!(state.filter_query, "ssh");
        // Re-enter and Esc clears.
        state.handle_key(KeyCode::Char('/'));
        state.handle_key(KeyCode::Esc);
        assert!(!state.filtering);
        assert!(state.filter_query.is_empty());
    }

    #[test]
    fn filter_backspace_deletes_last_char() {
        let mut state = state_with_two_gists();
        state.handle_key(KeyCode::Char('/'));
        state.handle_key(KeyCode::Char('x'));
        state.handle_key(KeyCode::Char('y'));
        state.handle_key(KeyCode::Backspace);
        assert_eq!(state.filter_query, "x");
    }

    #[test]
    fn space_on_selected_gist_returns_preview_content() {
        let mut state = state_with_two_gists();
        assert!(matches!(
            state.handle_key(KeyCode::Char(' ')),
            KeyOutcome::PreviewContent { .. }
        ));
    }

    #[test]
    fn space_blocks_preview_for_image_gist_file() {
        let mut state = state_with_two_gists();
        state.gist_catalog.owned[0].filename = "logo.png".into();
        state.gist_catalog.owned[0].content_type = Some("image/png".into());
        assert_eq!(state.handle_key(KeyCode::Char(' ')), KeyOutcome::None);
        assert!(state
            .status
            .as_deref()
            .is_some_and(|s| s.contains("image file")));
    }

    #[test]
    fn enter_blocks_diff_for_image_gist_file() {
        let mut state = state_with_two_gists();
        state.gist_catalog.owned[0].filename = "photo.jpg".into();
        state.gist_catalog.owned[0].content_type = Some("image/jpeg".into());
        assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
        assert!(state
            .status
            .as_deref()
            .is_some_and(|s| s.contains("image file")));
    }

    #[test]
    fn space_without_gist_is_noop() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char(' ')), KeyOutcome::None);
    }

    #[test]
    fn left_right_scrolls_focused_gist_pane() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![GistFile {
            description: "a fairly long description for scrolling".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "f.json")
        }];
        state.focus = FocusPane::Gist;
        assert_eq!(state.gist_cursor.hscroll, 0);
        state.handle_key(KeyCode::Left); // saturates at 0
        assert_eq!(state.gist_cursor.hscroll, 0);
        state.handle_key(KeyCode::Right);
        state.handle_key(KeyCode::Right);
        assert_eq!(state.gist_cursor.hscroll, 2);
        state.handle_key(KeyCode::Left);
        assert_eq!(state.gist_cursor.hscroll, 1);
    }

    #[test]
    fn gist_hscroll_caps_at_painted_row() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![GistFile {
            description: "tiny".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "f")
        }];
        state.focus = FocusPane::Gist;
        // Cap must use the painted display string (star / pin prefixes included), not the
        // star-less label helper — see issue #247.
        let ranked = state.ranked_gists();
        let row = marked_row_text(
            gist_row_display(&ranked[0], state.gist_view, &state),
            ranked[0].mark,
        );
        let max = hscroll_max_for_text(&row);
        for _ in 0..200 {
            state.handle_key(KeyCode::Right);
        }
        assert_eq!(state.gist_cursor.hscroll, max);
    }

    #[test]
    fn gist_hscroll_follows_the_selected_row() {
        let mut state = initial_state();
        state.gist_sort = GistSort::Name;
        state.gist_catalog.owned = vec![
            GistFile {
                description: "ab".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("short", "a.txt")
            },
            GistFile {
                description: "a fairly long description for scrolling".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("long", "b.txt")
            },
        ];
        state.focus = FocusPane::Gist;
        let ranked = state.ranked_gists();
        let short_max = hscroll_max_for_text(&marked_row_text(
            gist_row_display(&ranked[0], state.gist_view, &state),
            ranked[0].mark,
        ));
        let long_max = hscroll_max_for_text(&marked_row_text(
            gist_row_display(&ranked[1], state.gist_view, &state),
            ranked[1].mark,
        ));
        assert!(
            short_max < long_max,
            "fixture must make a.txt shorter than b.txt"
        );
        for _ in 0..200 {
            state.handle_key(KeyCode::Right);
        }
        assert_eq!(
            state.gist_cursor.hscroll, short_max,
            "Right must stop at the selected row, not the longest row in the pane"
        );
        state.handle_key(KeyCode::Down);
        for _ in 0..200 {
            state.handle_key(KeyCode::Right);
        }
        assert_eq!(state.gist_cursor.hscroll, long_max);
        state.handle_key(KeyCode::Up);
        assert_eq!(state.gist_cursor.index, 0);
        assert!(
            state.gist_cursor.hscroll <= short_max,
            "selected row must not stay scrolled past its own content (hscroll {}, max {})",
            state.gist_cursor.hscroll,
            short_max
        );
    }

    #[test]
    fn local_hscroll_caps_at_selected_row_not_the_longest() {
        let mut state = state_with_local_paths(&[
            "/cwd/ab.txt",
            "/cwd/a-fairly-long-filename-for-scrolling.md",
        ]);
        state.focus = FocusPane::Local;
        state.local_cursor.index = 0;
        let locals = state.visible_locals();
        let short_row = marked_row_text(
            local_row_label(&locals[0].candidate.path, &state.cwd),
            locals[0].mark,
        );
        let long_row = marked_row_text(
            local_row_label(&locals[1].candidate.path, &state.cwd),
            locals[1].mark,
        );
        let short_max = hscroll_max_for_text(&short_row);
        let long_max = hscroll_max_for_text(&long_row);
        assert!(
            short_max < long_max,
            "fixture must make the selected row shorter than its sibling"
        );
        for _ in 0..200 {
            state.handle_key(KeyCode::Right);
        }
        assert_eq!(
            state.local_cursor.hscroll, short_max,
            "Right must stop at the selected local row, not the longest row in the pane"
        );
    }

    #[test]
    fn gist_hscroll_caps_include_star_prefix() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![GistFile {
            description: "tiny".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("starred-id", "f")
        }];
        state.gist_catalog.starred_ids.insert("starred-id".into());
        state.focus = FocusPane::Gist;

        let ranked = state.ranked_gists();
        let display = gist_row_display(&ranked[0], state.gist_view, &state);
        assert!(
            display.starts_with("★ "),
            "display must include star prefix, got {display:?}"
        );
        let label = gist_row_label(&ranked[0], state.gist_view);
        assert!(
            !label.starts_with('★'),
            "label helper stays star-less for pure text tests"
        );
        // Regression: measuring the label (no star) under-scrolled by 2 chars.
        assert_eq!(text_len(&display), text_len(&label) + 2);

        let row = marked_row_text(display, ranked[0].mark);
        let max = hscroll_max_for_text(&row);
        let label_only_max = hscroll_max_for_text(&label);
        assert!(max > label_only_max, "star must raise the hscroll cap");

        for _ in 0..200 {
            state.handle_key(KeyCode::Right);
        }
        assert_eq!(
            state.gist_cursor.hscroll, max,
            "Right must reach the display-string max, not the star-less label max"
        );
    }

    #[test]
    fn moving_gist_selection_resets_hscroll() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![
            GistFile {
                description: "first long description here".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("a", "a.json")
            },
            GistFile {
                description: "second long description here".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("b", "b.json")
            },
        ];
        state.focus = FocusPane::Gist;
        state.handle_key(KeyCode::Right);
        assert_eq!(
            state.gist_cursor,
            ListCursor {
                index: 0,
                hscroll: 1
            }
        );
        state.handle_key(KeyCode::Down);
        // The vertical move clears the offset in the same step it moves the selection.
        assert_eq!(
            state.gist_cursor,
            ListCursor {
                index: 1,
                hscroll: 0
            }
        );
    }

    #[test]
    fn enter_with_no_local_but_gist_selected_returns_preview() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![GistFile {
            description: "first".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "alpha.json")
        }];
        state.focus = FocusPane::Gist;
        assert!(state.locals.is_empty());
        assert!(matches!(
            state.handle_key(KeyCode::Enter),
            KeyOutcome::PreviewDiff { .. }
        ));
    }

    #[test]
    fn changing_local_selection_resets_gist_index() {
        let mut state = initial_state();
        state.locals = vec![
            LocalCandidate {
                path: PathBuf::from("/tmp/a.json"),
                modified: None,
            },
            LocalCandidate {
                path: PathBuf::from("/tmp/b.json"),
                modified: None,
            },
        ];
        state.gist_cursor.index = 2;
        state.handle_key(KeyCode::Down); // move local selection down
        assert_eq!(state.gist_cursor.index, 0);
    }

    #[test]
    fn enter_in_gist_focus_with_selection_returns_preview() {
        let mut state = state_with_selection();
        assert!(matches!(
            state.handle_key(KeyCode::Enter),
            KeyOutcome::PreviewDiff { .. }
        ));
    }

    #[test]
    fn enter_with_nested_local_targets_its_directory() {
        let mut state = state_with_selection();
        state.cwd = PathBuf::from("/tmp");
        state.locals[0].path = PathBuf::from("/tmp/nested/settings.json");

        let KeyOutcome::PreviewDiff {
            local_path, target, ..
        } = state.handle_key(KeyCode::Enter)
        else {
            panic!("expected PreviewDiff");
        };

        assert_eq!(local_path, Some(PathBuf::from("/tmp/nested/settings.json")));
        assert_eq!(target, PathBuf::from("/tmp/nested/settings.json"));
    }

    #[test]
    fn enter_in_local_focus_previews_top_gist() {
        let mut state = state_with_selection();
        state.focus = FocusPane::Local;
        assert!(matches!(
            state.handle_key(KeyCode::Enter),
            KeyOutcome::PreviewDiff { .. }
        ));
    }

    #[test]
    fn enter_with_no_gists_is_noop_in_local_focus() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/x"),
            modified: None,
        }];
        state.focus = FocusPane::Local;
        assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
    }

    #[test]
    fn d_in_gist_focus_returns_download_gist() {
        let mut state = state_with_selection();
        assert!(matches!(
            state.handle_key(KeyCode::Char('d')),
            KeyOutcome::DownloadGist { .. }
        ));
    }

    #[test]
    fn d_in_local_focus_is_noop() {
        let mut state = state_with_selection();
        state.focus = FocusPane::Local;
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
    }

    #[test]
    fn d_without_gists_is_noop() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/x"),
            modified: None,
        }];
        state.focus = FocusPane::Gist;
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
    }

    #[test]
    fn enter_without_gists_is_noop() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/x"),
            modified: None,
        }];
        state.focus = FocusPane::Gist;
        assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
    }

    #[test]
    fn p_pins_unpinned_pair_then_unpins() {
        let mut state = state_with_selection();
        assert!(matches!(
            state.handle_key(KeyCode::Char('p')),
            KeyOutcome::Pin { .. }
        ));
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("/tmp/settings.json"),
            gist_id: "a".into(),
            gist_filename: "settings.json".into(),
            direction: None,
            last_seen_hash: None,
        }];
        assert!(matches!(
            state.handle_key(KeyCode::Char('p')),
            KeyOutcome::Unpin { .. }
        ));
    }

    #[test]
    fn p_without_local_or_gist_is_noop() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::None);
    }

    #[test]
    fn u_adds_when_gist_lacks_filename() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/config"),
            modified: None,
        }];
        state.gist_catalog.owned = vec![GistFile {
            description: "x".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "settings.json")
        }];
        state.focus = FocusPane::Gist;
        assert!(matches!(
            state.handle_key(KeyCode::Char('u')),
            KeyOutcome::UploadAdd { .. }
        ));
    }

    #[test]
    fn u_previews_when_gist_has_same_filename() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/settings.json"),
            modified: None,
        }];
        state.gist_catalog.owned = vec![GistFile {
            description: "x".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "settings.json")
        }];
        state.focus = FocusPane::Gist;
        assert!(matches!(
            state.handle_key(KeyCode::Char('u')),
            KeyOutcome::UploadPreview { .. }
        ));
    }

    #[test]
    fn u_without_selection_is_noop() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::None);
    }

    #[test]
    fn e_edits_local_with_file_selected() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/config"),
            modified: None,
        }];
        assert!(matches!(
            state.handle_key(KeyCode::Char('e')),
            KeyOutcome::EditLocal { .. }
        ));
    }

    #[test]
    fn e_without_local_is_noop() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::None);
    }

    #[test]
    fn n_opens_create_confirm() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/config.toml"),
            modified: None,
        }];
        assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
        assert!(state.screen.is_confirm());
        assert_eq!(
            state.pending_action().cloned(),
            Some(PendingAction::Create {
                local_path: PathBuf::from("/tmp/config.toml")
            })
        );
    }

    #[test]
    fn x_removes_selected_file_from_a_multifile_gist() {
        let mut state = initial_state();
        state.focus = FocusPane::Gist;
        state.gist_catalog.owned = vec![
            GistFile {
                description: "my notes".into(),
                updated_at: "2026-01-01T00:00:00Z".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                ..GistFile::fixture("abc123", "a.md")
            },
            GistFile {
                description: "my notes".into(),
                updated_at: "2026-01-01T00:00:00Z".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                ..GistFile::fixture("abc123", "b.md")
            },
        ];
        // X stages a single-file removal (not a whole-gist delete) and asks to confirm.
        assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
        assert!(state.screen.is_confirm());
        assert_eq!(
            state.pending_action().cloned(),
            Some(PendingAction::RemoveFile {
                gist_id: "abc123".into(),
                filename: "a.md".into(),
                label: "my notes".into(),
            })
        );
    }

    #[test]
    fn list_screen_capital_s_syncs_selected_pair() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("a.txt"),
            modified: None,
        }];
        state.gist_catalog.owned = vec![GistFile::fixture("g1", "a.txt")];
        let KeyOutcome::SyncSelectedPair { entry, .. } = state.handle_key(KeyCode::Char('S'))
        else {
            panic!("expected deferred pair sync");
        };
        assert!(matches!(entry.return_to, Screen::List));
    }

    #[test]
    fn list_filter_routes_chars_to_focused_pane() {
        let mut state = state_with_local_paths(&["/cwd/a.json", "/cwd/b.txt"]);
        state.focus = FocusPane::Local;
        state.filtering = true;

        state.handle_key(KeyCode::Char('j'));
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(state.local_filter_query, "js");
        assert_eq!(state.filter_query, ""); // gist pane untouched
    }

    #[test]
    fn list_filter_focus_gist_routes_to_gist_query() {
        let mut state = state_with_local_paths(&["/cwd/a.json"]);
        state.focus = FocusPane::Gist;
        state.filtering = true;

        state.handle_key(KeyCode::Char('x'));
        assert_eq!(state.filter_query, "x");
        assert_eq!(state.local_filter_query, "");
    }

    #[test]
    fn list_filter_navigates_while_typing() {
        let mut state = state_with_local_paths(&["/cwd/a.txt", "/cwd/b.txt", "/cwd/c.txt"]);
        state.focus = FocusPane::Local;
        state.filtering = true;

        state.handle_key(KeyCode::Down);
        assert_eq!(state.local_cursor.index, 1);
        assert!(state.filtering); // still in filter input
        state.handle_key(KeyCode::Up);
        assert_eq!(state.local_cursor.index, 0);
    }

    #[test]
    fn list_filter_empty_backspace_exits() {
        let mut state = state_with_local_paths(&["/cwd/a.txt"]);
        state.focus = FocusPane::Local;
        state.filtering = true;

        state.handle_key(KeyCode::Char('a'));
        state.handle_key(KeyCode::Backspace); // back to empty, still filtering
        assert!(state.filtering);
        assert_eq!(state.local_filter_query, "");
        state.handle_key(KeyCode::Backspace); // empty -> exit
        assert!(!state.filtering);
    }

    #[test]
    fn list_filter_tab_commits_and_switches_pane() {
        let mut state = state_with_local_paths(&["/cwd/a.json"]);
        state.focus = FocusPane::Local;
        state.filtering = true;
        state.handle_key(KeyCode::Char('j'));

        state.handle_key(KeyCode::Tab);
        assert!(!state.filtering); // committed, left input
        assert_eq!(state.local_filter_query, "j"); // query kept
        assert_eq!(state.focus, FocusPane::Gist); // switched pane
    }

    #[test]
    fn list_filter_esc_clears_focused_query() {
        let mut state = state_with_local_paths(&["/cwd/a.json"]);
        state.focus = FocusPane::Local;
        state.filtering = true;
        state.handle_key(KeyCode::Char('j'));

        state.handle_key(KeyCode::Esc);
        assert!(!state.filtering);
        assert_eq!(state.local_filter_query, "");
    }

    #[test]
    fn list_filter_char_resets_focused_index() {
        let mut state = state_with_local_paths(&["/cwd/a.txt", "/cwd/ab.txt", "/cwd/abc.txt"]);
        state.focus = FocusPane::Local;
        state.filtering = true;
        state.local_cursor.index = 2; // cursor not at top

        state.handle_key(KeyCode::Char('a')); // edit -> reset to top
        assert_eq!(state.local_cursor.index, 0);
    }

    #[test]
    fn list_filter_enter_keeps_query_and_exits() {
        let mut state = state_with_local_paths(&["/cwd/a.json"]);
        state.focus = FocusPane::Local;
        state.filtering = true;
        state.handle_key(KeyCode::Char('j'));

        state.handle_key(KeyCode::Enter);
        assert!(!state.filtering); // exited input
        assert_eq!(state.local_filter_query, "j"); // query kept
    }

    #[test]
    fn vim_j_k_move_list_selection() {
        let mut state = list_state_with_matches();
        state.focus = FocusPane::Gist;
        state.gist_cursor.index = 0;
        state.handle_key(KeyCode::Char('j'));
        assert_eq!(state.gist_cursor.index, 1);
        state.handle_key(KeyCode::Char('k'));
        assert_eq!(state.gist_cursor.index, 0);
    }

    #[test]
    fn vim_h_scrolls_focused_row_left() {
        let mut state = list_state_with_matches();
        state.focus = FocusPane::Gist;
        state.gist_cursor.hscroll = 2;
        state.handle_key(KeyCode::Char('h'));
        assert_eq!(state.gist_cursor.hscroll, 1);
    }

    #[test]
    fn list_page_keys_jump_local_selection() {
        let paths: Vec<String> = (0..15).map(|i| format!("/cwd/f{i:02}.txt")).collect();
        let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let mut state = state_with_local_paths(&path_refs);
        state.focus = FocusPane::Local;
        state.handle_key(KeyCode::PageDown);
        assert_eq!(state.local_cursor.index, 10);
        state.handle_key(KeyCode::PageDown);
        assert_eq!(state.local_cursor.index, 14);
        state.handle_key(KeyCode::PageUp);
        assert_eq!(state.local_cursor.index, 4);
    }

    #[test]
    fn list_filter_ctrl_f_pages_without_typing_f() {
        use crossterm::event::KeyModifiers;
        let paths: Vec<String> = (0..12).map(|i| format!("/cwd/f{i:02}.txt")).collect();
        let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let mut state = state_with_local_paths(&path_refs);
        state.focus = FocusPane::Local;
        state.filtering = true;
        state.local_filter_query.set("f");
        state.handle_key_with(KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert_eq!(state.local_cursor.index, 10);
        assert_eq!(state.local_filter_query, "f");
    }

    #[test]
    fn foreign_gist_blocks_pin() {
        let mut state = initial_state();
        state.gist_catalog.user_login = Some("me".into());
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/cwd/a.txt"),
            modified: None,
        }];
        state.gist_catalog.owned = vec![GistFile {
            description: "x".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: "other".into(),
            ..GistFile::fixture("foreign", "a.txt")
        }];
        state.local_cursor.index = 0;
        state.gist_cursor.index = 0;
        assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::None);
        assert!(state.status.as_ref().unwrap().contains("cannot pin"));
    }

    #[test]
    fn star_key_returns_toggle_intent() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![GistFile {
            description: "x".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("g1", "a.txt")
        }];
        state.gist_cursor.index = 0;
        assert!(matches!(
            state.handle_key(KeyCode::Char('*')),
            KeyOutcome::ToggleGistStar { .. }
        ));
    }

    /// Divider drag (#395) — geometry first: percent is the stored fact, cells are derived.
    #[test]
    fn split_cells_rounds_to_nearest_cell() {
        assert_eq!(split_cells(40, 100), 40);
        // 54.8 rounds up, matching what ratatui's percentage constraint resolved to before.
        assert_eq!(split_cells(40, 137), 55);
        assert_eq!(split_cells(15, 80), 12);
    }

    #[test]
    fn clamp_split_percent_keeps_both_panes_readable() {
        // Wide terminal: only the percent band bites.
        assert_eq!(clamp_split_percent(50, 200), Some(50));
        assert_eq!(clamp_split_percent(5, 200), Some(MIN_SPLIT_PERCENT));
        assert_eq!(clamp_split_percent(99, 200), Some(MAX_SPLIT_PERCENT));
        // Narrow terminal: the absolute floor takes over from the band on both sides.
        for width in [MIN_PANE_CELLS * 2, 40, 60] {
            let low = clamp_split_percent(0, width).expect("width fits two panes");
            let high = clamp_split_percent(100, width).expect("width fits two panes");
            assert!(
                split_cells(low, width) >= MIN_PANE_CELLS,
                "low {low} @ {width}"
            );
            assert!(
                width - split_cells(high, width) >= MIN_PANE_CELLS,
                "high {high} @ {width}"
            );
        }
        // Too narrow for two readable panes at any split.
        assert_eq!(clamp_split_percent(40, MIN_PANE_CELLS * 2 - 1), None);
    }

    #[test]
    fn percent_for_col_puts_the_divider_under_the_pointer() {
        let area = Rect::new(0, 1, 100, 20);
        assert_eq!(percent_for_col(area, 39), 40);
        assert_eq!(percent_for_col(area, 59), 60);
        // Offset areas measure from their own left edge.
        assert_eq!(percent_for_col(Rect::new(10, 1, 100, 20), 49), 40);
    }

    /// One percent is worth more than one cell above 100 columns, so not every column is
    /// reachable — the divider lands on the nearest one that is, never further.
    #[test]
    fn percent_for_col_lands_within_one_cell_on_widths_that_are_not_multiples_of_100() {
        for width in [80, 137, 200] {
            let area = Rect::new(0, 1, width, 20);
            for col in 20..(width - 20) {
                let cells = split_cells(percent_for_col(area, col), width);
                let landed = i32::from(cells) - 1; // the local pane's right border
                let step = i32::from(width).div_euclid(100) + 1; // cells one percent buys
                assert!(
                    (landed - i32::from(col)).abs() <= step,
                    "col {col} @ {width} landed on {landed}"
                );
            }
        }
    }

    /// A `SplitHit` for a 100-column List screen split at the default 40%.
    fn split_layout() -> MouseFrame {
        let mut layout = MouseFrame::default();
        let split = SplitHit {
            area: Rect::new(0, 1, 100, 20),
            divider_x: 39,
        };
        layout.register(HitTarget::Divider(split), split.area);
        layout
    }

    #[test]
    fn dragging_the_divider_resizes_the_panes_and_release_ends_the_drag() {
        let mut state = state_with_local_paths(&["a.rs"]);
        state.screen = Screen::List;
        let layout = split_layout();

        assert_eq!(
            state.handle_mouse(MouseInput::Click { col: 39, row: 5 }, &layout),
            KeyOutcome::None
        );
        assert!(state.mouse_session.is_dragging());

        state.handle_mouse(MouseInput::Drag { col: 59 }, &layout);
        assert_eq!(state.list_split_percent, 60);

        state.handle_mouse(MouseInput::Release, &layout);
        assert!(!state.mouse_session.is_dragging());
        // A release after the drag must not keep resizing.
        state.handle_mouse(MouseInput::Drag { col: 20 }, &layout);
        assert_eq!(state.list_split_percent, 60);
    }

    #[test]
    fn grabbing_the_divider_does_not_focus_or_select_a_pane() {
        let mut state = state_with_local_paths(&["a.rs", "b.rs"]);
        state.screen = Screen::List;
        state.focus = FocusPane::Gist;
        state.local_cursor.index = 1;
        let mut layout = split_layout();
        // The local pane reaches the divider, so without the grab check this would focus it.
        layout.register_pane(
            PaneTarget::Local,
            PaneHit {
                rect: Rect::new(0, 1, 40, 20),
                offset: 0,
            },
            2,
        );

        state.handle_mouse(MouseInput::Click { col: 39, row: 5 }, &layout);
        assert!(state.mouse_session.is_dragging());
        assert_eq!(state.focus, FocusPane::Gist);
        assert_eq!(state.local_cursor.index, 1);
    }

    #[test]
    fn the_grab_zone_is_widened_by_one_cell_on_each_side() {
        let layout = split_layout();
        for col in 38..=41 {
            let mut state = state_with_local_paths(&["a.rs"]);
            state.screen = Screen::List;
            state.handle_mouse(MouseInput::Click { col, row: 5 }, &layout);
            assert!(
                state.mouse_session.is_dragging(),
                "col {col} missed the divider"
            );
        }
        for col in [37, 42] {
            let mut state = state_with_local_paths(&["a.rs"]);
            state.screen = Screen::List;
            state.handle_mouse(MouseInput::Click { col, row: 5 }, &layout);
            assert!(
                !state.mouse_session.is_dragging(),
                "col {col} grabbed the divider"
            );
        }
    }

    #[test]
    fn a_running_drag_ignores_the_row() {
        let mut state = state_with_local_paths(&["a.rs"]);
        state.screen = Screen::List;
        let layout = split_layout();
        state.handle_mouse(MouseInput::Click { col: 39, row: 5 }, &layout);
        // Row 0 is the top bar, well outside the panes: the drag must survive it.
        state.handle_mouse(MouseInput::Drag { col: 49 }, &layout);
        assert_eq!(state.list_split_percent, 50);
    }

    #[test]
    fn dragging_clamps_to_a_readable_pane_width() {
        let mut state = state_with_local_paths(&["a.rs"]);
        state.screen = Screen::List;
        let layout = split_layout();
        state.handle_mouse(MouseInput::Click { col: 39, row: 5 }, &layout);
        state.handle_mouse(MouseInput::Drag { col: 0 }, &layout);
        assert_eq!(state.list_split_percent, MIN_SPLIT_PERCENT);
        state.handle_mouse(MouseInput::Drag { col: 99 }, &layout);
        assert_eq!(state.list_split_percent, MAX_SPLIT_PERCENT);
    }

    #[test]
    fn a_terminal_too_narrow_for_two_panes_has_no_grabbable_divider() {
        let mut state = state_with_local_paths(&["a.rs", "b.rs"]);
        state.screen = Screen::List;
        state.focus = FocusPane::Gist;
        let width = MIN_PANE_CELLS * 2 - 1;
        let mut layout = MouseFrame::default();
        let split = SplitHit {
            area: Rect::new(0, 1, width, 20),
            divider_x: width / 2,
        };
        layout.register(HitTarget::Divider(split), split.area);
        layout.register_pane(
            PaneTarget::Local,
            PaneHit {
                rect: Rect::new(0, 1, width / 2 + 1, 20),
                offset: 0,
            },
            2,
        );
        state.handle_mouse(
            MouseInput::Click {
                col: width / 2,
                row: 5,
            },
            &layout,
        );
        assert!(!state.mouse_session.is_dragging());
        // The press is not swallowed: it focuses the pane it landed in, as any other click would.
        assert_eq!(state.focus, FocusPane::Local);

        // Nor is the double-click reset, which would otherwise eat the "open this row" gesture
        // to write a percent that paint discards anyway.
        state.list_split_percent = 70;
        assert!(!state.reset_split_divider(width / 2, 5, &layout));
        assert_eq!(state.list_split_percent, 70);
    }

    #[test]
    fn drag_without_grabbing_the_divider_changes_nothing() {
        let mut state = state_with_local_paths(&["a.rs"]);
        state.screen = Screen::List;
        let layout = split_layout();
        state.handle_mouse(MouseInput::Drag { col: 70 }, &layout);
        assert_eq!(state.list_split_percent, DEFAULT_SPLIT_PERCENT);
        assert!(!state.mouse_session.is_dragging());
    }

    #[test]
    fn double_clicking_the_divider_restores_the_default_split() {
        let mut state = state_with_local_paths(&["a.rs"]);
        state.screen = Screen::List;
        let layout = split_layout();
        state.list_split_percent = 70;
        assert_eq!(
            state.handle_mouse(MouseInput::DoubleClick { col: 39, row: 5 }, &layout),
            KeyOutcome::None
        );
        assert_eq!(state.list_split_percent, DEFAULT_SPLIT_PERCENT);
        assert!(!state.mouse_session.is_dragging());
    }

    #[test]
    fn list_vm_carries_the_split_and_its_drag_state() {
        let mut state = state_with_local_paths(&["a.rs"]);
        state.screen = Screen::List;
        state.list_split_percent = 55;
        let vm = build_list_vm(&state);
        assert_eq!(vm.split_percent, 55);
        assert!(!vm.split_dragging);
        state.mouse_session.begin_divider_drag();
        assert!(build_list_vm(&state).split_dragging);
    }

    /// Pins the percent → cells conversion to what actually reaches the screen: at 40% of a
    /// 100-column terminal the local pane owns columns 0..=39, so its right border lands on
    /// column 39 and the gist pane's left border on column 40.
    #[test]
    fn render_puts_the_divider_where_the_percent_says() {
        let mut state = state_with_local_paths(&["a.rs"]);
        state.screen = Screen::List;
        let backend = ratatui::backend::TestBackend::new(100, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let vm = crate::tui::build_view_model(&state);
        let mut layout = MouseFrame::default();
        let ScreenVm::List(list) = &vm.screen else {
            panic!("not the List screen");
        };
        terminal
            .draw(|frame| render_list_vm(frame, &state, list, &vm.chrome, &mut layout))
            .unwrap();

        let buffer = terminal.backend().buffer();
        // Row 2 is inside both panes (row 0 is the top bar, row 1 the pane titles).
        assert_eq!(buffer.cell((39, 2)).unwrap().symbol(), "│");
        assert_eq!(buffer.cell((40, 2)).unwrap().symbol(), "│");
        assert_eq!(buffer.cell((20, 2)).unwrap().symbol(), " ");

        let hit = layout.split().expect("divider hit recorded");
        assert_eq!(hit.divider_x, 39);
        assert_eq!(hit.area.width, 100);
    }

    #[test]
    fn list_vm_empty_local_and_gist_messages() {
        let state = initial_state();
        let list = build_list_vm(&state);
        assert_eq!(list.local.empty, ListPaneEmpty::NoItems);
        assert!(list
            .local
            .empty_message
            .as_deref()
            .unwrap_or("")
            .contains("No local files"));
        assert_eq!(list.gist.empty, ListPaneEmpty::NoItems);
        assert!(list
            .gist
            .empty_message
            .as_deref()
            .unwrap_or("")
            .contains("No gists found"));
        assert!(matches!(list.footer, ListFooterVm::Hints { .. }));
    }

    #[test]
    fn list_vm_rows_include_pin_mark_and_star() {
        use crate::domain::{GistFile, LocalCandidate, PinnedMapping};

        let mut state = initial_state();
        state.cwd = PathBuf::from("/tmp/proj");
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("notes.txt"),
            modified: None,
        }];
        state.gist_catalog.owned = vec![GistFile {
            description: "demo notes".into(),
            ..GistFile::fixture("g1", "notes.txt")
        }];
        state.gist_catalog.starred_ids.insert("g1".into());
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("notes.txt"),
            gist_id: "g1".into(),
            gist_filename: "notes.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
        state.focus = FocusPane::Local;
        state.anchor = FocusPane::Local;
        state.local_cursor.index = 0;
        state.gist_cursor.index = 0;

        let list = build_list_vm(&state);
        assert_eq!(list.local.empty, ListPaneEmpty::HasRows);
        assert_eq!(list.gist.empty, ListPaneEmpty::HasRows);
        assert!(!list.local.rows.is_empty());
        assert!(!list.gist.rows.is_empty());
        assert!(
            list.gist.rows[0].label.contains('★'),
            "starred gist row: {}",
            list.gist.rows[0].label
        );
        assert!(
            list.local
                .rows
                .iter()
                .any(|r| r.label.contains("notes.txt")),
            "local rows: {:?}",
            list.local.rows
        );
        // Pin or exact-filename mark when both sides share the pair.
        let marked = list
            .local
            .rows
            .iter()
            .chain(list.gist.rows.iter())
            .any(|r| matches!(r.emphasis, RowEmphasis::Strong) || r.label.contains('↔'));
        assert!(
            marked,
            "local={:?} gist={:?}",
            list.local.rows, list.gist.rows
        );
    }

    #[test]
    fn list_vm_status_footer_and_filter_mode() {
        let mut state = initial_state();
        state.status = Some("Downloaded a.txt".into());
        let list = build_list_vm(&state);
        match list.footer {
            ListFooterVm::Status { text } => assert!(text.contains("Downloaded")),
            other => panic!("expected Status footer, got {other:?}"),
        }

        state.status = None;
        state.filtering = true;
        state.focus = FocusPane::Gist;
        let list = build_list_vm(&state);
        match list.footer {
            ListFooterVm::Filtering { focus, .. } => assert_eq!(focus, FocusPane::Gist),
            other => panic!("expected Filtering footer, got {other:?}"),
        }
    }
}
