use crate::domain::{group_gists, GistComment, GistFile, GistGroup, LocalCandidate, PinnedMapping};
use crate::ranking::{rank_gist_files, rank_local_files, MatchReason, RankedGistFile, RankedLocal};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    Local,
    Gist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    List,
    Diff,
    Confirm,
    Preview,
    Help,
    Pins,
    Gists,
    /// Single-gist detail: basic info + file list + comments (entered from Gists with Enter).
    GistDetail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    Download,
    Upload {
        gist_id: String,
        filename: String,
        local_path: PathBuf,
    },
    Create {
        local_path: PathBuf,
    },
    Delete {
        gist_id: String,
        label: String,
    },
    RemoveFile {
        gist_id: String,
        filename: String,
        label: String,
    },
    CompactGist {
        gist_id: String,
        label: String,
        count: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GistView {
    Description,
    Id,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GistTypeFilter {
    All,
    Public,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GistSort {
    Match,
    Name,
    Recent,
}

/// Sort order for the gist-level view (`Screen::Gists`). The `gh` list already
/// arrives updated-first, so `Updated` mirrors that; `Created` re-sorts by age.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GistGroupSort {
    Updated,
    Created,
}

impl GistGroupSort {
    fn next(self) -> Self {
        match self {
            GistGroupSort::Updated => GistGroupSort::Created,
            GistGroupSort::Created => GistGroupSort::Updated,
        }
    }

    fn label(self) -> &'static str {
        match self {
            GistGroupSort::Updated => "updated",
            GistGroupSort::Created => "created",
        }
    }
}

impl GistSort {
    fn next(self) -> Self {
        match self {
            GistSort::Match => GistSort::Name,
            GistSort::Name => GistSort::Recent,
            GistSort::Recent => GistSort::Match,
        }
    }

    fn label(self) -> &'static str {
        match self {
            GistSort::Match => "match",
            GistSort::Name => "name",
            GistSort::Recent => "recent",
        }
    }

    /// Re-orders ranked gists. `Match` keeps the incoming order (ranking score, or the
    /// gh list order when no local is selected); the others override it.
    fn apply(self, gists: &mut [RankedGistFile]) {
        match self {
            GistSort::Match => {}
            GistSort::Name => gists.sort_by(|a, b| a.file.filename.cmp(&b.file.filename)),
            GistSort::Recent => {
                gists.sort_by(|a, b| b.file.updated_at.cmp(&a.file.updated_at));
            }
        }
    }
}

/// Sort order for the local file pane. Mirrors [`GistSort`]: `Match` keeps the
/// incoming order (reverse-ranking score when the gist pane drives, else discovery
/// order); the others override it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalSort {
    Match,
    Name,
    Recent,
}

impl LocalSort {
    fn next(self) -> Self {
        match self {
            LocalSort::Match => LocalSort::Name,
            LocalSort::Name => LocalSort::Recent,
            LocalSort::Recent => LocalSort::Match,
        }
    }

    fn label(self) -> &'static str {
        match self {
            LocalSort::Match => "match",
            LocalSort::Name => "name",
            LocalSort::Recent => "recent",
        }
    }

    fn apply(self, locals: &mut [RankedLocal]) {
        match self {
            LocalSort::Match => {}
            LocalSort::Name => locals.sort_by(|a, b| {
                a.candidate
                    .path
                    .file_name()
                    .cmp(&b.candidate.path.file_name())
            }),
            // Most-recently-modified first; unknown mtimes (None) sort last.
            LocalSort::Recent => locals.sort_by_key(|r| std::cmp::Reverse(r.candidate.modified)),
        }
    }
}

impl GistTypeFilter {
    fn matches(self, public: bool) -> bool {
        match self {
            GistTypeFilter::All => true,
            GistTypeFilter::Public => public,
            GistTypeFilter::Secret => !public,
        }
    }

