//! `Screen::Diff` — key handling, view-model, paint, and palette items colocated in one
//! file (issue #287, Phase 2).

use crate::tui::view_model::{ChromeVm, DiffVm};
use crate::tui::{AppState, HelpTopic, KeyOutcome, PendingAction};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::List;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    3
}

/// Shared "would this key actually do something" predicate for `Screen::Diff`, mirrored by
/// both [`AppState::handle_key_diff`]'s match-arm guards and `diff_palette_items` so the two
/// can never silently drift (issue #288).
pub(crate) fn diff_guard(state: &AppState, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('d' | 'u') => state.diff_allows_sync() && !state.diff_identical(),
        _ => false,
    }
}

impl AppState {
    pub(crate) fn handle_key_diff(&mut self, code: KeyCode) -> KeyOutcome {
        match code {
            // In the diff, q and Esc return to wherever `enter()` recorded (List, Pins, …).
            KeyCode::Char('q') | KeyCode::Esc => {
                // Diff pairing identity lives on the payload; leaving drops it (not a full
                // `back_to_list()` — that would also discard the rest of `nav_stack`).
                self.staged_diff_gist = None;
                self.leave();
            }
            // Identical files have nothing to sync, so download/upload are not offered.
            // Revision-history diffs are read-only (no local file pairing).
            KeyCode::Char('d') if diff_guard(self, code) => {
                return KeyOutcome::DownloadRequested {
                    target: self.download_target(),
                };
            }
            KeyCode::Char('u') if diff_guard(self, code) => {
                return self.upload_intent();
            }
            // Toggle between the configured context radius and the full file; the line
            // count changes, so reset the vertical scroll. The choice is persisted.
            KeyCode::Char('c') => {
                self.diff_show_full = !self.diff_show_full;
                if let Some(d) = self.diff_mut() {
                    d.scroll = 0;
                }
                return KeyOutcome::PersistDiffContext;
            }
            // Soft-wrap long lines instead of horizontal scrolling; reset the now-meaningless
            // horizontal offset so wrapped lines start at column 0.
            KeyCode::Char('w') => {
                self.diff_wrap = !self.diff_wrap;
                if let Some(d) = self.diff_mut() {
                    d.hscroll = 0;
                }
            }
            _ => {}
        }
        KeyOutcome::None
    }
}

/// The diff pane title. The gist id, filenames, and both sides' mtimes live in the diff's
/// `--- / +++` header lines (see `diff_labels`); the title stays concise and avoids
/// repeating a path.
pub(crate) fn diff_title(state: &AppState) -> String {
    match state.pending_action() {
        Some(PendingAction::Upload {
            gist_id, filename, ..
        }) => format!("Upload → gist {gist_id} / {filename}"),
        Some(PendingAction::Create { local_path }) => {
            format!(
                "Create gist from {}",
                crate::config::display_path(local_path)
            )
        }
        Some(PendingAction::Delete { gist_id, .. }) => {
            format!("Delete gist {gist_id}")
        }
        Some(PendingAction::RemoveFile {
            gist_id, filename, ..
        }) => {
            format!("Remove {filename} from gist {gist_id}")
        }
        _ => {
            let label = if state.diff_identical() {
                "Diff (identical)"
            } else {
                "Diff"
            };
            let local = state.preview_local();
            let target = state.download_target();
            if local.as_os_str().is_empty() || local == target {
                format!("{label} → {}", crate::config::display_path(&target))
            } else {
                format!(
                    "{label}: {} → {}",
                    crate::config::display_path(&local),
                    crate::config::display_path(&target)
                )
            }
        }
    }
}

/// The `Screen::Diff` preview: the diff pane plus a scroll/commands footer.
///
/// #72 audit: this footer intentionally does not surface `state.status`. Diff actions (`d`/`u`)
/// transition to `Screen::Confirm` or to the IO that lands back on `List`; their results surface
/// on those destination screens (which read `state.status`), so no status is set while on Diff.
/// Footer hints for `Screen::Diff` (pure for tests).
pub(crate) fn diff_footer(state: &AppState) -> String {
    let context = if state.diff_show_full {
        "c context [full]".to_string()
    } else {
        format!("c context [{}]", state.diff_context)
    };
    // When wrapping, horizontal scroll (←→) is meaningless — drop it from the hint.
    let scroll = if state.diff_wrap {
        "↑↓ PgUp/Dn scroll"
    } else {
        "↑↓←→ PgUp/Dn scroll"
    };
    let wrap = if state.diff_wrap {
        "w wrap [on]"
    } else {
        "w wrap [off]"
    };
    let back = "Esc/q back";
    if !state.diff_allows_sync() {
        if state.diff_identical() {
            format!("Files are identical  ·  {scroll}  ·  {wrap}  ·  {context}  ·  {back}")
        } else {
            format!("{scroll}  ·  {wrap}  ·  {context}  ·  {back}")
        }
    } else if state.diff_identical() {
        format!("Files are identical — nothing to sync  ·  {scroll}  ·  {wrap}  ·  {context}  ·  {back}")
    } else {
        format!("{scroll}  ·  d download  ·  u upload  ·  {wrap}  ·  {context}  ·  {back}")
    }
}

/// Diff pane facts — also used as Confirm overwrite background (non-compact).
pub(crate) fn build_diff_vm(state: &AppState) -> DiffVm {
    let text = state.diff_body_text();
    let body = match state.effective_diff_context() {
        Some(radius) => crate::diff::collapse_context(text, radius),
        None => text.to_string(),
    };
    let download_target = state.download_target();
    let preview_local = state.preview_local();
    let ext = download_target
        .file_name()
        .or_else(|| preview_local.file_name())
        .and_then(|n| n.to_str())
        .and_then(crate::tui::view_model::file_ext);
    DiffVm {
        title: diff_title(state),
        body,
        footer: diff_footer(state),
        wrap: state.diff_wrap,
        scroll: state.diff_scroll(),
        hscroll: state.diff_hscroll(),
        syntax_highlight: state.syntax_highlight,
        ext,
    }
}

pub(crate) fn render_diff_vm(
    frame: &mut Frame,
    state: &AppState,
    diff: &DiffVm,
    chrome: &ChromeVm,
    layout: &mut crate::tui::MouseLayout,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    let footer_lines =
        crate::tui::wrap_line_count(&diff.footer, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(footer_lines)])
        .split(area);

    crate::tui::render_diff_pane_vm(frame, chunks[0], diff, &state.theme);

    crate::tui::render_footer(
        frame,
        chunks[1],
        "",
        &diff.footer,
        true,
        &state.theme,
        layout,
    );
    if chrome.mouse_enabled {
        layout.close_button = Some(crate::tui::render_close_button(frame, area, &state.theme));
    }
}

pub(crate) fn diff_palette_items(state: &AppState) -> Vec<crate::tui::palette::PaletteItem> {
    use crate::tui::palette::key_item;
    let g = |code| diff_guard(state, code);
    vec![
        key_item("d", "Download", KeyCode::Char('d'), g(KeyCode::Char('d'))),
        key_item("u", "Upload", KeyCode::Char('u'), g(KeyCode::Char('u'))),
        key_item("c", "Toggle full diff context", KeyCode::Char('c'), true),
        key_item("w", "Toggle line wrap", KeyCode::Char('w'), true),
        key_item("q", "Back", KeyCode::Char('q'), true),
    ]
}
