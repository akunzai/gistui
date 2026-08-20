//! Pure presentation seam: `AppState` (+ pin-sync cache) → immutable view models.
//!
//! The draw path builds a [`ViewModel`] once per frame and paints from it for every screen.
//! Builders never touch the filesystem or network (issues #241 / #250).

use super::render::{
    gist_info_line, gist_row_label, is_json_file, spinner_glyph, unix_now, CREATE_DESC_PREFIX,
    CREATE_DESC_SUFFIX,
};
use super::screens::lookup;
use super::{
    AppState, DetailFocus, FocusPane, GistView, PaletteMode, PendingAction, Screen, TextInput,
};
use crate::ranking::RankedGistFile;
use ratatui::style::Color;

/// Full-frame presentation contract produced by [`build_view_model`].
#[derive(Debug, Clone, PartialEq)]
pub struct ViewModel {
    pub chrome: ChromeVm,
    pub screen: ScreenVm,
}

/// Cross-screen chrome facts shared by every body (top bar / overlays).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChromeVm {
    /// Whether mouse hit targets (close button, list hits) should be recorded.
    pub mouse_enabled: bool,
    /// Background task overlay message, if any.
    pub bg_task_msg: Option<String>,
    pub spinner_frame: usize,
}

/// Per-screen body presentation contract (#250).
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenVm {
    List(ListVm),
    Gists(GistsVm),
    GistDetail(GistDetailVm),
    Revisions(RevisionsVm),
    Config(ConfigVm),
    Diff(DiffVm),
    Preview(PreviewVm),
    Pins(PinsVm),
    Confirm(ConfirmVm),
    Help(HelpVm),
    Palette(PaletteVm),
}

/// Command palette / context menu overlay (#250). `background` is the already-built ViewModel
/// for the screen underneath (issue #272) — `None` for a Confirm-origin (still unpainted, #277)
/// or Palette-origin (unreachable: the palette can't be opened while itself active).
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteVm {
    pub background: Option<Box<ScreenVm>>,
    pub title: &'static str,
    pub has_query: bool,
    /// Live query text + cursor, painted as the input line in Command mode
    /// (`has_query`) — carried here so paint never reads `state.palette()` directly.
    pub query: TextInput,
    pub selected: usize,
    pub items: Vec<PaletteRowVm>,
    pub key_width: usize,
    pub mode: PaletteMode,
    pub anchor: Option<(u16, u16)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteRowVm {
    pub key_hint: String,
    pub label: String,
    /// What the action risks, resolved by the keymap table so paint only looks it up.
    pub category: crate::tui::keymap::Category,
    pub enabled: bool,
}

/// Compact-gist confirm background (info + file list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactGistBgVm {
    pub block_title: String,
    pub info_line: String,
    pub files: Vec<String>,
    pub file_cursor: usize,
}

/// Diff screen / confirm background pane facts (#250). Highlighting still applied at paint time
/// with the live theme (body text + ext are pure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffVm {
    pub title: String,
    /// Diff body after optional context collapse.
    pub body: String,
    pub footer: String,
    pub wrap: bool,
    pub scroll: u16,
    pub hscroll: u16,
    pub syntax_highlight: bool,
    pub ext: Option<String>,
}

/// Full-screen file preview (#250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewVm {
    pub title: String,
    /// Raw file content (one logical source; paint may highlight/wrap).
    pub body: String,
    pub footer: String,
    pub footer_colored: bool,
    pub wrap: bool,
    pub scroll: u16,
    pub hscroll: u16,
    pub syntax_highlight: bool,
    pub ext: Option<String>,
}

/// Revision history list (#250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionsVm {
    pub pane: ListPaneVm,
    pub footer: String,
    pub footer_colored: bool,
}

/// Settings screen (#250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigVm {
    pub rows: Vec<String>,
    pub selected: usize,
    pub status: Option<String>,
}