    fn next(self) -> Self {
        match self {
            GistTypeFilter::All => GistTypeFilter::Public,
            GistTypeFilter::Public => GistTypeFilter::Secret,
            GistTypeFilter::Secret => GistTypeFilter::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            GistTypeFilter::All => "all",
            GistTypeFilter::Public => "public",
            GistTypeFilter::Secret => "secret",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    None,
    Quit,
    PreviewDiff,
    Download,
    DownloadGist,
    Pin,
    Unpin,
    UploadAdd,
    UploadPreview,
    Upload,
    Create(bool),
    PreviewContent,
    OpenBrowser,
    EditLocal,
    EditUpload,
    ExecuteDelete,
    ExecuteRemoveFile,
    /// Open the selected gist's detail screen and fetch its comments in the background.
    OpenGistDetail,
    /// Analyse the selected Gist-manager gist's revision count, then ask to confirm a compaction.
    CompactGist,
    /// Run the confirmed compaction (clone → squash → force-push) on the pending gist.
    ExecuteCompactGist,
    ApplyDescription,
    RefreshLocals,
    RefreshPreview,
    UnpinAtPin,
    /// Smart-sync the selected Pins-screen pair (direction from mtime).
    SyncPinAuto,
    /// Force push the selected Pins-screen pair (upload local → gist).
    SyncPinPush,
    /// Force pull the selected Pins-screen pair (download gist → local).
    SyncPinPull,
    /// Smart-sync the selected local↔gist pair from the List screen (if pinned).
    SyncSelectedPair,
    /// Diff the selected pinned pair (read-only, lands on Screen::Diff; q/Esc returns to Pins).
    PreviewPinDiff,
    /// Persist the diff-context toggle (`diff_show_full`) to config after pressing `c`.
    PersistDiffContext,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub locals: Vec<LocalCandidate>,
    pub gists: Vec<GistFile>,
    pub pinned: Vec<PinnedMapping>,
    pub focus: FocusPane,
    /// The pane that DRIVES the match ranking, decoupled from `focus`: the anchored pane
    /// shows natural order; the other pane is always ranked against the anchor's selection.
    /// `focus` only moves the cursor/highlight and does not affect ranking.
    pub anchor: FocusPane,
    pub local_index: usize,
    pub gist_index: usize,
    pub local_hscroll: u16,
    pub gist_hscroll: u16,
    pub screen: Screen,
    pub pending_action: Option<PendingAction>,
    pub gist_view: GistView,
    pub gist_type_filter: GistTypeFilter,
    pub gist_sort: GistSort,
    pub local_sort: LocalSort,
    pub filtering: bool,
    pub filter_query: String,
    pub diff_previewed: bool,
    pub diff_text: String,
    pub diff_scroll: u16,
    pub diff_hscroll: u16,
    pub diff_identical: bool,
    /// Unchanged context lines kept around each change in the diff view (from config).
    pub diff_context: u32,
    /// When true the diff view shows the full file; when false it collapses to
    /// `diff_context` lines. Toggled with `c` and persisted to config.
    pub diff_show_full: bool,
    pub preview_remote: String,
    pub preview_local: PathBuf,
    pub download_target: PathBuf,
    pub cwd: PathBuf,
    pub status: Option<String>,
    pub loading: bool,
    pub preview_title: String,
    pub preview_gist_key: Option<(String, String)>,
    /// Screen to return to when leaving the full-screen preview (default: List; set to
    /// GistDetail when a detail-view file preview is launched).
    pub preview_return: Screen,
    /// A `(gist_id, filename)` explicitly chosen for preview (e.g. a number key in the detail
    /// view), taken by the `PreviewContent` IO step; when `None` it falls back to the selected
    /// gist file on the list. Keeps `handle_key` pure: it records the intent, `run_loop` fetches.
    pub preview_request: Option<(String, String)>,
    pub gist_content_cache: std::collections::HashMap<(String, String), String>,
    pub local_recursive: bool,
    pub skip_dirs: Vec<String>,
    pub scan_depth: u32,
    pub local_scanning: bool,
    pub pins_index: usize,
    pub gists_index: usize,
    pub gists_hscroll: u16,
    pub gists_sort: GistGroupSort,
    pub gists_type_filter: GistTypeFilter,
    pub gists_filtering: bool,
    pub gists_filter_query: String,
    pub editing_description: bool,
    pub description_input: String,
    pub bg_task_msg: Option<String>,
    /// Set after the first `q`/`Esc` on the main list; a second press confirms the quit. Any
    /// other key clears it. Prevents an accidental single-key exit.
    pub quit_armed: bool,
    pub help_scroll: u16,
    pub upload_original_content: String,
    pub upload_edited_content: Option<String>,
    pub upload_json_pretty: bool,
    pub upload_json_sort: bool,
    pub upload_remote_content: Option<String>,
    pub upload_local_label: Option<String>,
    pub upload_gist_label: Option<String>,
    /// gist_id of the active download (set when entering the diff Confirm for a pull).
    pub download_gist_id: Option<String>,
    /// filename of the active download (set when entering the diff Confirm for a pull).
    pub download_gist_filename: Option<String>,
    /// Screen to return to when leaving the diff (default: List; set to Pins for pin diffs).
    pub diff_return: Screen,
    /// The gist currently shown in `Screen::GistDetail`; also guards stale comment responses.
    pub detail_gist_id: Option<String>,
    /// Comments: `None` means loading, `Some` is the fetched result.
    pub detail_comments: Option<Vec<GistComment>>,
    /// Comment-fetch error message, if any.
    pub detail_comments_error: Option<String>,
    /// Comment-pane scroll offset.
    pub detail_scroll: u16,
    /// Screen to return to after a compaction confirm is cancelled/finished (Gists or GistDetail).
    pub compact_return_screen: Screen,
}

fn unranked_gists(gists: Vec<GistFile>) -> Vec<RankedGistFile> {
    gists
        .into_iter()
        .map(|file| RankedGistFile {
            file,
            score: 0,
            reasons: Vec::new(),
        })
        .collect()
}

fn unranked_locals(locals: &[LocalCandidate]) -> Vec<RankedLocal> {
    locals
        .iter()
        .cloned()
        .map(|candidate| RankedLocal {
            candidate,
            score: 0,
            reasons: Vec::new(),
        })
        .collect()
}

impl AppState {
    pub fn upload_local_path(&self) -> Option<std::path::PathBuf> {
        match &self.pending_action {
            Some(PendingAction::Upload { local_path, .. }) => Some(local_path.clone()),
            _ => None,
        }
    }

    pub fn content_to_upload(&self) -> String {
        let base = self
            .upload_edited_content
            .as_ref()
            .unwrap_or(&self.upload_original_content);
        if let Some(local_path) = self.upload_local_path() {
            if is_json_file(&local_path) {
                if let Ok(transformed) = crate::domain::transform_json(
                    base,
                    self.upload_json_pretty,
                    self.upload_json_sort,
                ) {
                    return transformed;
                }
            }
        }
        base.clone()
    }

    pub fn update_upload_diff(&mut self) {
        let local_content = self.content_to_upload();
        let remote = self
            .upload_remote_content
            .as_ref()
            .cloned()
            .unwrap_or_default();
        let local_label = self.upload_local_label.clone().unwrap_or_default();
        let gist_label = self.upload_gist_label.clone().unwrap_or_default();

        let diff = crate::diff::unified_diff(&gist_label, &remote, &local_label, &local_content);
        self.diff_text = diff;
    }

    pub fn init_upload_state(
        &mut self,
        local_path: &std::path::Path,
        remote_content: Option<String>,
        local_label: String,
        gist_label: String,
    ) {
        self.upload_original_content = std::fs::read_to_string(local_path).unwrap_or_default();
        self.upload_edited_content = None;
        self.upload_json_pretty = false;
        self.upload_json_sort = false;
        self.upload_remote_content = remote_content;
        self.upload_local_label = Some(local_label);
        self.upload_gist_label = Some(gist_label);
        self.update_upload_diff();
    }

    pub fn ranked_gists(&self) -> Vec<RankedGistFile> {
        let query = self.filter_query.to_lowercase();
        let gists: Vec<GistFile> = self
            .gists
            .iter()
            .filter(|g| self.gist_type_filter.matches(g.public))
            .filter(|g| {
                query.is_empty()
                    || g.filename.to_lowercase().contains(&query)
                    || g.description.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        // Anchor-driven ranking: the gist pane is ranked against the selected local file
        // only while the LOCAL pane is the anchor (anchor == Local). When the gist pane
        // is the anchor it uses its own sort (no ranking), which also breaks the
        // otherwise-mutual dependency with `visible_locals`.
        // NOTE: only evaluate `selected_local()` inside the anchor==Local branch. Computing
        // it eagerly (e.g. in the match scrutinee) would recurse: selected_local ->
        // visible_locals -> selected_gist -> ranked_gists.
        let mut ranked = if self.anchor == FocusPane::Local {
            match self.selected_local() {
                Some(local) => rank_gist_files(&local.path, &gists, &self.pinned),
                None => unranked_gists(gists),
            }
        } else {
            unranked_gists(gists)
        };
        self.gist_sort.apply(&mut ranked);
        ranked
    }

    /// The local file list after sorting (and, while the gist pane drives, reverse ranking
    /// against the selected gist). Single source of truth for the local pane's order,
    /// selection, and rendering — mirrors `ranked_gists`.
    pub fn visible_locals(&self) -> Vec<RankedLocal> {
        // Mirror of `ranked_gists`: only evaluate `selected_gist()` in the anchor==Gist
        // branch to avoid recursing back through `ranked_gists` -> `selected_local`.
        let mut ranked = if self.anchor == FocusPane::Gist {
            match self.selected_gist() {
                Some(gist) => rank_local_files(&gist.file, &self.locals, &self.pinned),
                None => unranked_locals(&self.locals),
            }
        } else {
            unranked_locals(&self.locals)
        };
        self.local_sort.apply(&mut ranked);
        ranked
    }

    pub fn selected_local(&self) -> Option<LocalCandidate> {
        self.visible_locals()
            .into_iter()
            .nth(self.local_index)
            .map(|r| r.candidate)
    }

    pub fn selected_gist(&self) -> Option<RankedGistFile> {
        self.ranked_gists().into_iter().nth(self.gist_index)
    }

    /// All gists collapsed to one entry each (raw, unfiltered).
    pub fn gist_groups(&self) -> Vec<GistGroup> {
        group_gists(&self.gists)
    }

    /// The gist-level view's rows after the visibility filter, text filter, and sort
    /// are applied. This is the single source of truth for navigation, selection, and
    /// rendering in `Screen::Gists`.
    pub fn visible_gist_groups(&self) -> Vec<GistGroup> {
        let query = self.gists_filter_query.to_lowercase();
        let mut groups: Vec<GistGroup> = self
            .gist_groups()
            .into_iter()
            .filter(|g| self.gists_type_filter.matches(g.public))
            .filter(|g| {
                query.is_empty()
                    || g.description.to_lowercase().contains(&query)
                    || g.id.to_lowercase().contains(&query)
            })
            .collect();
        match self.gists_sort {
            GistGroupSort::Updated => groups.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
            GistGroupSort::Created => groups.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        }
        groups
    }

    /// The gist highlighted in the gist-level view.
    pub fn selected_group(&self) -> Option<GistGroup> {
        self.visible_gist_groups().into_iter().nth(self.gists_index)
    }

    /// Highest horizontal-scroll offset for the gist-level view, based on its longest
    /// visible row (mirrors `focused_hscroll_max` for the main panes).
    fn gists_hscroll_max(&self) -> u16 {
        self.visible_gist_groups()
            .iter()
            .map(|g| {
                gist_group_row_label(g, unix_now(), self.gists_sort)
                    .chars()
                    .count()
            })
            .max()
            .unwrap_or(0)
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16
    }

    /// Number of files the given gist holds in the current in-memory list. Used to guard
    /// against removing a gist's only file (GitHub forbids a fileless gist).
    fn gist_file_count(&self, gist_id: &str) -> usize {
        self.gists.iter().filter(|g| g.gist_id == gist_id).count()
    }

    /// Filenames the given gist holds in the current in-memory list (gh order).
    pub fn gist_filenames(&self, gist_id: &str) -> Vec<String> {
        self.gists
            .iter()
            .filter(|g| g.gist_id == gist_id)
            .map(|g| g.filename.clone())
            .collect()
    }

    /// Look up a gist group by id (unaffected by filtering); used by detail + confirm background.
    pub fn group_by_id(&self, gist_id: &str) -> Option<GistGroup> {
        self.gist_groups().into_iter().find(|g| g.id == gist_id)
    }

    /// The gist the current screen acts on: the gist-level cursor on `Gists`, the
    /// viewed gist on `GistDetail`, otherwise the gist owning the selected file row.
    /// Screen-aware so IO actions (open-in-browser, compact) target what the user sees.
    pub fn context_gist_id(&self) -> Option<String> {
        match self.screen {
            Screen::Gists => self.selected_group().map(|g| g.id),
            Screen::GistDetail => self.detail_gist_id.clone(),
            _ => self.selected_gist().map(|g| g.file.gist_id),
        }
    }

    /// Upload intent shared by the list and the diff screen: requires a selected local file
    /// and gist, then branches on whether the gist already holds a file of the local name
    /// (case C: preview + confirm overwrite) or not (case B: add directly).
    fn upload_intent(&mut self) -> KeyOutcome {
        let (Some(local), Some(gist)) = (self.selected_local(), self.selected_gist()) else {
            self.status = Some("select a local file and a gist to upload".into());
            return KeyOutcome::None;
        };
        let Some(local_filename) = local
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
        else {
            self.status = Some("local file has no name".into());
            return KeyOutcome::None;
        };
        let gist_id = gist.file.gist_id.clone();
        let has_same_name = self
            .gists
            .iter()
            .any(|g| g.gist_id == gist_id && g.filename == local_filename);
        if has_same_name {
            KeyOutcome::UploadPreview
        } else {
            KeyOutcome::UploadAdd
        }
    }

    /// Highest horizontal-scroll offset for the focused pane, based on its longest row
    /// (viewport width is unknown to the pure key logic, mirroring the diff scroll cap).
    fn focused_hscroll_max(&self) -> u16 {
        let longest = match self.focus {
            FocusPane::Local => self
                .locals
                .iter()
                .map(|c| local_row_label(&c.path, &self.cwd).chars().count())
                .max(),
            FocusPane::Gist => self
                .ranked_gists()
                .iter()
                .map(|g| gist_row_label(g, self.gist_view).chars().count())
                .max(),
        };
        longest
            .unwrap_or(0)
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16
    }

    fn scroll_focused_right(&mut self) {
        let max = self.focused_hscroll_max();
        let scroll = match self.focus {
            FocusPane::Local => &mut self.local_hscroll,
            FocusPane::Gist => &mut self.gist_hscroll,
        };
        if *scroll < max {
            *scroll += 1;
        }
    }

    fn scroll_focused_left(&mut self) {
        let scroll = match self.focus {
            FocusPane::Local => &mut self.local_hscroll,
            FocusPane::Gist => &mut self.gist_hscroll,
        };
        *scroll = scroll.saturating_sub(1);
    }

    /// Reset the non-anchor ("ranked") pane to its top match: the pane that re-ranks
    /// whenever the anchor pane's selection changes.
    fn reset_ranked_pane(&mut self) {
        match self.anchor {
            FocusPane::Local => {
                self.gist_index = 0;
                self.gist_hscroll = 0;
            }
            FocusPane::Gist => {
                self.local_index = 0;
                self.local_hscroll = 0;
            }
        }
    }

    pub fn enter_diff(
        &mut self,
        diff_text: String,
        remote: String,
        local: PathBuf,
        target: PathBuf,
    ) {
        self.diff_text = diff_text;
        self.preview_remote = remote;
        self.preview_local = local;
        self.download_target = target;
        self.diff_previewed = true;
        self.diff_scroll = 0;
        self.diff_hscroll = 0;
        self.diff_identical = false;
        self.status = None;
        self.screen = Screen::Diff;
    }

    pub fn back_to_list(&mut self) {
        self.screen = Screen::List;
        self.pending_action = None;
        self.diff_text.clear();
        self.preview_remote.clear();
        self.preview_local = PathBuf::new();
        self.download_target = PathBuf::new();
        // Clear the pull's pinned-pair identity so a later non-pinned download
        // can't be mis-attributed to a pin via a stale gist id/filename.
        self.download_gist_id = None;
        self.download_gist_filename = None;
        self.diff_scroll = 0;
        self.diff_hscroll = 0;
        self.diff_identical = false;
        self.diff_previewed = false;
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some(message.into());
    }

    pub fn scroll_diff_down(&mut self) {
        let max = self
            .diff_text
            .lines()
            .count()
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16;
        if self.diff_scroll < max {
            self.diff_scroll += 1;
        }
    }

    pub fn scroll_diff_up(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_sub(1);
    }

    pub fn scroll_diff_right(&mut self) {
        let max = self
            .diff_text
            .lines()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0)
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16;
        if self.diff_hscroll < max {
            self.diff_hscroll += 1;
        }
    }

    pub fn scroll_diff_left(&mut self) {
        self.diff_hscroll = self.diff_hscroll.saturating_sub(1);
    }

    /// Context radius to render the diff with: `None` shows the full file, `Some(n)`
    /// collapses unchanged regions to `n` lines around each change.
    pub fn effective_diff_context(&self) -> Option<usize> {
        if self.diff_show_full {
            None
        } else {
            Some(self.diff_context as usize)
        }
    }

    pub fn handle_key(&mut self, code: KeyCode) -> KeyOutcome {
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

    fn handle_key_help(&mut self, code: KeyCode) -> KeyOutcome {
        match code {
            KeyCode::Down => {
                self.help_scroll = self.help_scroll.saturating_add(1);
                KeyOutcome::None
            }
            KeyCode::Up => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
                KeyOutcome::None
            }
            _ => {
                // Any other key just closes the help overlay back to the list.
                self.screen = Screen::List;
                self.help_scroll = 0;
                KeyOutcome::None
            }
        }
    }

    fn handle_key_pins(&mut self, code: KeyCode) -> KeyOutcome {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.screen = Screen::List,
            KeyCode::Down if self.pins_index + 1 < self.pinned.len() => {
                self.pins_index += 1;
            }
            KeyCode::Up if self.pins_index > 0 => {
                self.pins_index -= 1;
            }
            KeyCode::Enter if !self.pinned.is_empty() => return KeyOutcome::PreviewPinDiff,
            KeyCode::Char('x') if !self.pinned.is_empty() => return KeyOutcome::UnpinAtPin,
            KeyCode::Char('s') if !self.pinned.is_empty() => return KeyOutcome::SyncPinAuto,
            KeyCode::Char('u') if !self.pinned.is_empty() => return KeyOutcome::SyncPinPush,
            KeyCode::Char('d') if !self.pinned.is_empty() => return KeyOutcome::SyncPinPull,
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
                KeyCode::Backspace => {
                    self.description_input.pop();
                }
                KeyCode::Char(c) => self.description_input.push(c),
                _ => {}
            }
            return KeyOutcome::None;
        }
        // Inline text filter: capture the query until Enter (keep) or Esc (clear).
        if self.gists_filtering {
            match code {
                KeyCode::Esc => {
                    self.gists_filter_query.clear();
                    self.gists_filtering = false;
                    self.gists_index = 0;
                    self.gists_hscroll = 0;
                }
                KeyCode::Enter => self.gists_filtering = false,
                KeyCode::Backspace => {
                    self.gists_filter_query.pop();
                    self.gists_index = 0;
                    self.gists_hscroll = 0;
                }
                KeyCode::Char(c) => {
                    self.gists_filter_query.push(c);
                    self.gists_index = 0;
                    self.gists_hscroll = 0;
                }
                _ => {}
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
                    self.description_input = group.description.clone();
                }
            }
            KeyCode::Enter if self.gists_index < groups.len() => {
                return KeyOutcome::OpenGistDetail;
            }
            KeyCode::Char('o') if self.gists_index < groups.len() => {
                return KeyOutcome::OpenBrowser
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
            KeyCode::Down => self.detail_scroll = self.detail_scroll.saturating_add(1),
            KeyCode::Up => self.detail_scroll = self.detail_scroll.saturating_sub(1),
            KeyCode::PageDown => self.detail_scroll = self.detail_scroll.saturating_add(10),
            KeyCode::PageUp => self.detail_scroll = self.detail_scroll.saturating_sub(10),
            KeyCode::Char('o') => return KeyOutcome::OpenBrowser,
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
            _ => {}
        }
        KeyOutcome::None
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
            KeyCode::Down => self.scroll_diff_down(),
            KeyCode::Up => self.scroll_diff_up(),
            KeyCode::Right => self.scroll_diff_right(),
            KeyCode::Left => self.scroll_diff_left(),
            _ => {}
        }
        KeyOutcome::None
    }

