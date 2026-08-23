//! `Screen::Revisions` — key handling, view-model, paint, palette items, and apply handlers
//! colocated in one file (issue #287, Phase 2; issue #383).

use crate::actions::SystemRunner;
use crate::tui::bg::{revision_version_label, Jobs, LoopFlow};
use crate::tui::keys::{apply_list_cursor_nav, NavAction};
use crate::tui::render::list_pane::render_list_pane;
use crate::tui::text::hscroll_max_for_text;
use crate::tui::view_model::{
    ChromeVm, ListPaneEmpty, ListPaneVm, PaneTitleVm, RevisionsVm, RowEmphasis, RowVm,
};
use crate::tui::{
    AppState, HelpTopic, HitTarget, KeyOutcome, ListCursor, MouseFrame, PaneTarget, RowTarget,
    Screen,
};
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
    let not_head = rev.is_some_and(|r| r.cursor.index > 0);
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
                if entries_len > 0 && self.revision().is_some_and(|r| r.cursor.index > 0) =>
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
            KeyCode::Char('r') if self.revision().is_some_and(|r| r.cursor.index == 0) => {
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
        entries.get(rev.cursor.index)
    }

    fn revision_diff_incremental_intent(&mut self) -> KeyOutcome {
        let Some(rev) = self.revision() else {
            return KeyOutcome::None;
        };
        let Some(gist_id) = rev.gist_id.clone() else {
            return KeyOutcome::None;
        };
        let filename = rev.target_file.clone();
        let index = rev.cursor.index;
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
            entry: self.defer_entry(),
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
            entry: self.defer_entry(),
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
            entry: self.defer_entry(),
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
    /// scrolls it horizontally. Precomputes len/hmax with `&self`, then mutates the cursor
    /// (issue #274 pattern, applied here in #408: cannot hold `&mut RevisionState` from
    /// `match &mut self.screen` while calling `&self` helpers).
    pub(crate) fn apply_navigation_revisions(&mut self, action: NavAction) -> bool {
        let len = self
            .revision()
            .and_then(|r| r.entries.as_ref().map(|e| e.len()))
            .unwrap_or(0);
        if len == 0 {
            return false;
        }
        let hmax = self.revisions_hscroll_max();
        let Screen::Revisions(rev) = &mut self.screen else {
            return false;
        };
        apply_list_cursor_nav(&mut rev.cursor, action, len, hmax);
        true
    }

    /// Select the clicked row on `Screen::Revisions`, moving the entry cursor. Returns `true`
    /// when a row was hit.
    pub(crate) fn click_select_revisions(&mut self, target: RowTarget) -> bool {
        let Some(idx) = target.list_index() else {
            return false;
        };
        let Screen::Revisions(rev) = &mut self.screen else {
            return false;
        };
        rev.cursor.select(idx);
        true
    }

    /// Highest horizontal-scroll offset for the Revisions screen, bounded by the selected
    /// row's complete rendered label (mirrors `gists_hscroll_max` / `pins_hscroll_max`;
    /// issue #408 — Revisions previously had no bound and could overflow `u16`).
    fn revisions_hscroll_max(&self) -> u16 {
        let now = crate::tui::render::unix_now();
        self.revision()
            .and_then(|r| {
                let entries = r.entries.as_ref()?;
                let idx = r.cursor.index;
                entries
                    .get(idx)
                    .map(|entry| revision_row_label(entry, idx, now))
            })
            .map(|label| hscroll_max_for_text(&label))
            .unwrap_or(0)
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
            (ListPaneEmpty::HasRows, None, rows, Some(rev.cursor.index))
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
            hscroll: rev.cursor.hscroll,
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
        &state.settings.theme(),
        chrome.mouse_enabled,
        layout,
        PaneTarget::List,
    );
    crate::tui::render_footer(
        frame,
        chunks[1],
        "",
        &revs.footer,
        revs.footer_colored,
        crate::tui::keymap::for_screen(&state.screen),
        &state.settings.theme(),
    );
    if chrome.mouse_enabled {
        let close = crate::tui::render_close_button(frame, area, &state.settings.theme());
        layout.register(HitTarget::Close, close);
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
                let before = rev.cursor.index;
                rev.cursor.clamp_len(entries.len());
                if rev.cursor.index != before {
                    rev.cursor.hscroll = 0;
                }
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

/// Re-fetch revision history for `gist_id`. Shared by `KeyOutcome::FetchRevisions` and
/// `Jobs::absorb` when apply set `revisions_stale` (issue #383).
pub(crate) fn request_revisions(jobs: &mut Jobs, state: &mut AppState, gist_id: String) {
    jobs.spawn_action(
        state,
        "Loading revisions…",
        move || {
            let result = crate::gh::fetch_gist_commits_json(&SystemRunner, &gist_id)
                .map_err(|e| e.to_string())
                .and_then(|raw| {
                    crate::gh::parse_gist_commits_json(&raw).map_err(|e| e.to_string())
                });
            (result, gist_id)
        },
        move |(result, gist_id), state| on_revisions_fetched(state, gist_id, result),
    );
}

/// `RestoreRevisionDone` outcome: return to the Revisions screen and re-fetch both the
/// gist list and the revision history for the restored gist.
pub(crate) fn on_restore_revision_done(
    state: &mut AppState,
    result: std::result::Result<(), String>,
    gist_id: String,
    filename: String,
) -> LoopFlow {
    match result {
        Ok(()) => {
            state
                .gist_content_store
                .invalidate_file(&crate::domain::GistFileRef::id_name(
                    gist_id.clone(),
                    filename.clone(),
                ));
            state.set_status(format!(
                "Restored {filename} from old revision (new revision created)"
            ));
            // Return to the revisions list `enter_confirm` parked when the
            // restore confirm was entered.
            state.leave();
            if !state.screen.is_revisions() {
                state.screen = Screen::Revisions(Box::default());
            }
            let gist_id = state.revision_mut().and_then(|rev| {
                rev.cursor = ListCursor::default();
                rev.entries = None;
                rev.fetch_error = None;
                rev.gist_id.clone()
            });
            state.gist_list_stale = true;
            state.revisions_stale = gist_id;
        }
        Err(error) => {
            state.set_status(format!("restore failed: {error}"));
            // Stay on Confirm payload if still open.
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
        revision_mut(&mut state).cursor.index = 0;
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
        revision_mut(&mut state).cursor.index = 1;
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
        revision_mut(&mut state).cursor.index = 0;
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
        revision_mut(&mut state).cursor.index = 1;
        assert!(matches!(
            state.handle_key(KeyCode::Enter),
            KeyOutcome::RevisionDiffIncremental { .. }
        ));
    }

    #[test]
    fn revision_diff_omits_download_upload() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::default());
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
        state.gist_catalog.owned = vec![GistFile {
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
        revision_mut(&mut state).cursor.index = 0;
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
        let mut layout = MouseFrame::default();
        layout.register_pane(PaneTarget::List, hit, 2);
        let out = state.handle_mouse(MouseInput::Click { col: 5, row: 2 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(revision_ref(&state).cursor.index, 1);
        let mut by_key = state.clone();
        let key_out = by_key.handle_key(KeyCode::Enter);
        let by_mouse = state.handle_mouse(MouseInput::DoubleClick { col: 5, row: 2 }, &layout);
        assert_eq!(by_mouse, key_out);
        assert!(matches!(
            by_mouse,
            KeyOutcome::RevisionDiffIncremental { .. }
        ));
    }

    /// A click in the pane's blank area does nothing (issue #408).
    #[test]
    fn revisions_blank_pane_click_is_a_noop() {
        let mut state = state_with_gists();
        state.screen = Screen::Revisions(Box::default());
        revision_mut(&mut state).entries = Some(vec![crate::domain::GistRevision {
            version: "v1".into(),
            committed_at: "2026-06-10T00:00:00Z".into(),
            user: "u".into(),
            change_status: crate::domain::GistRevisionChangeStatus {
                total: 1,
                additions: 1,
                deletions: 0,
            },
        }]);
        let hit = PaneHit {
            rect: Rect::new(0, 0, 40, 10),
            offset: 0,
        };
        let mut layout = MouseFrame::default();
        layout.register_pane(PaneTarget::List, hit, 1);
        let out = state.handle_mouse(MouseInput::Click { col: 5, row: 8 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(revision_ref(&state).cursor.index, 0);
    }

    /// Vertical/page navigation and row clicks reset horizontal scroll; Right stops at the
    /// selected row's complete rendered label and cannot overflow (issue #408 — previously
    /// unbounded, so a selected row could scroll entirely blank and `hscroll` could wrap).
    #[test]
    fn revisions_navigation_resets_hscroll_and_right_is_clamped() {
        let mut state = state_with_gists();
        state.screen = Screen::Revisions(Box::default());
        let entries = vec![
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
        ];
        revision_mut(&mut state).entries = Some(entries.clone());
        let label = revision_row_label(&entries[0], 0, crate::tui::render::unix_now());
        let hmax = hscroll_max_for_text(&label);

        // Right stops at the row's own max — many more presses than the label is long.
        for _ in 0..hmax + 50 {
            state.handle_key(KeyCode::Right);
        }
        assert_eq!(revision_ref(&state).cursor.hscroll, hmax);

        // Down resets it.
        state.handle_key(KeyCode::Down);
        assert_eq!(revision_ref(&state).cursor.index, 1);
        assert_eq!(revision_ref(&state).cursor.hscroll, 0);

        state.handle_key(KeyCode::Right);
        assert!(revision_ref(&state).cursor.hscroll > 0);
        state.handle_key(KeyCode::Up);
        assert_eq!(revision_ref(&state).cursor.hscroll, 0);

        state.handle_key(KeyCode::Right);
        state.handle_key(KeyCode::PageDown);
        assert_eq!(revision_ref(&state).cursor.hscroll, 0);

        state.handle_key(KeyCode::Right);
        state.handle_key(KeyCode::PageUp);
        assert_eq!(revision_ref(&state).cursor.hscroll, 0);

        // A row click resets it too.
        revision_mut(&mut state).cursor.hscroll = 5;
        let hit = PaneHit {
            rect: Rect::new(0, 0, 40, 10),
            offset: 0,
        };
        let mut layout = MouseFrame::default();
        layout.register_pane(PaneTarget::List, hit, 2);
        state.handle_mouse(MouseInput::Click { col: 5, row: 1 }, &layout);
        assert_eq!(revision_ref(&state).cursor.hscroll, 0);
    }

    /// A shrinking revision list clamps an out-of-range cursor and clears its stale
    /// horizontal scroll; a cursor that stays in range keeps its horizontal scroll
    /// (issue #408).
    #[test]
    fn on_revisions_fetched_clamps_an_out_of_range_cursor() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some("g1".into()),
            cursor: ListCursor {
                index: 3,
                hscroll: 7,
            },
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

        on_revisions_fetched(&mut state, "g1".into(), Ok(vec![entry.clone(), entry]));

        assert_eq!(revision_ref(&state).cursor.index, 1, "clamped into range");
        assert_eq!(
            revision_ref(&state).cursor.hscroll,
            0,
            "stale hscroll cleared"
        );
    }

    /// An in-range cursor is left untouched by a fetch (issue #408).
    #[test]
    fn on_revisions_fetched_preserves_an_in_range_cursor() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some("g1".into()),
            cursor: ListCursor {
                index: 1,
                hscroll: 4,
            },
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

        on_revisions_fetched(
            &mut state,
            "g1".into(),
            Ok(vec![entry.clone(), entry.clone(), entry]),
        );

        assert_eq!(revision_ref(&state).cursor.index, 1);
        assert_eq!(revision_ref(&state).cursor.hscroll, 4);
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

    #[test]
    fn on_restore_revision_done_err_sets_status() {
        let mut state = initial_state();

        on_restore_revision_done(&mut state, Err("boom".into()), "g1".into(), "a.txt".into());

        assert_eq!(state.status.as_deref(), Some("restore failed: boom"));
        assert!(!state.gist_list_stale);
        assert!(state.revisions_stale.is_none());
    }

    #[test]
    fn on_restore_revision_done_ok_returns_to_revisions_and_marks_stale() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some("g1".into()),
            cursor: ListCursor {
                index: 3,
                ..Default::default()
            },
            entries: Some(Vec::new()),
            fetch_error: Some("old".into()),
            ..Default::default()
        }));
        state.enter_confirm(
            PendingAction::RestoreRevision {
                gist_id: "g1".into(),
                filename: "a.txt".into(),
                version: "abc".into(),
                version_label: "abc (1d ago)".into(),
                content: "old".into(),
            },
            String::new(),
        );
        let file = crate::domain::GistFileRef::id_name("g1", "a.txt");
        state.gist_content_store.insert(&file, "stale".into());

        on_restore_revision_done(&mut state, Ok(()), "g1".into(), "a.txt".into());

        assert!(state.gist_list_stale);
        assert_eq!(state.revisions_stale.as_deref(), Some("g1"));
        assert!(state.screen.is_revisions());
        let rev = state.revision().unwrap();
        assert_eq!(rev.cursor.index, 0);
        assert!(rev.entries.is_none());
        assert!(rev.fetch_error.is_none());
        assert_eq!(
            state.status.as_deref(),
            Some("Restored a.txt from old revision (new revision created)")
        );
        assert!(matches!(
            state.gist_content_store.lookup(
                &state.gist_catalog,
                file,
                crate::tui::gist_content::FetchPolicy::PreferCache
            ),
            crate::tui::gist_content::ContentLookup::Miss(_)
        ));
    }
}