/// Single-gist detail presentation (#250). Layout geometry still filled during paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GistDetailVm {
    /// No `detail.gist_id` or group missing from state.
    pub missing: bool,
    pub block_title: String,
    pub info_line: String,
    pub focus: DetailFocus,
    pub files: Vec<String>,
    pub files_title: String,
    pub file_cursor: usize,
    pub comments_count: u32,
    pub comments: CommentsPaneVm,
    pub footer: String,
    pub footer_colored: bool,
    pub description_input: Option<TextInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentsPaneVm {
    Loading,
    /// Not loaded yet — prompt to open the tab.
    PromptLoad,
    Error {
        message: String,
    },
    Empty,
    Thread {
        title: String,
        affordance: CommentsAffordance,
        lines: Vec<CommentLineVm>,
        scroll: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentsAffordance {
    LoadingMore,
    LoadOlder,
    StartOfThread,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentLineVm {
    Author { text: String },
    Body { text: String },
    Blank,
}

/// Gist-manager list presentation (#250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GistsVm {
    pub pane: ListPaneVm,
    pub filtering: bool,
    pub filter_query: crate::tui::text_input::TextInput,
    pub footer_title: String,
    pub footer: String,
    pub footer_colored: bool,
}

/// Main dual-pane List screen presentation (#250).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListVm {
    pub local: ListPaneVm,
    pub gist: ListPaneVm,
    pub footer: ListFooterVm,
}

/// A pane title split into the parts that must survive and the one part that may shrink,
/// joined to the pane width at paint time by `render::fit_title` (#338).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTitleVm {
    /// Joined with ` · `, most- to least-important. A segment that does not fit is dropped
    /// whole — never half-shown — together with everything after it.
    pub segments: Vec<String>,
    /// Trailing re-derivable context (the Local pane's working directory). The only part that
    /// may be shortened behind a `…`, and the first to give way when the title is too narrow.
    pub context: Option<String>,
    /// Abbreviated `head`, used only when spending the difference buys back a state segment
    /// the full head would have dropped. Drop the pane's *name*, never its number, count, or
    /// markers — the head still has to identify the pane it labels.
    pub short_head: Option<String>,
}

impl PaneTitleVm {
    /// A title whose first segment — pane label, count, and the markers that must never be
    /// clipped — is `head`.
    pub fn new(head: String) -> Self {
        Self {
            segments: vec![head],
            context: None,
            short_head: None,
        }
    }

    /// Append one more segment, less important than everything already pushed.
    pub fn push(&mut self, segment: impl Into<String>) {
        self.segments.push(segment.into());
    }