    fn handle_key_filter(&mut self, code: KeyCode) -> KeyOutcome {
        match code {
            KeyCode::Esc => {
                self.filter_query.clear();
                self.filtering = false;
                self.gist_index = 0;
                self.gist_hscroll = 0;
            }
            KeyCode::Enter => self.filtering = false,
            KeyCode::Backspace => {
                self.filter_query.pop();
                self.gist_index = 0;
                self.gist_hscroll = 0;
            }
            KeyCode::Char(c) => {
                self.filter_query.push(c);
                self.gist_index = 0;
                self.gist_hscroll = 0;
            }
            _ => {}
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
            KeyCode::Down => match self.focus {
                FocusPane::Local if self.local_index + 1 < self.locals.len() => {
                    self.local_index += 1;
                    self.local_hscroll = 0;
                    if self.anchor == FocusPane::Local {
                        self.reset_ranked_pane();
                    }
                }
                FocusPane::Gist if self.gist_index + 1 < self.ranked_gists().len() => {
                    self.gist_index += 1;
                    self.gist_hscroll = 0;
                    if self.anchor == FocusPane::Gist {
                        self.reset_ranked_pane();
                    }
                }
                _ => {}
            },
            KeyCode::Up => match self.focus {
                FocusPane::Local if self.local_index > 0 => {
                    self.local_index -= 1;
                    self.local_hscroll = 0;
                    if self.anchor == FocusPane::Local {
                        self.reset_ranked_pane();
                    }
                }
                FocusPane::Gist if self.gist_index > 0 => {
                    self.gist_index -= 1;
                    self.gist_hscroll = 0;
                    if self.anchor == FocusPane::Gist {
                        self.reset_ranked_pane();
                    }
                }
                _ => {}
            },
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
            KeyCode::Char('s') => {
                // Cycle the focused pane's sort: match -> name -> recent -> match.
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
            KeyCode::Char('r') => {
                self.local_recursive = !self.local_recursive;
                self.local_index = 0;
                self.local_hscroll = 0;
                return KeyOutcome::RefreshLocals;
            }
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('?') => self.screen = Screen::Help,
            KeyCode::Char('P') => {
                self.pins_index = 0;
                self.screen = Screen::Pins;
            }
            KeyCode::Char('S') => return KeyOutcome::SyncSelectedPair,
            KeyCode::Char('g') => {
                if self.gists.is_empty() {
                    self.status = Some("no gists to manage".into());
                    return KeyOutcome::None;
                }
                // Reset the gist-level view's own filters so the target is always
                // visible, then land on the gist that owns the selected file row.
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
            KeyCode::Char('p') => {
                let (Some(local), Some(gist)) = (self.selected_local(), self.selected_gist())
                else {
                    self.status = Some("select a local file and a gist to pin".into());
                    return KeyOutcome::None;
                };
                let already = self.pinned.iter().any(|m| {
                    m.local_path == local.path
                        && m.gist_id == gist.file.gist_id
                        && m.gist_filename == gist.file.filename
                });
                return if already {
                    KeyOutcome::Unpin
                } else {
                    KeyOutcome::Pin
                };
            }
            KeyCode::Char('u') => return self.upload_intent(),
            KeyCode::Char('X') => {
                let Some(gist) = self.selected_gist() else {
                    self.status = Some("select a gist file to remove".into());
                    return KeyOutcome::None;
                };
                let gist_id = gist.file.gist_id.clone();
                let filename = gist.file.filename.clone();
                // A gist must keep at least one file; deleting the whole gist lives in the
                // gist-level view (g -> X) instead.
                if self.gist_file_count(&gist_id) <= 1 {
                    self.status = Some(format!(
                        "{filename} is the gist's only file — use g then X to delete the gist"
                    ));
                    return KeyOutcome::None;
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
            KeyCode::Char('n') => {
                let Some(local) = self.selected_local() else {
                    self.status = Some("select a local file to create a gist".into());
                    return KeyOutcome::None;
                };
                // Create is a two-step confirm: type an optional description (inline
                // editor, shared with the gist-level view), then choose visibility.
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
            _ => {}
        }
        KeyOutcome::None
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
        match code {
            KeyCode::Down => {
                self.scroll_diff_down();
                return KeyOutcome::None;
            }
            KeyCode::Up => {
                self.scroll_diff_up();
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
                KeyCode::Backspace => {
                    self.description_input.pop();
                }
                KeyCode::Char(c) => self.description_input.push(c),
                _ => {}
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

enum BgTaskOutcome {
    PreviewDiff {
        result: std::result::Result<String, String>,
        local_path: Option<PathBuf>,
        local_label: String,
        gist_label: String,
        target: PathBuf,
        // True when the local pane was focused at trigger time: frame the preview as an
        // upload (old = gist, new = local) instead of a download.
        upload_orientation: bool,
    },
    DownloadSelected {
        result: std::result::Result<String, String>,
        target: PathBuf,
        local_label: String,
        gist_label: String,
        gist_id: String,
        filename: String,
    },
    UploadPreview {
        result: std::result::Result<String, String>,
        gist_id: String,
        filename: String,
        local_path: PathBuf,
        local_label: String,
        gist_label: String,
    },
    UploadReplace {
        result: std::result::Result<(), String>,
        gist_id: String,
        filename: String,
    },
    CreateGist {
        result: std::result::Result<(), String>,
        local_path: PathBuf,
        public: bool,
    },
    PreviewContent {
        result: std::result::Result<String, String>,
        key: (String, String),
        preview_title: String,
    },
    DeleteGist {
        result: std::result::Result<(), String>,
        gist_id: String,
    },
    RemoveFile {
        result: std::result::Result<(), String>,
        gist_id: String,
        filename: String,
    },
    ApplyDescription {
        result: std::result::Result<(), String>,
        gist_id: String,
    },
    CompactAnalyze {
        result: std::result::Result<usize, String>,
        gist_id: String,
        label: String,
    },
    CompactGist {
        result: std::result::Result<(), String>,
        label: String,
        count: usize,
    },
    CommentsFetched {
        gist_id: String,
        result: Result<Vec<GistComment>, String>,
    },
}

pub fn initial_state() -> AppState {
    AppState {
        locals: Vec::new(),
        gists: Vec::new(),
        pinned: Vec::new(),
        focus: FocusPane::Local,
        anchor: FocusPane::Local,
        local_index: 0,
        gist_index: 0,
        local_hscroll: 0,
        gist_hscroll: 0,
        screen: Screen::List,
        pending_action: None,
        gist_view: GistView::Description,
        gist_type_filter: GistTypeFilter::All,
        gist_sort: GistSort::Match,
        local_sort: LocalSort::Match,
        filtering: false,
        filter_query: String::new(),
        diff_previewed: false,
        diff_text: String::new(),
        diff_scroll: 0,
        diff_hscroll: 0,
        diff_identical: false,
        diff_context: 3,
        diff_show_full: false,
        preview_remote: String::new(),
        preview_local: PathBuf::new(),
        download_target: PathBuf::new(),
        cwd: PathBuf::from("."),
        status: None,
        loading: false,
        preview_title: String::new(),
        preview_gist_key: None,
        preview_return: Screen::List,
        preview_request: None,
        gist_content_cache: std::collections::HashMap::new(),
        local_recursive: false,
        skip_dirs: crate::config::AppConfig::default().skip_dirs,
        scan_depth: crate::config::AppConfig::default().scan_depth,
        local_scanning: false,
        pins_index: 0,
        gists_index: 0,
        gists_hscroll: 0,
        gists_sort: GistGroupSort::Updated,
        gists_type_filter: GistTypeFilter::All,
        gists_filtering: false,
        gists_filter_query: String::new(),
        editing_description: false,
        description_input: String::new(),
        bg_task_msg: None,
        quit_armed: false,
        help_scroll: 0,
        upload_original_content: String::new(),
        upload_edited_content: None,
        upload_json_pretty: false,
        upload_json_sort: false,
        upload_remote_content: None,
        upload_local_label: None,
        upload_gist_label: None,
        download_gist_id: None,
        download_gist_filename: None,
        diff_return: Screen::List,
        detail_gist_id: None,
        detail_comments: None,
        detail_comments_error: None,
        detail_scroll: 0,
        compact_return_screen: Screen::Gists,
    }
}

pub fn load_startup_state() -> Result<AppState> {
    let mut state = initial_state();
    let config_path = crate::config::config_path()?;
    let config = crate::config::load_config(&config_path)?;
    let cwd = std::env::current_dir()?;

    state.pinned = config.pinned;
    state.skip_dirs = config.skip_dirs;
    state.scan_depth = config.scan_depth;
    state.diff_context = config.diff_context;
    state.diff_show_full = config.diff_show_full;
    state.locals = crate::local::discover_local_candidates(
        &cwd,
        &state.pinned,
        false,
        &state.skip_dirs,
        state.scan_depth,
    )?;
    state.cwd = cwd;
    // Start focused on the gist pane: the common flow is to pick a gist and pull it
    // into the cwd, and the gist list is shown even when no local file is selected.
    state.focus = FocusPane::Gist;
    // The gist list is fetched off-thread by run_loop so the TUI appears instantly.
    state.loading = true;
    // Show last-known gists immediately from the on-disk cache; the background fetch
    // refreshes them once it completes.
    if let Ok(path) = crate::cache::cache_path() {
        state.gists = crate::cache::load_cached_gists(&path);
    }

    Ok(state)
}

fn cache_gists(gists: &[GistFile]) {
    if let Ok(path) = crate::cache::cache_path() {
        crate::cache::save_cached_gists(&path, gists);
    }
}

/// Fetches the gist list on a background thread so startup does not block on `gh`.
/// Mirrors the previous graceful degradation: an empty list on any error.
fn spawn_gist_fetch() -> std::sync::mpsc::Receiver<Vec<GistFile>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let gists = if crate::gh::check_gh_ready().is_ok() {
            crate::gh::fetch_gist_list_json()
                .and_then(|raw| crate::gh::parse_gist_list_json(&raw))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let _ = tx.send(gists);
    });
    rx
}

fn spawn_local_scan(
    cwd: std::path::PathBuf,
    pinned: Vec<crate::domain::PinnedMapping>,
    recursive: bool,
    skip_dirs: Vec<String>,
    max_depth: u32,
) -> std::sync::mpsc::Receiver<Vec<LocalCandidate>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let candidates = crate::local::discover_local_candidates(
            &cwd, &pinned, recursive, &skip_dirs, max_depth,
        )
        .unwrap_or_default();
        let _ = tx.send(candidates);
    });
    rx
}

pub fn run() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);

    result
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut state = load_startup_state()?;
    let mut gist_rx = Some(spawn_gist_fetch());
    let mut local_rx: Option<std::sync::mpsc::Receiver<Vec<LocalCandidate>>> = None;
    let mut bg_rx: Option<std::sync::mpsc::Receiver<BgTaskOutcome>> = None;

    loop {
        terminal.draw(|frame| render(frame, &state))?;

        // Absorb the background gist list once it arrives.
        if state.loading {
            if let Some(ref rx) = gist_rx {
                if let Ok(gists) = rx.try_recv() {
                    cache_gists(&gists);
                    state.gists = gists;
                    state.loading = false;
                    if state.gist_index >= state.ranked_gists().len() {
                        state.gist_index = 0;
                    }
                    let count = state.visible_gist_groups().len();
                    if count > 0 && state.gists_index >= count {
                        state.gists_index = count - 1;
                    }
                    gist_rx = None;
                }
            }
        }

        // Absorb a completed background local scan.
        if state.local_scanning {
            if let Some(ref rx) = local_rx {
                if let Ok(locals) = rx.try_recv() {
                    let selected = state.selected_local().map(|c| c.path.clone());
                    state.locals = locals;
                    state.local_index = selected
                        .and_then(|path| state.locals.iter().position(|c| c.path == path))
                        .unwrap_or(0)
                        .min(state.locals.len().saturating_sub(1));
                    if state.gist_index >= state.ranked_gists().len() {
                        state.gist_index = 0;
                    }
                    state.local_scanning = false;
                    state.status = None;
                    local_rx = None;
                }
            }
        }

        // Absorb a completed background per-action task.
        if let Some(ref rx) = bg_rx {
            if let Ok(outcome) = rx.try_recv() {
                state.bg_task_msg = None;
                bg_rx = None;
                match outcome {
                    BgTaskOutcome::PreviewDiff {
                        result,
                        local_path,
                        local_label,
                        gist_label,
                        target,
                        upload_orientation,
                    } => match result {
                        Ok(remote) => {
                            let local_content = local_path
                                .as_ref()
                                .map(|path| std::fs::read_to_string(path).unwrap_or_default())
                                .unwrap_or_default();
                            let diff = preview_diff_text(
                                upload_orientation,
                                &local_label,
                                &local_content,
                                &gist_label,
                                &remote,
                            );
                            let identical = local_content == remote;
                            state.enter_diff(diff, remote, local_path.unwrap_or_default(), target);
                            state.diff_identical = identical;
                        }
                        Err(error) => state.set_status(format!("fetch failed: {error}")),
                    },
                    BgTaskOutcome::DownloadSelected {
                        result,
                        target,
                        local_label,
                        gist_label,
                        gist_id,
                        filename,
                    } => match result {
                        Ok(remote) => {
                            if target.exists() {
                                let local_content =
                                    std::fs::read_to_string(&target).unwrap_or_default();
                                let diff = crate::diff::unified_diff(
                                    &local_label,
                                    &local_content,
                                    &gist_label,
                                    &remote,
                                );
                                let identical = local_content == remote;
                                state.download_gist_id = Some(gist_id);
                                state.download_gist_filename = Some(filename);
                                state.enter_diff(diff, remote, target.clone(), target);
                                state.diff_identical = identical;
                            } else {
                                match crate::actions::execute_download(&target, &remote, false) {
                                    Ok(()) => {
                                        state
                                            .set_status(format!("Downloaded {}", target.display()));
                                        record_pin_sync(
                                            &mut state,
                                            &target,
                                            &gist_id,
                                            &filename,
                                            &remote,
                                            crate::domain::SyncDirection::Download,
                                        );
                                        refresh_locals(&mut state);
                                    }
                                    Err(error) => {
                                        state.set_status(format!("download failed: {error}"))
                                    }
                                }
                            }
                        }
                        Err(error) => state.set_status(format!("fetch failed: {error}")),
                    },
                    BgTaskOutcome::UploadPreview {
                        result,
                        gist_id,
                        filename,
                        local_path,
                        local_label,
                        gist_label,
                    } => match result {
                        Ok(remote) => {
                            state.pending_action = Some(PendingAction::Upload {
                                gist_id,
                                filename,
                                local_path: local_path.clone(),
                            });
                            state.init_upload_state(
                                &local_path,
                                Some(remote),
                                local_label,
                                gist_label,
                            );
                            state.diff_scroll = 0;
                            state.diff_hscroll = 0;
                            state.status = None;
                            state.screen = Screen::Confirm;
                        }
                        Err(error) => state.set_status(format!("fetch failed: {error}")),
                    },
                    BgTaskOutcome::UploadReplace {
                        result,
                        gist_id,
                        filename,
                    } => match result {
                        Ok(_) => {
                            state
                                .gist_content_cache
                                .remove(&(gist_id.clone(), filename.clone()));
                            state.set_status(format!("Uploaded {} to gist {}", filename, gist_id));
                            if let Some(local_path) = state.upload_local_path() {
                                let content = state.content_to_upload();
                                record_pin_sync(
                                    &mut state,
                                    &local_path,
                                    &gist_id,
                                    &filename,
                                    &content,
                                    crate::domain::SyncDirection::Upload,
                                );
                            }
                            state.back_to_list();
                            state.loading = true;
                            gist_rx = Some(spawn_gist_fetch());
                        }
                        Err(error) => {
                            state.set_status(format!("upload failed: {error}"));
                            state.screen = Screen::Confirm;
                        }
                    },
                    BgTaskOutcome::CreateGist {
                        result,
                        local_path,
                        public,
                    } => match result {
                        Ok(_) => {
                            let visibility = if public { "public" } else { "secret" };
                            state.set_status(format!(
                                "Created {} gist from {}",
                                visibility,
                                local_path.display()
                            ));
                            state.description_input.clear();
                            state.back_to_list();
                            state.loading = true;
                            gist_rx = Some(spawn_gist_fetch());
                        }
                        Err(error) => {
                            state.set_status(format!("create failed: {error}"));
                            state.screen = Screen::List;
                            state.pending_action = None;
                            state.description_input.clear();
                        }
                    },
                    BgTaskOutcome::PreviewContent {
                        result,
                        key,
                        preview_title,
                    } => match result {
                        Ok(content) => {
                            state
                                .gist_content_cache
                                .insert(key.clone(), content.clone());
                            state.preview_title = preview_title;
                            state.preview_gist_key = Some(key);
                            state.diff_text = content;
                            state.diff_scroll = 0;
                            state.diff_hscroll = 0;
                            state.status = None;
                            state.screen = Screen::Preview;
                        }
                        Err(error) => state.set_status(format!("fetch failed: {error}")),
                    },
                    BgTaskOutcome::DeleteGist { result, gist_id } => match result {
                        Ok(_) => {
                            state.set_status(format!("Deleted gist {gist_id}"));
                            state.loading = true;
                            gist_rx = Some(spawn_gist_fetch());
                        }
                        Err(error) => state.set_status(format!("delete failed: {error}")),
                    },
                    BgTaskOutcome::RemoveFile {
                        result,
                        gist_id,
                        filename,
                    } => match result {
                        Ok(_) => {
                            state
                                .gist_content_cache
                                .remove(&(gist_id.clone(), filename.clone()));
                            state.set_status(format!("Removed {filename} from gist {gist_id}"));
                            state.loading = true;
                            gist_rx = Some(spawn_gist_fetch());
                        }
                        Err(error) => state.set_status(format!("remove failed: {error}")),
                    },
                    BgTaskOutcome::ApplyDescription { result, gist_id } => match result {
                        Ok(_) => {
                            state.set_status(format!("Updated description for gist {gist_id}"));
                            state.loading = true;
                            gist_rx = Some(spawn_gist_fetch());
                        }
                        Err(error) => {
                            state.set_status(format!("description update failed: {error}"))
                        }
                    },
                    BgTaskOutcome::CompactAnalyze {
                        result,
                        gist_id,
                        label,
                    } => match result {
                        Ok(count) if count <= 1 => state.set_status(format!(
                            "\"{label}\" already has a single revision — nothing to compact"
                        )),
                        Ok(count) => {
                            state.diff_text = format!(
                                "Compact gist {gist_id} (\"{label}\").\n\nIt has {count} revisions. Compacting clones it to a temp dir, squashes the history to a single commit, and force-pushes — the {} older revisions are gone for good.",
                                count - 1
                            );
                            state.diff_scroll = 0;
                            state.diff_hscroll = 0;
                            state.pending_action = Some(PendingAction::CompactGist {
                                gist_id,
                                label,
                                count,
                            });
                            state.screen = Screen::Confirm;
                        }
                        Err(error) => state.set_status(format!("revision check failed: {error}")),
                    },
                    BgTaskOutcome::CompactGist {
                        result,
                        label,
                        count,
                    } => match result {
                        Ok(_) => {
                            state.set_status(format!(
                                "Compacted \"{label}\" ({count} → 1 revision)"
                            ));
                            state.loading = true;
                            gist_rx = Some(spawn_gist_fetch());
                        }
                        Err(error) => state.set_status(format!("compact failed: {error}")),
                    },
                    BgTaskOutcome::CommentsFetched { gist_id, result } => {
                        state.apply_fetched_comments(&gist_id, result);
                    }
                }
            }
        }

        // Poll so the loop also wakes to check the background fetches, not only on input.
        if !event::poll(std::time::Duration::from_millis(150))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            // Windows reports both Press and Release (and Repeat) for each
            // keystroke, while Unix terminals report only Press. Without this
            // filter every key fires twice on Windows — Tab toggles focus back
            // to where it started and Up/Down jump two rows. See ratatui#347.
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if state.bg_task_msg.is_some() {
                if key.code == KeyCode::Esc {
                    state.bg_task_msg = None;
                    bg_rx = None;
                    state.set_status("Cancelled");
                }
                continue;
            }

            match state.handle_key(key.code) {
                KeyOutcome::Quit => break,
                KeyOutcome::PreviewDiff => {
                    let Some(ranked) = state.selected_gist() else {
                        continue;
                    };
                    // List-originated diff returns to the List on Esc (reset any
                    // leftover Pins origin from an earlier pin diff).
                    state.diff_return = Screen::List;
                    let local_path = state.selected_local().map(|local| local.path.clone());
                    let gist = ranked.file.clone();
                    let gist_id = gist.gist_id.clone();
                    let filename = gist.filename.clone();
                    let (local_label, gist_label) = diff_labels(local_path.as_deref(), &gist);
                    let target = state.cwd.join(&filename);
                    let upload_orientation = state.focus == FocusPane::Local;

                    state.bg_task_msg = Some("Loading diff…".to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    bg_rx = Some(rx);
                    std::thread::spawn(move || {
                        let result = crate::gh::fetch_gist_file_content(&gist_id, &filename)
                            .map_err(|e| e.to_string());
                        let _ = tx.send(BgTaskOutcome::PreviewDiff {
                            result,
                            local_path,
                            local_label,
                            gist_label,
                            target,
                            upload_orientation,
                        });
                    });
                }
                KeyOutcome::Download => download(&mut state),
                KeyOutcome::DownloadGist => {
                    let Some(ranked) = state.selected_gist() else {
                        continue;
                    };
                    let gist = ranked.file.clone();
                    let gist_id = gist.gist_id.clone();
                    let filename = gist.filename.clone();
                    let target = state.cwd.join(&filename);
                    let (local_label, gist_label) = diff_labels(Some(&target), &gist);

                    state.bg_task_msg = Some("Downloading…".to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    bg_rx = Some(rx);
                    std::thread::spawn(move || {
                        let result = crate::gh::fetch_gist_file_content(&gist_id, &filename)
                            .map_err(|e| e.to_string());
                        let _ = tx.send(BgTaskOutcome::DownloadSelected {
                            result,
                            target,
                            local_label,
                            gist_label,
                            gist_id,
                            filename,
                        });
                    });
                }
                KeyOutcome::OpenGistDetail => {
                    let Some(group) = state.selected_group() else {
                        continue;
                    };
                    let gist_id = group.id.clone();
                    state.screen = Screen::GistDetail;
                    state.detail_gist_id = Some(gist_id.clone());
                    state.detail_comments = None;
                    state.detail_comments_error = None;
                    state.detail_scroll = 0;

                    state.bg_task_msg = Some("Loading comments…".to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    bg_rx = Some(rx);
                    let fetch_id = gist_id.clone();
                    std::thread::spawn(move || {
                        let result = crate::gh::fetch_gist_comments_json(&fetch_id)
                            .map_err(|e| e.to_string())
                            .and_then(|raw| {
                                crate::gh::parse_gist_comments_json(&raw).map_err(|e| e.to_string())
                            });
                        let _ = tx.send(BgTaskOutcome::CommentsFetched {
                            gist_id: fetch_id,
                            result,
                        });
                    });
                }
                KeyOutcome::CompactGist => {
                    let Some(gist_id) = state.context_gist_id() else {
                        continue;
                    };
                    let Some(group) = state.group_by_id(&gist_id) else {
                        continue;
                    };
                    let label = if group.description.trim().is_empty() {
                        group.id.clone()
                    } else {
                        group.description.clone()
                    };

                    state.bg_task_msg = Some("Checking revisions…".to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    bg_rx = Some(rx);
                    std::thread::spawn(move || {
                        let result = crate::actions::execute_command(
                            &crate::actions::gist_revision_count_command(&gist_id),
                        )
                        .map_err(|e| e.to_string())
                        .and_then(|out| {
                            crate::actions::parse_revision_count(&out)
                                .ok_or_else(|| "could not parse revision count".to_string())
                        });
                        let _ = tx.send(BgTaskOutcome::CompactAnalyze {
                            result,
                            gist_id,
                            label,
                        });
                    });
                }
                KeyOutcome::Pin => pin_selected(&mut state),
                KeyOutcome::Unpin => unpin_selected(&mut state),
                KeyOutcome::UploadAdd => {
                    let (Some(local), Some(gist)) = (state.selected_local(), state.selected_gist())
                    else {
                        continue;
                    };
                    let local_path = local.path.clone();
                    let gist_id = gist.file.gist_id.clone();
                    let Some(filename) = upload_local_filename(&local_path) else {
                        state.set_status("local file has no name");
                        continue;
                    };

                    state.pending_action = Some(PendingAction::Upload {
                        gist_id,
                        filename: filename.clone(),
                        local_path: local_path.clone(),
                    });

                    let local_label = format!("local: {}", local_path.display());
                    let gist_label = "(new file)".to_string();
                    state.init_upload_state(
                        &local_path,
                        Some(String::new()),
                        local_label,
                        gist_label,
                    );
                    state.screen = Screen::Confirm;
                }
                KeyOutcome::UploadPreview => {
                    let (Some(local), Some(gist)) = (state.selected_local(), state.selected_gist())
                    else {
                        continue;
                    };
                    let Some(filename) = upload_local_filename(&local.path) else {
                        state.set_status("local file has no name");
                        continue;
                    };
                    let gist_id = gist.file.gist_id.clone();
                    let gist_file = gist.file.clone();
                    let local_path = local.path.clone();
                    let (local_label, gist_label) = diff_labels(Some(&local_path), &gist_file);

                    state.bg_task_msg = Some("Loading diff…".to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    bg_rx = Some(rx);
                    std::thread::spawn(move || {
                        let result = crate::gh::fetch_gist_file_content(&gist_id, &filename)
                            .map_err(|e| e.to_string());
                        let _ = tx.send(BgTaskOutcome::UploadPreview {
                            result,
                            gist_id,
                            filename,
                            local_path,
                            local_label,
                            gist_label,
                        });
                    });
                }
                KeyOutcome::Upload => {
                    let Some(PendingAction::Upload {
                        gist_id,
                        filename,
                        local_path: _,
                    }) = state.pending_action.clone()
                    else {
                        continue;
                    };

                    let upload_content = state.content_to_upload();

                    // Generate unique temp directory in workspace
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    let temp_dir = state.cwd.join(format!(".gistui_upload_{timestamp}"));

                    if let Err(e) = std::fs::create_dir_all(&temp_dir) {
                        state.set_status(format!("failed to create temp dir: {e}"));
                        continue;
                    }

                    let temp_file_path = temp_dir.join(&filename);
                    if let Err(e) = std::fs::write(&temp_file_path, &upload_content) {
                        state.set_status(format!("failed to write temp file: {e}"));
                        let _ = std::fs::remove_dir_all(&temp_dir);
                        continue;
                    }

                    let has_same_name = state
                        .gists
                        .iter()
                        .any(|g| g.gist_id == gist_id && g.filename == filename);

                    let plan = if has_same_name {
                        let target = GistFile {
                            gist_id: gist_id.clone(),
                            description: String::new(),
                            filename: filename.clone(),
                            public: false,
                            updated_at: String::new(),
                            created_at: String::new(),
                        };
                        crate::actions::upload_command(&temp_file_path, &target)
                    } else {
                        crate::actions::upload_add_command(&temp_file_path, &gist_id)
                    };

                    state.back_to_list();
                    state.bg_task_msg = Some("Uploading…".to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    bg_rx = Some(rx);
                    std::thread::spawn(move || {
                        let result = crate::actions::execute_command(&plan)
                            .map(|_| ())
                            .map_err(|e| e.to_string());

                        let _ = std::fs::remove_dir_all(&temp_dir);

                        let _ = tx.send(BgTaskOutcome::UploadReplace {
                            result,
                            gist_id,
                            filename,
                        });
                    });
                }
                KeyOutcome::EditUpload => {
                    edit_upload_buffer(terminal, &mut state)?;
                }
                KeyOutcome::Create(public) => {
                    let Some(PendingAction::Create { local_path }) = state.pending_action.clone()
                    else {
                        continue;
                    };
                    let description = state.description_input.clone();
                    let plan = crate::actions::create_command(&local_path, public, &description);

                    state.bg_task_msg = Some("Creating gist…".to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    bg_rx = Some(rx);
                    std::thread::spawn(move || {
                        let result = crate::actions::execute_command(&plan)
                            .map(|_| ())
                            .map_err(|e| e.to_string());
                        let _ = tx.send(BgTaskOutcome::CreateGist {
                            result,
                            local_path,
                            public,
                        });
                    });
                }
                KeyOutcome::PreviewContent => {
                    // A detail-view number key records the exact file in `preview_request`;
                    // otherwise fall back to the file selected on the list.
                    let key = match state.preview_request.take() {
                        Some(key) => key,
                        None => match state.selected_gist() {
                            Some(gist) => (gist.file.gist_id.clone(), gist.file.filename.clone()),
                            None => continue,
                        },
                    };
                    if let Some(cached) = state.gist_content_cache.get(&key) {
                        state.preview_title = format!("Preview: {} / {}", key.0, key.1);
                        state.preview_gist_key = Some(key);
                        state.diff_text = cached.clone();
                        state.diff_scroll = 0;
                        state.diff_hscroll = 0;
                        state.status = None;
                        state.screen = Screen::Preview;
                    } else {
                        let gist_id = key.0.clone();
                        let filename = key.1.clone();
                        let preview_title = format!("Preview: {gist_id} / {filename}");
                        state.bg_task_msg = Some("Loading preview…".to_string());
                        let (tx, rx) = std::sync::mpsc::channel();
                        bg_rx = Some(rx);
                        std::thread::spawn(move || {
                            let result = crate::gh::fetch_gist_file_content(&gist_id, &filename)
                                .map_err(|e| e.to_string());
                            let _ = tx.send(BgTaskOutcome::PreviewContent {
                                result,
                                key,
                                preview_title,
                            });
                        });
                    }
                }
                KeyOutcome::RefreshPreview => {
                    if let Some(key) = state.preview_gist_key.clone() {
                        state.gist_content_cache.remove(&key);
                        let gist_id = key.0.clone();
                        let filename = key.1.clone();
                        let preview_title = format!("Preview: {gist_id} / {filename}");
                        state.bg_task_msg = Some("Loading preview…".to_string());
                        let (tx, rx) = std::sync::mpsc::channel();
                        bg_rx = Some(rx);
                        std::thread::spawn(move || {
                            let result = crate::gh::fetch_gist_file_content(&gist_id, &filename)
                                .map_err(|e| e.to_string());
                            let _ = tx.send(BgTaskOutcome::PreviewContent {
                                result,
                                key,
                                preview_title,
                            });
                        });
                    }
                }
                KeyOutcome::OpenBrowser => open_browser(&mut state),
                KeyOutcome::EditLocal => edit_local(terminal, &mut state)?,
                KeyOutcome::ExecuteDelete => {
                    let Some(PendingAction::Delete { gist_id, .. }) = state.pending_action.clone()
                    else {
                        continue;
                    };
                    let plan = crate::actions::delete_command(&gist_id);
                    state.back_to_list();

                    state.bg_task_msg = Some("Deleting gist…".to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    bg_rx = Some(rx);
                    std::thread::spawn(move || {
                        let result = crate::actions::execute_command(&plan)
                            .map(|_| ())
                            .map_err(|e| e.to_string());
                        let _ = tx.send(BgTaskOutcome::DeleteGist { result, gist_id });
                    });
                }
                KeyOutcome::ExecuteRemoveFile => {
                    let Some(PendingAction::RemoveFile {
                        gist_id, filename, ..
                    }) = state.pending_action.clone()
                    else {
                        continue;
                    };
                    let plan = crate::actions::remove_file_command(&gist_id, &filename);
                    state.back_to_list();

                    state.bg_task_msg = Some("Removing file…".to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    bg_rx = Some(rx);
                    std::thread::spawn(move || {
                        let result = crate::actions::execute_command(&plan)
                            .map(|_| ())
                            .map_err(|e| e.to_string());
                        let _ = tx.send(BgTaskOutcome::RemoveFile {
                            result,
                            gist_id,
                            filename,
                        });
                    });
                }
                KeyOutcome::ExecuteCompactGist => {
                    let Some(PendingAction::CompactGist {
                        gist_id,
                        label,
                        count,
                    }) = state.pending_action.clone()
                    else {
                        continue;
                    };
                    state.pending_action = None;
                    state.screen = state.compact_return_screen;

                    state.bg_task_msg = Some("Compacting revisions…".to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    bg_rx = Some(rx);
                    std::thread::spawn(move || {
                        let result = crate::actions::execute_compact_gist(&gist_id)
                            .map_err(|e| e.to_string());
                        let _ = tx.send(BgTaskOutcome::CompactGist {
                            result,
                            label,
                            count,
                        });
                    });
                }
                KeyOutcome::ApplyDescription => {
                    let Some(group) = state.selected_group() else {
                        state.editing_description = false;
                        continue;
                    };
                    let gist_id = group.id.clone();
                    let description = state.description_input.clone();
                    let plan = crate::actions::edit_description_command(&gist_id, &description);
                    state.editing_description = false;
                    state.description_input.clear();

                    state.bg_task_msg = Some("Updating description…".to_string());
                    let (tx, rx) = std::sync::mpsc::channel();
                    bg_rx = Some(rx);
                    std::thread::spawn(move || {
                        let result = crate::actions::execute_command(&plan)
                            .map(|_| ())
                            .map_err(|e| e.to_string());
                        let _ = tx.send(BgTaskOutcome::ApplyDescription { result, gist_id });
                    });
                }
                KeyOutcome::RefreshLocals => {
                    state.set_status("Scanning files…");
                    state.local_scanning = true;
                    local_rx = Some(spawn_local_scan(
                        state.cwd.clone(),
                        state.pinned.clone(),
                        state.local_recursive,
                        state.skip_dirs.clone(),
                        state.scan_depth,
                    ));
                }
                KeyOutcome::UnpinAtPin => unpin_at_pin_index(&mut state),
                KeyOutcome::SyncSelectedPair => {
                    let (Some(local), Some(gist)) = (state.selected_local(), state.selected_gist())
                    else {
                        continue;
                    };
                    let local_abs = state.cwd.join(&local.path);
                    let gist_id = gist.file.gist_id.clone();
                    let filename = gist.file.filename.clone();
                    let idx = state.pinned.iter().position(|m| {
                        pin_local_abs(&state, m) == local_abs
                            && m.gist_id == gist_id
                            && m.gist_filename == filename
                    });
                    let Some(idx) = idx else {
                        state.set_status("pair is not pinned — press p to pin first");
                        continue;
                    };
                    let m = state.pinned[idx].clone();
                    match state.pin_sync_status(idx) {
                        crate::domain::SyncStatus::Push => {
                            spawn_pin_push(&mut state, &mut bg_rx, &m)
                        }
                        crate::domain::SyncStatus::Pull => {
                            spawn_pin_pull(&mut state, &mut bg_rx, &m)
                        }
                        crate::domain::SyncStatus::InSync => state.set_status("already in sync"),
                        crate::domain::SyncStatus::Unknown => state.set_status(
                            "can't tell which side is newer — use u to push or d to pull",
                        ),
                    }
                }
                KeyOutcome::SyncPinPush => {
                    if let Some(m) = selected_pin(&state) {
                        spawn_pin_push(&mut state, &mut bg_rx, &m);
                    }
                }
                KeyOutcome::SyncPinPull => {
                    if let Some(m) = selected_pin(&state) {
                        spawn_pin_pull(&mut state, &mut bg_rx, &m);
                    }
                }
                KeyOutcome::SyncPinAuto => {
                    let Some(m) = selected_pin(&state) else {
                        continue;
                    };
                    match state.pin_sync_status(state.pins_index) {
                        crate::domain::SyncStatus::InSync => state.set_status("already in sync"),
                        crate::domain::SyncStatus::Pull => {
                            spawn_pin_pull(&mut state, &mut bg_rx, &m)
                        }
                        crate::domain::SyncStatus::Push => {
                            // Cheap, network-free no-op check: if the local file still
                            // hashes to last_seen_hash, the newer mtime is a touch with
                            // no content change. Note: only fires for plain pushes — a
                            // push whose baseline was a JSON-transformed/redacted upload
                            // won't match the raw file, so it harmlessly falls through to
                            // a full push.
                            let local_abs = pin_local_abs(&state, &m);
                            let unchanged = m.last_seen_hash.as_deref().is_some_and(|baseline| {
                                std::fs::read(&local_abs)
                                    .map(|b| crate::domain::sha256_hex(&b) == baseline)
                                    .unwrap_or(false)
                            });
                            if unchanged {
                                state.set_status("already in sync");
                            } else {
                                spawn_pin_push(&mut state, &mut bg_rx, &m);
                            }
                        }
                        crate::domain::SyncStatus::Unknown => state.set_status(
                            "can't tell which side is newer — use u to push or d to pull",
                        ),
                    }
                }
                KeyOutcome::PreviewPinDiff => {
                    if let Some(m) = selected_pin(&state) {
                        state.diff_return = Screen::Pins;
                        spawn_pin_diff(&mut state, &mut bg_rx, &m);
                    }
                }
                KeyOutcome::PersistDiffContext => persist_diff_context(&mut state),
                KeyOutcome::None => {}
            }
        }
    }

    Ok(())
}

impl AppState {
    /// `(local_ts, remote_ts)` Unix-seconds for `pinned[index]`, from in-memory data.
    pub fn pin_mtimes(&self, index: usize) -> (Option<u64>, Option<u64>) {
        let Some(m) = self.pinned.get(index) else {
            return (None, None);
        };
        let local_abs = if m.local_path.is_absolute() {
            m.local_path.clone()
        } else {
            self.cwd.join(&m.local_path)
        };
        let local_ts = self.locals.iter().find_map(|c| {
            let cabs = if c.path.is_absolute() {
                c.path.clone()
            } else {
                self.cwd.join(&c.path)
            };
            (cabs == local_abs).then_some(c.modified).flatten()
        });
        let remote_ts = self.gists.iter().find_map(|g| {
            (g.gist_id == m.gist_id && g.filename == m.gist_filename)
                .then(|| crate::domain::parse_rfc3339_to_unix(&g.updated_at))
                .flatten()
        });
        (local_ts, remote_ts)
    }

    /// Derive the [`SyncStatus`] for `pinned[index]` from in-memory mtimes.
    /// Local mtime comes from the matching local candidate (if discovered);
    /// remote mtime from the matching gist's `updated_at`.
    pub fn pin_sync_status(&self, index: usize) -> crate::domain::SyncStatus {
        let (local_ts, remote_ts) = self.pin_mtimes(index);
        crate::domain::sync_status(local_ts, remote_ts)
    }
}

/// Draw a centered, bordered box over the current frame, sized to fit `body` (clamped to
/// the frame) and wiped clean with `Clear` so whatever is behind it doesn't bleed through.
/// This is the shared "centered window" primitive behind both the loading overlay and the
/// confirm prompt.
fn render_centered_modal(frame: &mut Frame, title: &str, body: &str, border: Color) {
    let area = frame.area();
    let max_width = area.width.saturating_sub(2).max(1);
    let width = ((area.width as u32 * 60 / 100) as u16).clamp(max_width.min(20), max_width);
    // Inner text width = box width minus the two border columns and the horizontal padding.
    let inner_width = width.saturating_sub(4);
    let body_lines = wrap_line_count(body, inner_width).max(1);
    let max_height = area.height.saturating_sub(2).max(1);
    let height = (body_lines + 2).clamp(max_height.min(3), max_height);
    let rect = Rect {
        x: area.width.saturating_sub(width) / 2,
        y: area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(body.to_string())
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .title(title.to_string())
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(border))
                    .padding(Padding::horizontal(1)),
            ),
        rect,
    );
}

/// A centered "Working…" box shown while a blocking `gh` action runs.
fn render_loading_overlay(frame: &mut Frame, msg: &str) {
    render_centered_modal(frame, "Working…", &format!("⏳ {msg}"), Color::Cyan);
}

/// Civil date (year, month, day) from a day count since the Unix epoch — Howard Hinnant's
/// algorithm. UTC, leap-second agnostic (fine for display).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn format_unix_utc(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86400));
    let rem = secs.rem_euclid(86400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

fn file_mtime_label(path: &std::path::Path) -> String {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| format_unix_utc(d.as_secs() as i64))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Normalises the gist API's RFC3339 `updated_at` (e.g. `2026-06-08T11:06:18Z`) to
/// `2026-06-08 11:06 UTC` for display alongside the local file's mtime.
fn gist_time_label(updated_at: &str) -> String {
    if updated_at.is_empty() {
        "unknown".to_string()
    } else if updated_at.len() >= 16 {
        format!("{} UTC", updated_at[..16].replace('T', " "))
    } else {
        updated_at.to_string()
    }
}

// ---------------------------------------------------------------------------
// Pinned-sync helpers (Task 9 + Task 10)
// ---------------------------------------------------------------------------

type BgRx = Option<std::sync::mpsc::Receiver<BgTaskOutcome>>;

/// The pin currently selected in the Pins screen, if any.
fn selected_pin(state: &AppState) -> Option<crate::domain::PinnedMapping> {
    state.pinned.get(state.pins_index).cloned()
}

/// Resolve a pin's absolute local path against cwd.
fn pin_local_abs(state: &AppState, m: &crate::domain::PinnedMapping) -> PathBuf {
    if m.local_path.is_absolute() {
        m.local_path.clone()
    } else {
        state.cwd.join(&m.local_path)
    }
}

/// Spawn the push (upload local → gist) flow for a pin: lands in the existing
/// upload `Screen::Confirm` diff.
fn spawn_pin_push(state: &mut AppState, bg_rx: &mut BgRx, m: &crate::domain::PinnedMapping) {
    let local_path = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: gist_id.clone(),
        filename: filename.clone(),
        local_path: local_path.clone(),
    });
    let gist_file = GistFile {
        gist_id: gist_id.clone(),
        description: String::new(),
        filename: filename.clone(),
        public: false,
        updated_at: String::new(),
        created_at: String::new(),
    };
    let (local_label, gist_label) = diff_labels(Some(&local_path), &gist_file);
    state.bg_task_msg = Some("Loading diff…".to_string());
    let (tx, rx) = std::sync::mpsc::channel();
    *bg_rx = Some(rx);
    std::thread::spawn(move || {
        let result =
            crate::gh::fetch_gist_file_content(&gist_id, &filename).map_err(|e| e.to_string());
        let _ = tx.send(BgTaskOutcome::UploadPreview {
            result,
            gist_id,
            filename,
            local_path,
            local_label,
            gist_label,
        });
    });
}

/// Spawn the pull (download gist → local) flow for a pin: lands in the existing
/// download `Screen::Confirm` diff when the local file exists.
fn spawn_pin_pull(state: &mut AppState, bg_rx: &mut BgRx, m: &crate::domain::PinnedMapping) {
    let target = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    let gist_file = GistFile {
        gist_id: gist_id.clone(),
        description: String::new(),
        filename: filename.clone(),
        public: false,
        updated_at: String::new(),
        created_at: String::new(),
    };
    let (local_label, gist_label) = diff_labels(Some(&target), &gist_file);
    state.bg_task_msg = Some("Downloading…".to_string());
    let (tx, rx) = std::sync::mpsc::channel();
    *bg_rx = Some(rx);
    std::thread::spawn(move || {
        let result =
            crate::gh::fetch_gist_file_content(&gist_id, &filename).map_err(|e| e.to_string());
        let _ = tx.send(BgTaskOutcome::DownloadSelected {
            result,
            target,
            local_label,
            gist_label,
            gist_id,
            filename,
        });
    });
}

/// Spawn a read-only diff (gist vs local) for a pin, landing on `Screen::Diff`.
fn spawn_pin_diff(state: &mut AppState, bg_rx: &mut BgRx, m: &crate::domain::PinnedMapping) {
    let local_abs = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    let gist_file = GistFile {
        gist_id: gist_id.clone(),
        description: String::new(),
        filename: filename.clone(),
        public: false,
        updated_at: String::new(),
        created_at: String::new(),
    };
    let (local_label, gist_label) = diff_labels(Some(&local_abs), &gist_file);
    state.bg_task_msg = Some("Loading diff…".to_string());
    let (tx, rx) = std::sync::mpsc::channel();
    *bg_rx = Some(rx);
    let target = local_abs.clone();
    std::thread::spawn(move || {
        let result =
            crate::gh::fetch_gist_file_content(&gist_id, &filename).map_err(|e| e.to_string());
        let _ = tx.send(BgTaskOutcome::PreviewDiff {
            result,
            local_path: Some(local_abs),
            local_label,
            gist_label,
            target,
            // Pin diffs originate from the Pins screen (no focused pane); keep the
            // historical download orientation (old = local, new = gist).
            upload_orientation: false,
        });
    });
}

/// If `(local_abs, gist_id, filename)` is a pinned pair, record the sync result
/// (hash of `content` + `direction`) to config and update `state.pinned`.
fn record_pin_sync(
    state: &mut AppState,
    local_abs: &std::path::Path,
    gist_id: &str,
    filename: &str,
    content: &str,
    direction: crate::domain::SyncDirection,
) {
    // Find the pin using its STORED (possibly relative) local_path form.
    let stored_local = state.pinned.iter().find_map(|m| {
        let mabs = pin_local_abs(state, m);
        (mabs == local_abs && m.gist_id == gist_id && m.gist_filename == filename)
            .then(|| m.local_path.clone())
    });
    let Some(stored_local) = stored_local else {
        return;
    };
    let hash = crate::domain::sha256_hex(content.as_bytes());
    if let Ok(path) = crate::config::config_path() {
        if let Ok(config) = crate::config::load_config(&path) {
            if let Ok(updated) = crate::actions::record_sync(
                &path,
                config,
                &stored_local,
                gist_id,
                filename,
                &hash,
                direction,
            ) {
                state.pinned = updated.pinned;
            }
        }
    }
}

/// Builds the `--- local` / `+++ gist` diff header labels showing each side's filename and
/// last-modified time, plus the gist's id.
fn diff_labels(local_path: Option<&std::path::Path>, gist: &GistFile) -> (String, String) {
    let local_name = local_path
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("(none)");
    let local_time = local_path
        .map(file_mtime_label)
        .unwrap_or_else(|| "—".to_string());
    let local_label = format!("local: {local_name} ({local_time})");
    let gist_label = format!(
        "gist {} / {} ({})",
        gist.gist_id,
        gist.filename,
        gist_time_label(&gist.updated_at)
    );
    (local_label, gist_label)
}

/// Orientation for the `Enter` diff preview, driven by the focused pane: focusing the gist
/// pane frames it as a *download* (old = local, new = gist), focusing the local pane frames
/// it as an *upload* (old = gist, new = local). The dedicated `d`/`u` actions keep their own
/// fixed orientation; this only affects the read-only preview.
fn preview_diff_text(
    upload_orientation: bool,
    local_label: &str,
    local_content: &str,
    gist_label: &str,
    remote: &str,
) -> String {
    if upload_orientation {
        crate::diff::unified_diff(gist_label, remote, local_label, local_content)
    } else {
        crate::diff::unified_diff(local_label, local_content, gist_label, remote)
    }
}

fn open_browser(state: &mut AppState) {
    let gist_id = state.context_gist_id();
    let Some(gist_id) = gist_id else {
        return;
    };
    let plan = crate::actions::open_browser_command(&gist_id);
    match crate::actions::execute_command(&plan) {
        Ok(_) => state.set_status(format!("Opened gist {gist_id} in the browser")),
        Err(error) => state.set_status(format!("open failed: {error}")),
    }
}

/// Opens the selected local file in `$VISUAL`/`$EDITOR` (default `vi`). A terminal editor
/// needs the full terminal, so the TUI leaves raw mode / the alternate screen for the
/// duration and restores afterwards. `$EDITOR` may include flags (e.g. `code --wait`).
fn edit_local(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
) -> Result<()> {
    let Some(local) = state.selected_local() else {
        return Ok(());
    };
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let Some(program) = parts.next() else {
        state.set_status("no editor configured (set $EDITOR)");
        return Ok(());
    };
    let args: Vec<&str> = parts.collect();

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    let result = std::process::Command::new(program)
        .args(&args)
        .arg(&local.path)
        .status();
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.clear()?;

    match result {
        Ok(_) => state.set_status(format!("Edited {}", local.path.display())),
        Err(error) => state.set_status(format!("editor failed: {error}")),
    }
    Ok(())
}

fn edit_upload_buffer(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
) -> Result<()> {
    let Some(local_path) = state.upload_local_path() else {
        return Ok(());
    };
    let Some(filename) = local_path.file_name().and_then(|n| n.to_str()) else {
        return Ok(());
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_file_path = state
        .cwd
        .join(format!(".gistui_redact_{timestamp}_{filename}"));

    let current_content = state.content_to_upload();
    if let Err(e) = std::fs::write(&temp_file_path, &current_content) {
        state.set_status(format!("failed to write temp file: {e}"));
        return Ok(());
    }

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let Some(program) = parts.next() else {
        state.set_status("no editor configured (set $EDITOR)");
        let _ = std::fs::remove_file(&temp_file_path);
        return Ok(());
    };
    let args: Vec<&str> = parts.collect();

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    let result = std::process::Command::new(program)
        .args(&args)
        .arg(&temp_file_path)
        .status();
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.clear()?;

    match result {
        Ok(_) => match std::fs::read_to_string(&temp_file_path) {
            Ok(edited_content) => {
                state.upload_edited_content = Some(edited_content);
                state.update_upload_diff();
                state.set_status("Edited redact buffer");
            }
            Err(e) => state.set_status(format!("failed to read edited file: {e}")),
        },
        Err(error) => state.set_status(format!("editor failed: {error}")),
    }

    let _ = std::fs::remove_file(&temp_file_path);
    Ok(())
}

fn download(state: &mut AppState) {
    let target = state.download_target.clone();
    let content = state.preview_remote.clone();
    match crate::actions::execute_download(&target, &content, true) {
        Ok(()) => {
            state.set_status(format!("Downloaded {}", target.display()));
            if let (Some(gid), Some(fname)) = (
                state.download_gist_id.clone(),
                state.download_gist_filename.clone(),
            ) {
                record_pin_sync(
                    state,
                    &target,
                    &gid,
                    &fname,
                    &content,
                    crate::domain::SyncDirection::Download,
                );
            }
            state.back_to_list();
            refresh_locals(state);
        }
        Err(error) => {
            state.set_status(format!("download failed: {error}"));
            state.screen = Screen::Diff;
        }
    }
}

/// Quick flat re-scan used after a download/upload to make the new file visible immediately.
/// Always non-recursive since downloads only write to cwd root.
fn refresh_locals(state: &mut AppState) {
    let selected = state.selected_local().map(|c| c.path.clone());
    if let Ok(locals) = crate::local::discover_local_candidates(
        &state.cwd,
        &state.pinned,
        false,
        &state.skip_dirs,
        state.scan_depth,
    ) {
        state.locals = locals;
        state.local_index = selected
            .and_then(|path| state.locals.iter().position(|c| c.path == path))
            .unwrap_or(0)
            .min(state.locals.len().saturating_sub(1));
        if state.gist_index >= state.ranked_gists().len() {
            state.gist_index = 0;
        }
    }
}

/// Persist the diff-context toggle (`diff_show_full`) to the config file, leaving the
/// configured `diff_context` radius untouched. IO boundary, called from `run_loop`.
fn persist_diff_context(state: &mut AppState) {
    let result = crate::config::config_path().and_then(|path| {
        let mut config = crate::config::load_config(&path)?;
        config.diff_show_full = state.diff_show_full;
        crate::config::save_config(&path, &config)?;
        Ok(())
    });
    match result {
        Ok(()) if state.diff_show_full => state.set_status("Diff context: full file"),
        Ok(()) => state.set_status(format!("Diff context: {} lines", state.diff_context)),
        Err(error) => state.set_status(format!("save config failed: {error}")),
    }
}

fn pin_selected(state: &mut AppState) {
    let (Some(local), Some(gist)) = (state.selected_local(), state.selected_gist()) else {
        return;
    };
    let result = crate::config::config_path().and_then(|path| {
        let config = crate::config::load_config(&path)?;
        crate::actions::pin_mapping(&path, config, &local.path, &gist.file, None, None)
    });
    match result {
        Ok(config) => {
            state.pinned = config.pinned;
            state.skip_dirs = config.skip_dirs;
            state.scan_depth = config.scan_depth;
            state.set_status(format!(
                "Pinned {} <-> {}",
                local.path.display(),
                gist.file.filename
            ));
        }
        Err(error) => state.set_status(format!("pin failed: {error}")),
    }
}

fn unpin_selected(state: &mut AppState) {
    let Some(local) = state.selected_local() else {
        return;
    };
    let result = crate::config::config_path().and_then(|path| {
        let config = crate::config::load_config(&path)?;
        crate::actions::unpin_mapping(&path, config, &local.path)
    });
    match result {
        Ok(config) => {
            state.pinned = config.pinned;
            state.skip_dirs = config.skip_dirs;
            state.scan_depth = config.scan_depth;
            state.set_status(format!("Unpinned {}", local.path.display()));
        }
        Err(error) => state.set_status(format!("unpin failed: {error}")),
    }
}

fn unpin_at_pin_index(state: &mut AppState) {
    let Some(mapping) = state.pinned.get(state.pins_index).cloned() else {
        return;
    };
    let label = mapping.local_path.display().to_string();
    let result = crate::config::config_path().and_then(|path| {
        let config = crate::config::load_config(&path)?;
        crate::actions::unpin_mapping_exact(&path, config, &mapping.local_path, &mapping.gist_id)
    });
    match result {
        Ok(config) => {
            state.pinned = config.pinned;
            state.skip_dirs = config.skip_dirs;
            state.scan_depth = config.scan_depth;
            state.pins_index = state.pins_index.min(state.pinned.len().saturating_sub(1));
            refresh_locals(state);
            state.set_status(format!("Unpinned {label}"));
        }
        Err(error) => state.set_status(format!("unpin failed: {error}")),
    }
}

fn upload_local_filename(local: &std::path::Path) -> Option<String> {
    local.file_name().and_then(|n| n.to_str()).map(String::from)
}

mod render;
use render::*;

#[cfg(test)]
mod tests;
