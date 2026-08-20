//! `Screen::Revisions` — key handling, view-model, paint, palette items, and apply handlers
//! colocated in one file (issue #287, Phase 2; issue #383).

use crate::tui::bg::{revision_version_label, LoopFlow};
use crate::tui::keys::{point_in, NavAction, PAGE_SCROLL};
use crate::tui::render::list_pane::render_list_pane;
use crate::tui::view_model::{
    ChromeVm, ListPaneEmpty, ListPaneVm, PaneTitleVm, RevisionsVm, RowEmphasis, RowVm,
};
use crate::tui::{AppState, HelpTopic, KeyOutcome, MouseLayout, Screen};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::Revisions;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    1
}

/// Shared "would this key actually do something" predicate for `Screen::Revisions`, mirrored
/// by both [`AppState::handle_key_revisions`]'s match-arm guards and `revisions_palette_items`
/// so the two can never silently drift (issue #288).
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

impl AppState {
    pub(crate) fn handle_key_revisions(&mut self, code: KeyCode) -> KeyOutcome {
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

    /// Advance `revision_target_file` to the next filename in this gist (wraps). Returns
    /// false when the gist has at most one file.
    fn cycle_revision_target_file(&mut self) -> bool {
        let Some(gist_id) = self.revision().and_then(|r| r.gist_id.clone()) else {
            return false;
        };
        let files = self.gist_filenames(&gist_id);
        if files.len() <= 1 {
            return false;
        }
        let Some(rev) = self.revision_mut() else {
            return false;
        };
        let current = files
            .iter()
            .position(|f| f == &rev.target_file)
            .unwrap_or(0);
        rev.target_file = files[(current + 1) % files.len()].clone();
        true
    }

    fn selected_revision(&self) -> Option<&crate::domain::GistRevision> {
        let rev = self.revision()?;
        let entries = rev.entries.as_ref()?;
        entries.get(rev.index)
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

    /// Footer label for the revision-history target file, including `(n/total)` when multi-file.
    pub(crate) fn revision_target_file_label(&self) -> String {
        let Some(rev) = self.revision() else {
            return String::new();
        };
        let Some(gist_id) = rev.gist_id.as_deref() else {
            return rev.target_file.clone();
        };
        let files = self.gist_filenames(gist_id);
        if files.len() <= 1 {
            return rev.target_file.clone();
        }
        let index = files
            .iter()
            .position(|f| f == &rev.target_file)
            .map(|i| i + 1)
            .unwrap_or(1);
        format!("{} ({index}/{})", rev.target_file, files.len())
    }

    /// Arrow / hjkl / page-key navigation for `Screen::Revisions`: moves the entry cursor, or
    /// scrolls it horizontally.
    pub(crate) fn apply_navigation_revisions(&mut self, action: NavAction) -> bool {
        let Screen::Revisions(rev) = &mut self.screen else {
            return false;
        };
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

    /// Select the clicked row on `Screen::Revisions`, moving the entry cursor. Returns `true`
    /// when a row was hit.
    pub(crate) fn click_select_revisions(
        &mut self,
        col: u16,
        row: u16,
        layout: &MouseLayout,
    ) -> bool {
        let Screen::Revisions(rev) = &mut self.screen else {
            return false;
        };
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
}

/// Builds a single Revisions-screen row. Pure so the age/delta formatting is unit-testable
/// without a frame.
fn revision_row_label(rev: &crate::domain::GistRevision, index: usize, now: u64) -> String {
    let age = crate::domain::parse_rfc3339_to_unix(&rev.committed_at)
        .map(|t| crate::domain::humanize_age(now as i64 - t as i64))
        .unwrap_or_else(|| "?".into());
    let delta = format!(
        "+{}/-{}",
        rev.change_status.additions, rev.change_status.deletions
    );
    let sha = crate::domain::short_sha(&rev.version);
    let current = if index == 0 { " (current)" } else { "" };
    format!(
        "#{}  {} ago  {}  {}  {}{}",
        index + 1,
        age,
        delta,
        rev.user,
        sha,
        current
    )
}

pub(crate) fn build_revisions_vm(state: &AppState) -> RevisionsVm {
    let rev = state.revision().cloned().unwrap_or_default();
    let (footer, footer_colored) = if let Some(message) = &state.status {
        (message.clone(), false)
    } else if rev.entries.is_none() {
        ("Loading revisions…".to_string(), false)
    } else if let Some(err) = &rev.fetch_error {
        (err.clone(), false)
    } else {
        let file = state.revision_target_file_label();
        (format!("file={file}"), false)
    };

    let gist_id = rev.gist_id.as_deref().unwrap_or("");
    let label = state
        .group_by_id(gist_id)
        .map(|g| {
            if g.description.trim().is_empty() {
                g.id.clone()
            } else {
                g.description.clone()
            }
        })
        .unwrap_or_else(|| gist_id.to_string());

    let now = crate::tui::render::unix_now();
    let (empty, empty_message, rows, selected) = match &rev.entries {
        None => (
            ListPaneEmpty::Loading,
            Some("  ⏳ Loading revisions…".into()),
            Vec::new(),
            None,
        ),
        Some(entries) if entries.is_empty() => (
            ListPaneEmpty::NoItems,
            Some("  📭 No revisions found".into()),
            Vec::new(),
            None,
        ),
        Some(entries) => {
            let rows = entries
                .iter()
                .enumerate()
                .map(|(i, r)| RowVm {
                    label: revision_row_label(r, i, now),
                    emphasis: RowEmphasis::None,
                })
                .collect();
            (ListPaneEmpty::HasRows, None, rows, Some(rev.index))
        }
    };

    let count = rows.len();
    RevisionsVm {
        pane: ListPaneVm {
            title: PaneTitleVm::new(format!(
                "Revisions: {label} {}",
                crate::tui::render::count_label(count, count)
            )),
            focused: true,
            selected,
            empty,
            empty_message,
            rows,
            hscroll: rev.hscroll,
            scrollbar: false,
        },
        footer,
        footer_colored,
    }
}

pub(crate) fn render_revisions_vm(
    frame: &mut Frame,
    state: &AppState,
    revs: &RevisionsVm,
    chrome: &ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    let footer_lines =
        crate::tui::render::wrap_line_count(&revs.footer, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(footer_lines)])
        .split(area);

    render_list_pane(
        frame,
        chunks[0],
        &revs.pane,
        &state.theme,
        chrome.mouse_enabled,
        &mut layout.list,
    );
    crate::tui::render_footer(
        frame,
        chunks[1],
        "",
        &revs.footer,
        revs.footer_colored,
        crate::tui::keymap::for_screen(&state.screen),
        &state.theme,
    );
    if chrome.mouse_enabled {
        layout.close_button = Some(crate::tui::render_close_button(frame, area, &state.theme));
    }
}

/// `RevisionsFetched` outcome. Returns [`LoopFlow::SkipIteration`] if the fetch belongs to
/// a gist the Revisions screen has since navigated away from.
pub(crate) fn on_revisions_fetched(
    state: &mut AppState,
    gist_id: String,
    result: std::result::Result<Vec<crate::domain::GistRevision>, String>,
) -> LoopFlow {
    if state.revision().and_then(|r| r.gist_id.as_deref()) != Some(gist_id.as_str()) {
        return LoopFlow::SkipIteration;
    }
    match result {
        Ok(entries) => {
            let short = entries.len() <= 1;
            if let Some(rev) = state.revision_mut() {
                rev.fetch_error = None;
                rev.entries = Some(entries);
            }
            if short {
                state.set_status("only one revision — nothing to restore");
            }
        }
        Err(error) => {
            if let Some(rev) = state.revision_mut() {
                rev.entries = Some(Vec::new());
                rev.fetch_error = Some(error);
            }
        }
    }
    LoopFlow::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::screens::diff::diff_footer;
    use crate::tui::test_support::{
        detail_mut, list_state_with_matches, revision_ref, state_with_gists,
    };
    use crate::tui::*;
    use crossterm::event::KeyCode;
    use std::path::PathBuf;

    fn revision_mut(state: &mut AppState) -> &mut RevisionState {
        if !state.screen.is_revisions() {
            state.screen = Screen::Revisions(Box::default());
        }
        state.revision_mut().expect("expected Screen::Revisions")
    }

    #[test]
    fn revisions_r_on_head_is_blocked() {
        let mut state = state_with_gists();
        state.screen = Screen::Revisions(Box::default());
        revision_mut(&mut state).entries = Some(vec![crate::domain::GistRevision {
            version: "abc".into(),
            committed_at: "2026-06-10T00:00:00Z".into(),
            user: "u".into(),
            change_status: crate::domain::GistRevisionChangeStatus {
                total: 1,
                additions: 1,
                deletions: 0,
            },
        }]);
        state.handle_key(KeyCode::Char('r'));
        assert_eq!(
            state.status.as_deref(),
            Some("only one revision — nothing to restore")
        );
    }

    #[test]
    fn revisions_capital_d_on_current_shows_status() {
        let mut state = state_with_gists();
        state.screen = Screen::Revisions(Box::default());
        revision_mut(&mut state).gist_id = Some("g1".into());
        revision_mut(&mut state).target_file = "a.txt".into();
        revision_mut(&mut state).index = 0;
        revision_mut(&mut state).entries = Some(vec![
            crate::domain::GistRevision {
                version: "v2".into(),
                committed_at: "2026-06-10T00:00:00Z".into(),
                user: "u".into(),
                change_status: crate::domain::GistRevisionChangeStatus {
                    total: 1,
                    additions: 1,
                    deletions: 0,
                },
            },
            crate::domain::GistRevision {
                version: "v1".into(),
                committed_at: "2026-06-01T00:00:00Z".into(),
                user: "u".into(),
                change_status: crate::domain::GistRevisionChangeStatus {
                    total: 2,
                    additions: 2,
                    deletions: 0,
                },
            },
        ]);
        assert_eq!(state.handle_key(KeyCode::Char('D')), KeyOutcome::None);
        assert_eq!(state.status.as_deref(), Some("already at current revision"));
        revision_mut(&mut state).index = 1;
        assert!(matches!(
            state.handle_key(KeyCode::Char('D')),
            KeyOutcome::RevisionDiff { .. }
        ));
    }

    #[test]
    fn revisions_enter_triggers_incremental_diff() {
        let mut state = state_with_gists();
        state.screen = Screen::Revisions(Box::default());
        revision_mut(&mut state).gist_id = Some("g1".into());
        revision_mut(&mut state).target_file = "a.txt".into();
        revision_mut(&mut state).index = 0;
        revision_mut(&mut state).entries = Some(vec![
            crate::domain::GistRevision {
                version: "v2".into(),
                committed_at: "2026-06-10T00:00:00Z".into(),
                user: "u".into(),
                change_status: crate::domain::GistRevisionChangeStatus {
                    total: 1,
                    additions: 1,
                    deletions: 0,
                },
            },
            crate::domain::GistRevision {
                version: "v1".into(),
                committed_at: "2026-06-01T00:00:00Z".into(),
                user: "u".into(),
                change_status: crate::domain::GistRevisionChangeStatus {
                    total: 2,
                    additions: 2,
                    deletions: 0,
                },
            },
        ]);
        assert!(matches!(
            state.handle_key(KeyCode::Enter),
            KeyOutcome::RevisionDiffIncremental { .. }
        ));
        revision_mut(&mut state).index = 1;
        assert!(matches!(
            state.handle_key(KeyCode::Enter),
            KeyOutcome::RevisionDiffIncremental { .. }
        ));
    }

    #[test]
    fn revision_diff_omits_download_upload() {
        let mut state = initial_state();
        state.pending_return = Some(Screen::Revisions(Box::default()));
        state.enter_diff("diff".into(), String::new(), PathBuf::new(), PathBuf::new());
        let footer = diff_footer(&state);
        assert!(!footer.contains("download"));
        assert!(!footer.contains("upload"));
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
        assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::None);
    }

    #[test]
    fn revisions_capital_f_cycles_target_file() {
        let mut state = state_with_gists();
        state.screen = Screen::Revisions(Box::default());
        revision_mut(&mut state).gist_id = Some("g1".into());
        revision_mut(&mut state).target_file = "a.txt".into();
        revision_mut(&mut state).entries = Some(vec![]);
        state.handle_key(KeyCode::Char('F'));
        assert_eq!(revision_ref(&state).target_file, "b.txt");
        state.handle_key(KeyCode::Char('F'));
        assert_eq!(revision_ref(&state).target_file, "a.txt");
        assert_eq!(state.revision_target_file_label(), "a.txt (1/2)");
    }

    #[test]
    fn revisions_capital_f_on_single_file_gist_shows_status() {
        let mut state = initial_state();
        state.gists = vec![GistFile {
            description: "solo".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("g1", "only.txt")
        }];
        state.screen = Screen::Revisions(Box::default());
        revision_mut(&mut state).gist_id = Some("g1".into());
        revision_mut(&mut state).target_file = "only.txt".into();
        revision_mut(&mut state).entries = Some(vec![]);
        state.handle_key(KeyCode::Char('F'));
        assert_eq!(state.status.as_deref(), Some("only one file in this gist"));
    }

    #[test]
    fn revisions_click_selects_and_double_click_matches_enter() {
        let mut state = state_with_gists();
        state.screen = Screen::Revisions(Box::default());
        revision_mut(&mut state).gist_id = Some("g1".into());
        revision_mut(&mut state).target_file = "a.txt".into();
        revision_mut(&mut state).index = 0;
        revision_mut(&mut state).entries = Some(vec![
            crate::domain::GistRevision {
                version: "v2".into(),
                committed_at: "2026-06-10T00:00:00Z".into(),
                user: "u".into(),
                change_status: crate::domain::GistRevisionChangeStatus {
                    total: 1,
                    additions: 1,
                    deletions: 0,
                },
            },
            crate::domain::GistRevision {
                version: "v1".into(),
                committed_at: "2026-06-01T00:00:00Z".into(),
                user: "u".into(),
                change_status: crate::domain::GistRevisionChangeStatus {
                    total: 2,
                    additions: 2,
                    deletions: 0,
                },
            },
        ]);
        let hit = PaneHit {
            rect: Rect::new(0, 0, 40, 10),
            offset: 0,
        };
        let layout = MouseLayout {
            list: Some(hit),
            ..Default::default()
        };
        let out = state.handle_mouse(MouseInput::Click { col: 5, row: 2 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(revision_ref(&state).index, 1);
        let mut by_key = state.clone();
        let key_out = by_key.handle_key(KeyCode::Enter);
        let by_mouse = state.handle_mouse(MouseInput::DoubleClick { col: 5, row: 2 }, &layout);
        assert_eq!(by_mouse, key_out);
        assert!(matches!(
            by_mouse,
            KeyOutcome::RevisionDiffIncremental { .. }
        ));
    }

    #[test]
    fn capital_h_from_list_opens_revisions_for_selected_gist_file() {
        let mut state = list_state_with_matches();
        state.focus = FocusPane::Gist;
        state.gist_index = 0;
        let outcome = state.handle_key(KeyCode::Char('H'));
        assert!(matches!(outcome, KeyOutcome::FetchRevisions { .. }));
        assert!(state.screen.is_revisions());
        assert_eq!(revision_ref(&state).gist_id.as_deref(), Some("a"));
        assert_eq!(revision_ref(&state).target_file, "settings.json");
        assert_eq!(state.nav_stack.last(), Some(&Screen::List));
    }

    #[test]
    fn capital_h_from_gist_detail_opens_revisions_and_fetches() {
        let mut state = state_with_gists();
        state.screen = Screen::GistDetail(Box::default());
        detail_mut(&mut state).gist_id = Some("g1".into());
        detail_mut(&mut state).file_cursor = 1;
        let outcome = state.handle_key(KeyCode::Char('H'));
        assert!(matches!(outcome, KeyOutcome::FetchRevisions { .. }));
        assert!(state.screen.is_revisions());
        assert_eq!(revision_ref(&state).gist_id.as_deref(), Some("g1"));
        assert_eq!(revision_ref(&state).target_file, "b.txt");
        assert!(state.nav_stack.last().is_some_and(Screen::is_gist_detail));
        assert!(revision_ref(&state).entries.is_none());
    }

    #[test]
    fn on_revisions_fetched_returns_skip_iteration_on_gist_mismatch() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some("g1".into()),
            ..Default::default()
        }));

        let flow = on_revisions_fetched(&mut state, "other-gist".into(), Ok(Vec::new()));

        assert!(matches!(flow, LoopFlow::SkipIteration));
        // Not applied — still no entries.
        assert!(state.revision().unwrap().entries.is_none());
    }

    #[test]
    fn on_revisions_fetched_ok_sets_entries() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some("g1".into()),
            ..Default::default()
        }));
        let entry = GistRevision {
            version: "abc123".into(),
            committed_at: "2026-01-01T00:00:00Z".into(),
            user: "alice".into(),
            change_status: crate::domain::GistRevisionChangeStatus {
                total: 1,
                additions: 1,
                deletions: 0,
            },
        };

        let flow = on_revisions_fetched(&mut state, "g1".into(), Ok(vec![entry.clone(), entry]));

        assert!(matches!(flow, LoopFlow::Proceed));
        assert_eq!(state.revision().unwrap().entries.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn on_revisions_fetched_err_sets_fetch_error() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some("g1".into()),
            ..Default::default()
        }));

        on_revisions_fetched(&mut state, "g1".into(), Err("boom".into()));

        let rev = state.revision().unwrap();
        assert_eq!(rev.fetch_error.as_deref(), Some("boom"));
        assert_eq!(rev.entries, Some(Vec::new()));
    }
}