    /// Append the active filter as `/query`; a blank query adds nothing.
    pub fn push_filter(&mut self, query: &str) {
        if !query.is_empty() {
            self.push(format!("/{query}"));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPaneVm {
    pub title: PaneTitleVm,
    /// Accent border plus a solid highlight bar. `false` paints a dim border and bolds the
    /// selected row instead; screens with a single pane are always focused.
    pub focused: bool,
    pub selected: Option<usize>,
    pub empty: ListPaneEmpty,
    /// Prebuilt empty/loading/filter-miss message when [`Self::empty`] is not [`HasRows`].
    pub empty_message: Option<String>,
    pub rows: Vec<RowVm>,
    /// Horizontal offset, applied to the selected row only (#341).
    pub hscroll: u16,
    /// Paint a scrollbar when the rows overflow the viewport.
    pub scrollbar: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListPaneEmpty {
    HasRows,
    /// Local scan or gist fetch in progress with no rows yet.
    Loading,
    NoItems,
    NoFilterMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowVm {
    /// Full row text including any mark prefix, before horizontal scroll.
    pub label: String,
    pub emphasis: RowEmphasis,
}

/// How one row stands out from its neighbours. Resolved by the view-model builder so the paint
/// side only looks it up — the domain reasons behind it (an exact filename match, a pinned
/// mapping with a missing side) stay on this side of the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowEmphasis {
    #[default]
    None,
    /// Bolded row — List's exact-filename match.
    Strong,
    /// Painted in the delete colour — a pinned mapping whose local side is missing.
    Danger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListFooterVm {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinsVm {
    pub pane: ListPaneVm,
    pub filtering: bool,
    pub filter_query: crate::tui::text_input::TextInput,
    pub footer_title: String,
    pub footer: String,
    pub footer_colored: bool,
}

/// Confirm modal + which background to paint under it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmVm {
    pub title: &'static str,
    pub border: Color,
    pub kind: ConfirmModalKind,
    pub background: ConfirmBackgroundVm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmBackgroundVm {
    /// Standard overwrite/upload/create backdrop: pre-built diff view model.
    Diff(DiffVm),
    /// Compaction confirm: gist info + file list.
    CompactGist(CompactGistBgVm),
    /// Missing group or nothing to show.
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmModalKind {
    /// Static y/n (or multi-key) prompt body.
    Prompt { text: String },
    /// Create-flow description editor.
    DescriptionInput {
        prefix: &'static str,
        input: TextInput,
        suffix: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpVm {
    pub mode: HelpModeVm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpModeVm {
    Index {
        items: Vec<HelpIndexItemVm>,
        selected: usize,
    },
    Topic {
        title: String,
        /// Plain lines for the topic body (About is pre-formatted without ratatui spans).
        lines: Vec<String>,
        scroll: u16,
        /// Repo-link row index in About body lines, when this is the About topic.
        about_repo_line: Option<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpIndexItemVm {
    pub key: String,
    pub title: String,
}

/// Pure chrome facts shared across screens (and palette backgrounds).
pub(crate) fn build_chrome(state: &AppState) -> ChromeVm {
    ChromeVm {
        mouse_enabled: state.mouse_enabled,
        bg_task_msg: state.bg_task_msg.clone(),
        spinner_frame: state.spinner_frame,
    }
}

/// Pure: map app state (+ pin sync cache) into a view model. No FS / network / mutation.
pub fn build_view_model(state: &AppState) -> ViewModel {
    ViewModel {
        chrome: build_chrome(state),
        screen: (lookup(&state.screen).build_vm)(state),
    }
}

/// ViewModel for the screen a palette is covering, by its origin's tag. `state`'s accessors
/// (`config()`/`help()`/etc., #242) already resolve through a palette-parked payload, so these
/// build fns are called directly rather than on `p.origin_screen` itself.
///
/// `None` for Confirm (blank background preserved as-is, tracked separately in #277) and Palette
/// (unreachable — the palette can't be opened while itself active).
pub(crate) fn build_background_screen_vm(state: &AppState, origin: &Screen) -> Option<ScreenVm> {
    match origin {
        Screen::Confirm(_) | Screen::Palette(_) => None,
        other => Some((lookup(other).build_vm)(state)),
    }
}

pub(crate) fn build_compact_gist_bg_vm(state: &AppState, gist_id: &str) -> Option<CompactGistBgVm> {
    let group = state.group_by_id(gist_id)?;
    let block_title = if group.description.trim().is_empty() {
        format!("Gist {}", group.id)
    } else {
        format!("Gist: {}", group.description)
    };
    let files = state.gist_file_display_names(gist_id);
    let file_cursor = state
        .detail()
        .map(|d| d.file_cursor)
        .unwrap_or(0)
        .min(files.len().saturating_sub(1));
    Some(CompactGistBgVm {
        block_title,
        info_line: gist_info_line(
            &group,
            unix_now(),
            state.current_user_login.as_deref(),
            state.gist_is_starred(gist_id),
            state.gist_counts(gist_id),
        ),
        files,
        file_cursor,
    })
}

pub(crate) fn file_ext(name: &str) -> Option<String> {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
}

/// Full gist file-list row as painted — [`gist_row_label`] plus `★ ` when the gist is starred.
/// Hscroll max must measure this string (or [`marked_row_text`] of it), not the star-less label.
pub(crate) fn gist_row_display(g: &RankedGistFile, view: GistView, state: &AppState) -> String {
    let label = gist_row_label(g, view);
    if state.gist_is_starred(&g.file.gist_id) {
        format!("★ {label}")
    } else {
        label
    }
}

/// The prompt shown inside the centered confirm modal — one line per pending action,
/// listing the keys that resolve it. Pure so it can be unit-tested.
pub(crate) fn confirm_prompt(state: &AppState) -> String {
    match state.pending_action() {
        Some(PendingAction::Create { .. }) if state.editing_description => {
            // `_` is the plain-text caret; the rendered modal draws a reverse-video
            // cursor at its real position instead (see render_confirm).
            format!(
                "{CREATE_DESC_PREFIX}{}_{CREATE_DESC_SUFFIX}",
                state.description_input
            )
        }
        Some(PendingAction::Create { local_path }) => {
            let desc = if state.description_input.is_empty() {
                "no description".to_string()
            } else {
                format!("desc: {}", state.description_input)
            };
            format!(
                "Create gist from {} ({desc})?  s secret  p public  Esc cancel",
                crate::config::display_path(local_path)
            )
        }
        Some(PendingAction::Upload {
            gist_id: _,
            filename: _,
            local_path: _,
        }) if state.upload.watching => {
            format!(
                "{} watching for edits — close the editor to continue  ·  n cancel",
                spinner_glyph(state.spinner_frame)
            )
        }
        Some(PendingAction::Upload {
            gist_id,
            filename,
            local_path,
        }) => {
            let edited_status = if state.upload.edited_content.is_some() {
                " [edited]"
            } else {
                ""
            };
            let mut opts = format!("y yes  n/Esc cancel  e edit{edited_status}");
            if is_json_file(local_path) {
                let pretty_status = if state.upload.json_pretty {
                    " [on]"
                } else {
                    " [off]"
                };
                let sort_status = if state.upload.json_sort {
                    " [on]"
                } else {
                    " [off]"
                };
                opts.push_str(&format!("  p pretty{pretty_status}  s sort{sort_status}"));
            }
            format!("Upload {filename} to gist {gist_id}?  ·  {opts}")
        }
        Some(PendingAction::Delete { gist_id, label }) => {
            format!("Permanently delete \"{label}\" ({gist_id})? (y/n)")
        }
        Some(PendingAction::RemoveFile {
            gist_id, filename, ..
        }) => {
            format!("Remove {filename} from gist {gist_id}? (y/n)")
        }
        Some(PendingAction::CompactGist { label, count, .. }) => {
            format!(
                "Compact {count} revisions of \"{label}\" into one? This force-pushes and cannot be undone. (y/n)"
            )
        }
        Some(PendingAction::RestoreRevision {
            filename,
            version_label,
            ..
        }) => {
            format!(
                "Restore {filename} to revision {version_label}? This uploads old content as a new revision. (y/n)"
            )
        }
        _ => format!(
            "Overwrite {}? (y/n)",
            crate::config::display_path(&state.download_target())
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{PinnedMapping, SyncStatus};
    use crate::tui::{initial_state, ConfigState, HelpState};
    use std::path::PathBuf;

    #[test]
    fn pins_vm_reads_cache_not_requiring_disk_for_status() {
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("notes.txt"),
            gist_id: "g1".into(),
            gist_filename: "notes.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
        // Hand-populate cache — builder must not need a real file.
        state.pin_sync_cache = vec![crate::tui::PinSyncCacheEntry {
            status: SyncStatus::InSync,
            local_ts: Some(100),
            remote_ts: Some(100),
        }];
        state.pin_sync_cache_dirty = false;

        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Pins(pins) => {
                assert_eq!(pins.pane.empty, ListPaneEmpty::HasRows);
                assert_eq!(pins.pane.rows.len(), 1);
                assert_eq!(pins.pane.rows[0].emphasis, RowEmphasis::None);
                assert!(pins.pane.rows[0].label.contains('✓'));
                assert!(pins.pane.rows[0].label.contains("notes.txt"));
            }
            other => panic!("expected Pins, got {other:?}"),
        }
    }

    #[test]
    fn pins_vm_unknown_when_cache_missing() {
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("a.txt"),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
        state.pin_sync_cache.clear();
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Pins(pins) => {
                assert_eq!(pins.pane.rows[0].emphasis, RowEmphasis::None);
                assert!(pins.pane.rows[0].label.starts_with('?'));
            }
            other => panic!("expected Pins, got {other:?}"),
        }
    }

    #[test]
    fn pins_vm_empty_states() {
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Pins(pins) => assert_eq!(pins.pane.empty, ListPaneEmpty::NoItems),
            other => panic!("expected Pins, got {other:?}"),
        }

        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("a.txt"),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
        if let Some(p) = state.pins_mut() {
            p.filter_query = crate::tui::TextInput::from("zzz-no-match");
        }
        state.pin_sync_cache = vec![crate::tui::PinSyncCacheEntry::default()];
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Pins(pins) => assert_eq!(pins.pane.empty, ListPaneEmpty::NoFilterMatch),
            other => panic!("expected Pins, got {other:?}"),
        }
    }

    #[test]
    fn confirm_vm_prompt_identity() {
        let mut state = initial_state();
        state.enter_confirm(
            PendingAction::Upload {
                gist_id: "g1".into(),
                filename: "notes.txt".into(),
                local_path: PathBuf::from("notes.txt"),
            },
            String::new(),
        );
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Confirm(c) => {
                assert_eq!(c.title, "Upload");
                match c.kind {
                    ConfirmModalKind::Prompt { text } => {
                        assert!(text.contains("Upload notes.txt to gist g1"));
                    }
                    other => panic!("expected Prompt, got {other:?}"),
                }
            }
            other => panic!("expected Confirm, got {other:?}"),
        }
    }

    #[test]
    fn confirm_vm_overwrite_download() {
        let mut state = initial_state();
        state.enter_diff(
            String::new(),
            String::new(),
            PathBuf::new(),
            PathBuf::from("notes.txt"),
        );
        state.enter_confirm_from_diff(PendingAction::Download);
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Confirm(c) => {
                assert_eq!(c.title, "Overwrite");
                match c.kind {
                    ConfirmModalKind::Prompt { text } => {
                        assert!(text.contains("Overwrite notes.txt"));
                    }
                    other => panic!("expected Prompt, got {other:?}"),
                }
            }
            other => panic!("expected Confirm, got {other:?}"),
        }
    }

    #[test]
    fn help_vm_index_lists_topics() {
        let mut state = initial_state();
        state.screen = Screen::Help(Box::new(HelpState {
            index_open: true,
            ..HelpState::default()
        }));
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::Help(h) => match h.mode {
                HelpModeVm::Index { items, selected } => {
                    assert!(!items.is_empty());
                    assert_eq!(selected, 0);
                    assert!(items.iter().any(|i| i.title.contains("List")));
                }
                other => panic!("expected Index, got {other:?}"),
            },
            other => panic!("expected Help, got {other:?}"),
        }
    }

    #[test]
    fn list_vm_empty_local_and_gist_messages() {
        let state = initial_state();
        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::List(list) => {
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
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn list_vm_rows_include_pin_mark_and_star() {
        use crate::domain::{GistFile, LocalCandidate, PinnedMapping};

        let mut state = initial_state();
        state.cwd = PathBuf::from("/tmp/proj");
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("notes.txt"),
            pinned: true,
            modified: None,
        }];
        state.gists = vec![GistFile {
            description: "demo notes".into(),
            ..GistFile::for_sync("g1".into(), "notes.txt".into(), None)
        }];
        state.starred_gist_ids.insert("g1".into());
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("notes.txt"),
            gist_id: "g1".into(),
            gist_filename: "notes.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
        state.focus = FocusPane::Local;
        state.anchor = FocusPane::Local;
        state.local_index = 0;
        state.gist_index = 0;

        let vm = build_view_model(&state);
        match vm.screen {
            ScreenVm::List(list) => {
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
                    .any(|r| matches!(r.emphasis, RowEmphasis::Strong) || r.label.contains('📌'));
                assert!(
                    marked,
                    "local={:?} gist={:?}",
                    list.local.rows, list.gist.rows
                );
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn list_vm_status_footer_and_filter_mode() {
        let mut state = initial_state();
        state.status = Some("Downloaded a.txt".into());
        match build_view_model(&state).screen {
            ScreenVm::List(list) => match list.footer {
                ListFooterVm::Status { text } => assert!(text.contains("Downloaded")),
                other => panic!("expected Status footer, got {other:?}"),
            },
            other => panic!("expected List, got {other:?}"),
        }

        state.status = None;
        state.filtering = true;
        state.focus = FocusPane::Gist;
        match build_view_model(&state).screen {
            ScreenVm::List(list) => match list.footer {
                ListFooterVm::Filtering { focus, .. } => assert_eq!(focus, FocusPane::Gist),
                other => panic!("expected Filtering footer, got {other:?}"),
            },
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn key_dense_screens_expose_contextual_action_hints() {
        let mut state = initial_state();
        let ScreenVm::List(list) = build_view_model(&state).screen else {
            panic!("expected List");
        };
        let ListFooterVm::Hints { text } = list.footer else {
            panic!("expected List hints");
        };
        assert!(text.contains("Enter diff") && text.contains("d download"));

        state.screen = Screen::Pins(Box::default());
        let ScreenVm::Pins(pins) = build_view_model(&state).screen else {
            panic!("expected Pins");
        };
        assert!(pins.footer.contains("s sync") && pins.footer.contains("x unpin"));
        assert!(
            pins.footer_title.contains("✓ synced") && pins.footer_title.contains("↓ remote newer")
        );

        state.screen = Screen::Gists(Box::default());
        let ScreenVm::Gists(gists) = build_view_model(&state).screen else {
            panic!("expected Gists");
        };
        assert!(gists.footer.contains("Enter detail") && gists.footer.contains("H revisions"));
    }

    #[test]
    fn gists_vm_empty_and_rows() {
        use crate::domain::GistFile;

        let mut state = initial_state();
        state.screen = Screen::Gists(Box::default());
        match build_view_model(&state).screen {
            ScreenVm::Gists(g) => {
                assert_eq!(g.pane.empty, ListPaneEmpty::NoItems);
                assert!(g
                    .pane
                    .empty_message
                    .as_deref()
                    .unwrap_or("")
                    .contains("No gists found"));
                assert!(g.pane.title.segments[0].contains("Gists"));
            }
            other => panic!("expected Gists, got {other:?}"),
        }

        state.gists = vec![
            GistFile {
                description: "alpha".into(),
                ..GistFile::for_sync("g1".into(), "a.txt".into(), None)
            },
            GistFile {
                description: "beta".into(),
                ..GistFile::for_sync("g2".into(), "b.txt".into(), None)
            },
        ];
        state.starred_gist_ids.insert("g1".into());
        state.gist_comment_counts.insert("g1".into(), 2);
        match build_view_model(&state).screen {
            ScreenVm::Gists(g) => {
                assert_eq!(g.pane.empty, ListPaneEmpty::HasRows);
                assert_eq!(g.pane.rows.len(), 2);
                let starred = g
                    .pane
                    .rows
                    .iter()
                    .find(|r| r.label.contains("alpha"))
                    .expect("g1's row");
                assert!(
                    starred.label.contains('★') || starred.label.contains("g1"),
                    "row: {}",
                    starred.label
                );
                assert!(starred.label.contains("💬") || starred.label.contains("g1"));
            }
            other => panic!("expected Gists, got {other:?}"),
        }
    }

    #[test]
    fn gists_vm_filter_miss_and_status_footer() {
        use crate::domain::GistFile;

        let mut state = initial_state();
        state.screen = Screen::Gists(Box::default());
        state.gists = vec![GistFile::for_sync("g1".into(), "a.txt".into(), None)];
        if let Some(gm) = state.gist_manager_mut() {
            gm.filter_query = crate::tui::TextInput::from("zzz-nope");
        }
        match build_view_model(&state).screen {
            ScreenVm::Gists(g) => assert_eq!(g.pane.empty, ListPaneEmpty::NoFilterMatch),
            other => panic!("expected Gists, got {other:?}"),
        }

        if let Some(gm) = state.gist_manager_mut() {
            gm.filter_query = crate::tui::TextInput::default();
        }
        state.status = Some("Compacted g1".into());
        match build_view_model(&state).screen {
            ScreenVm::Gists(g) => {
                assert!(!g.footer_colored);
                assert!(g.footer.contains("Compacted"));
            }
            other => panic!("expected Gists, got {other:?}"),
        }
    }

    #[test]
    fn gist_detail_vm_missing_without_id() {
        let mut state = initial_state();
        state.screen = Screen::GistDetail(Box::default());
        match build_view_model(&state).screen {
            ScreenVm::GistDetail(d) => {
                assert!(d.missing);
                assert!(matches!(d.comments, CommentsPaneVm::PromptLoad));
            }
            other => panic!("expected GistDetail, got {other:?}"),
        }
    }

    #[test]
    fn gist_detail_vm_files_and_comments_states() {
        use crate::domain::{GistComment, GistFile};

        let mut state = initial_state();
        state.screen = Screen::GistDetail(Box::default());
        state.gists = vec![
            GistFile {
                description: "demo".into(),
                content_type: Some("text/plain".into()),
                size: 1_536,
                ..GistFile::for_sync("g1".into(), "a.txt".into(), None)
            },
            GistFile::for_sync("g1".into(), "b.txt".into(), None),
        ];
        if let Some(d) = state.detail_mut() {
            d.gist_id = Some("g1".into());
            d.focus = DetailFocus::Files;
            d.file_cursor = 1;
        }
        state.gist_comment_counts.insert("g1".into(), 3);

        match build_view_model(&state).screen {
            ScreenVm::GistDetail(d) => {
                assert!(!d.missing);
                assert!(d.block_title.contains("demo") || d.block_title.contains("g1"));
                assert!(
                    d.info_line.contains("g1")
                        || d.info_line.contains("secret")
                        || d.info_line.contains("public")
                );
                assert_eq!(d.files.len(), 2);
                assert_eq!(d.files[0], "a.txt · 1.5 KiB · text/plain");
                assert_eq!(d.files_title, "Files (2): 1.5 KiB total");
                assert_eq!(d.comments_count, 3);
                assert_eq!(d.file_cursor, 1);
                assert_eq!(d.focus, DetailFocus::Files);
                assert!(matches!(d.comments, CommentsPaneVm::PromptLoad));
            }
            other => panic!("expected GistDetail, got {other:?}"),
        }

        if let Some(d) = state.detail_mut() {
            d.focus = DetailFocus::Comments;
            d.comments = Some(vec![GistComment {
                author: "alice".into(),
                body: "hello\nworld".into(),
                created_at: "2020-01-01T00:00:00Z".into(),
            }]);
            d.comments_total = Some(1);
            d.comments_loaded_oldest_page = 1;
        }
        match build_view_model(&state).screen {
            ScreenVm::GistDetail(d) => match d.comments {
                CommentsPaneVm::Thread {
                    title,
                    affordance,
                    lines,
                    ..
                } => {
                    assert!(title.contains("Comments"));
                    assert_eq!(affordance, CommentsAffordance::StartOfThread);
                    assert!(lines.iter().any(|l| matches!(
                        l,
                        CommentLineVm::Author { text } if text.contains("alice")
                    )));
                    assert!(lines.iter().any(|l| matches!(
                        l,
                        CommentLineVm::Body { text } if text.contains("hello")
                    )));
                }
                other => panic!("expected Thread, got {other:?}"),
            },
            other => panic!("expected GistDetail, got {other:?}"),
        }

        if let Some(d) = state.detail_mut() {
            d.comments = Some(vec![]);
        }
        match build_view_model(&state).screen {
            ScreenVm::GistDetail(d) => assert!(matches!(d.comments, CommentsPaneVm::Empty)),
            other => panic!("expected GistDetail, got {other:?}"),
        }
    }

    #[test]
    fn revisions_vm_loading_and_rows() {
        use crate::domain::{GistFile, GistRevision};

        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::default());
        if let Some(r) = state.revision_mut() {
            r.gist_id = Some("g1".into());
        }
        state.gists = vec![GistFile {
            description: "hist".into(),
            ..GistFile::for_sync("g1".into(), "a.txt".into(), None)
        }];
        match build_view_model(&state).screen {
            ScreenVm::Revisions(r) => {
                assert_eq!(r.pane.empty, ListPaneEmpty::Loading);
                assert!(r.footer.contains("Loading"));
                assert!(
                    r.pane.title.segments[0].contains("hist")
                        || r.pane.title.segments[0].contains("g1")
                );
            }
            other => panic!("expected Revisions, got {other:?}"),
        }

        if let Some(r) = state.revision_mut() {
            r.entries = Some(vec![GistRevision {
                version: "abc1234deadbeef".into(),
                committed_at: "2020-01-01T00:00:00Z".into(),
                user: "alice".into(),
                change_status: crate::domain::GistRevisionChangeStatus {
                    total: 1,
                    additions: 1,
                    deletions: 0,
                },
            }]);
            r.index = 0;
        }
        match build_view_model(&state).screen {
            ScreenVm::Revisions(r) => {
                assert_eq!(r.pane.empty, ListPaneEmpty::HasRows);
                assert_eq!(r.pane.rows.len(), 1);
                let row = &r.pane.rows[0].label;
                assert!(row.contains("alice") || row.contains("abc"));
                assert_eq!(r.pane.selected, Some(0));
            }
            other => panic!("expected Revisions, got {other:?}"),
        }
    }

    #[test]
    fn config_vm_rows_and_status() {
        let mut state = initial_state();
        state.screen = Screen::Config(Box::new(ConfigState { index: 1 }));
        state.status = Some("Theme saved".into());
        match build_view_model(&state).screen {
            ScreenVm::Config(c) => {
                assert!(!c.rows.is_empty());
                assert_eq!(c.selected, 1);
                assert_eq!(c.status.as_deref(), Some("Theme saved"));
                assert!(c
                    .rows
                    .iter()
                    .any(|r| r.contains("Theme") || r.contains("Mouse")));
            }
            other => panic!("expected Config, got {other:?}"),
        }
    }

    #[test]
    fn diff_vm_title_footer_and_body() {
        let mut state = initial_state();
        state.enter_diff(
            "--- a\n+++ b\n-old\n+new\n".into(),
            String::new(),
            PathBuf::new(),
            PathBuf::from("notes.txt"),
        );
        match build_view_model(&state).screen {
            ScreenVm::Diff(d) => {
                assert!(d.title.contains("Diff") || d.title.contains("notes"));
                assert!(d.body.contains("+new") || d.body.contains("old"));
                assert!(d.footer.contains("scroll") || d.footer.contains("back"));
                assert_eq!(d.ext.as_deref(), Some("txt"));
            }
            other => panic!("expected Diff, got {other:?}"),
        }
    }

    #[test]
    fn preview_vm_title_and_status_footer() {
        let mut state = initial_state();
        state.enter_preview(
            "gist: notes.txt".into(),
            "hello preview\n".into(),
            Some(("g1".into(), "notes.rs".into())),
        );
        state.status = Some("refresh failed".into());
        match build_view_model(&state).screen {
            ScreenVm::Preview(p) => {
                assert_eq!(p.title, "gist: notes.txt");
                assert!(p.body.contains("hello preview"));
                assert!(!p.footer_colored);
                assert!(p.footer.contains("refresh failed"));
                assert_eq!(p.ext.as_deref(), Some("rs"));
            }
            other => panic!("expected Preview, got {other:?}"),
        }
    }

    #[test]
    fn palette_vm_items_and_title() {
        use crate::tui::palette::{PaletteExec, PaletteItem, PaletteMode, PaletteState};
        use crossterm::event::KeyCode;

        let mut state = initial_state();
        state.screen = Screen::Palette(Box::new(PaletteState {
            mode: PaletteMode::Menu,
            items: vec![PaletteItem {
                key_hint: "d".into(),
                category: crate::tui::keymap::Category::Write,
                label: "download".into(),
                exec: PaletteExec::Key(KeyCode::Char('d'), crossterm::event::KeyModifiers::empty()),
                enabled: true,
                search: "d download".into(),
            }],
            selected: 0,
            origin_screen: Screen::List,
            anchor: Some((10, 5)),
            ..PaletteState::default()
        }));
        match build_view_model(&state).screen {
            ScreenVm::Palette(p) => {
                assert_eq!(p.title, "Menu");
                match p.background.as_deref() {
                    Some(ScreenVm::List(_)) => {}
                    other => panic!("expected List background, got {other:?}"),
                }
                assert_eq!(p.items.len(), 1);
                assert_eq!(p.items[0].label, "download");
                assert!(p.items[0].enabled);
                assert_eq!(p.anchor, Some((10, 5)));
            }
            other => panic!("expected Palette, got {other:?}"),
        }
    }

    #[test]
    fn palette_vm_background_stays_blank_over_confirm() {
        let mut state = initial_state();
        state.enter_confirm(
            PendingAction::Upload {
                gist_id: "g1".into(),
                filename: "notes.txt".into(),
                local_path: PathBuf::from("notes.txt"),
            },
            String::new(),
        );
        // `;` (menu) has no items over Confirm and won't open (palette.rs:60-66); Ctrl+P
        // (command palette) always opens because it also carries the cross-screen items.
        state.open_palette_command();
        match build_view_model(&state).screen {
            ScreenVm::Palette(p) => assert!(p.background.is_none()),
            other => panic!("expected Palette, got {other:?}"),
        }
    }

    #[test]
    fn confirm_vm_compact_background() {
        use crate::domain::GistFile;

        let mut state = initial_state();
        state.gists = vec![GistFile {
            description: "pack".into(),
            ..GistFile::for_sync("g1".into(), "a.txt".into(), None)
        }];
        state.enter_confirm(
            PendingAction::CompactGist {
                gist_id: "g1".into(),
                label: "pack".into(),
                count: 3,
            },
            String::new(),
        );
        match build_view_model(&state).screen {
            ScreenVm::Confirm(c) => match c.background {
                ConfirmBackgroundVm::CompactGist(bg) => {
                    assert!(bg.block_title.contains("pack") || bg.block_title.contains("g1"));
                    assert!(!bg.files.is_empty());
                }
                other => panic!("expected CompactGist bg, got {other:?}"),
            },
            other => panic!("expected Confirm, got {other:?}"),
        }
    }

    #[test]
    fn chrome_carries_bg_task() {
        let mut state = initial_state();
        state.bg_task_msg = Some("Working…".into());
        state.spinner_frame = 3;
        let vm = build_view_model(&state);
        assert_eq!(vm.chrome.bg_task_msg.as_deref(), Some("Working…"));
        assert_eq!(vm.chrome.spinner_frame, 3);
    }
}
