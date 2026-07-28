//! `Screen::Revisions` — key handling, view-model, paint, and palette items colocated in
//! one file (issue #287, Phase 2).

use crate::tui::bg::revision_version_label;
use crate::tui::view_model::{ChromeVm, RevisionsEmptyKind, RevisionsVm};
use crate::tui::{AppState, HelpTopic, KeyOutcome, MouseLayout, PaneHit};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Padding},
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
            RevisionsEmptyKind::Loading,
            Some("  ⏳ Loading revisions…".into()),
            Vec::new(),
            None,
        ),
        Some(entries) if entries.is_empty() => (
            RevisionsEmptyKind::NoRevisions,
            Some("  📭 No revisions found".into()),
            Vec::new(),
            None,
        ),
        Some(entries) => {
            let rows = entries
                .iter()
                .enumerate()
                .map(|(i, r)| revision_row_label(r, i, now))
                .collect();
            (RevisionsEmptyKind::HasRows, None, rows, Some(rev.index))
        }
    };

    let count = rows.len();
    RevisionsVm {
        title: format!(
            "Revisions: {label} {}",
            crate::tui::render::count_label(count, count)
        ),
        empty,
        empty_message,
        rows,
        selected,
        footer,
        footer_colored,
        hscroll: rev.hscroll,
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

    let items: Vec<ListItem> = match revs.empty {
        RevisionsEmptyKind::HasRows => revs
            .rows
            .iter()
            .map(|row| ListItem::new(crate::tui::render::hscroll_str(row, revs.hscroll)))
            .collect(),
        _ => {
            let msg = revs.empty_message.clone().unwrap_or_else(|| "  ".into());
            vec![ListItem::new(msg).style(Style::default().fg(state.theme.dim))]
        }
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(revs.title.clone())
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.theme.accent))
                .style(state.theme.base_style())
                .padding(Padding::horizontal(1)),
        )
        .style(state.theme.base_style())
        .highlight_style(
            Style::default()
                .bg(state.theme.accent)
                .fg(state.theme.fg_on_accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(revs.selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if chrome.mouse_enabled {
        layout.list = Some(PaneHit {
            rect: chunks[0],
            offset: list_state.offset(),
        });
    }
    crate::tui::render_footer(
        frame,
        chunks[1],
        "",
        &revs.footer,
        revs.footer_colored,
        &state.theme,
        layout,
    );
    if chrome.mouse_enabled {
        layout.close_button = Some(crate::tui::render_close_button(frame, area, &state.theme));
    }
}

pub(crate) fn revisions_palette_items(state: &AppState) -> Vec<crate::tui::palette::PaletteItem> {
    use crate::tui::palette::key_item;
    let g = |code| revisions_guard(state, code);
    vec![
        key_item(
            "Enter",
            "Diff parent → revision",
            KeyCode::Enter,
            g(KeyCode::Enter),
        ),
        key_item(
            "D",
            "Diff revision vs head",
            KeyCode::Char('D'),
            g(KeyCode::Char('D')),
        ),
        key_item(
            "r",
            "Restore revision",
            KeyCode::Char('r'),
            g(KeyCode::Char('r')),
        ),
        // The palette *is* gated through `revisions_guard` here, unlike the real handler's
        // own `F` arm (which stays unconditional — see the comment on that case in
        // `revisions_guard`): cycling the target file doesn't need the revision list loaded,
        // so `revisions_guard`'s `F` case checks file count, not `has_entries` (issue #288).
        key_item(
            "F",
            "Cycle target file",
            KeyCode::Char('F'),
            g(KeyCode::Char('F')),
        ),
        key_item("q", "Back", KeyCode::Char('q'), true),
        key_item("?", "Help", KeyCode::Char('?'), true),
    ]
}
