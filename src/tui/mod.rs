use crate::domain::{group_gists, GistComment, GistFile, GistGroup, LocalCandidate, PinnedMapping};
use crate::ranking::{rank_gist_files, rank_local_files, MatchReason, RankedGistFile, RankedLocal};
use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, widgets::Clear, Terminal};
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

/// Which tab `Screen::GistDetail` shows, and which the navigation keys drive: the file list
/// or the comments (only one is visible at a time). Defaults to `Files` — the gist's primary
/// content — with the comments one `Tab` away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailFocus {
    Comments,
    Files,
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

/// Generates a small enum whose variants cycle in declaration order. `next()` advances to the
/// following variant (wrapping past the last) and `label()` returns each variant's short
/// status-footer label. Keeping the variant↔label pairing in one place lets the sort enums
/// share a single definition instead of hand-rolling near-identical `next`/`label` impls.
macro_rules! cycling_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident { $($variant:ident => $label:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        $vis enum $name {
            $($variant),+
        }

        impl $name {
            /// Cycle to the next variant in declaration order, wrapping past the last.
            fn next(self) -> Self {
                const ORDER: &[$name] = &[$($name::$variant),+];
                let i = ORDER.iter().position(|v| *v == self).unwrap_or(0);
                ORDER[(i + 1) % ORDER.len()]
            }

            /// The short status-footer label for this variant.
            fn label(self) -> &'static str {
                match self {
                    $($name::$variant => $label),+
                }
            }
        }
    };
}

cycling_enum! {
    /// Sort order for the ranked gist pane. `Match` keeps the incoming order (ranking score,
    /// or the gh list order when no local is selected); the others override it.
    pub enum GistSort {
        Match => "match",
        Name => "name",
        Recent => "recent",
    }
}

cycling_enum! {
    /// Sort order for the gist-level view (`Screen::Gists`). The `gh` list already
    /// arrives updated-first, so `Updated` mirrors that; `Created` re-sorts by age.
    pub enum GistGroupSort {
        Updated => "updated",
        Created => "created",
    }
}

cycling_enum! {
    /// Sort order for the local file pane. Mirrors [`GistSort`]: `Match` keeps the
    /// incoming order (reverse-ranking score when the gist pane drives, else discovery
    /// order); the others override it.
    pub enum LocalSort {
        Match => "match",
        Name => "name",
        Recent => "recent",
    }
}

impl GistSort {
    /// Re-orders ranked gists. `Match` keeps the incoming order; the others override it.
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

impl LocalSort {
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
    /// Copy the context gist's web URL to the system clipboard (`y`).
    CopyGistUrl,
    /// Copy the previewed file content to the system clipboard (`Y`, Preview screen).
    CopyPreviewContent,
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
    /// Text filter for the LOCAL pane (List screen). Independent of `filter_query`
    /// (the gist pane), so both panes can be filtered at once. Matched against the
    /// cwd-relative display label, i.e. the exact string shown in the local list.
    pub local_filter_query: String,
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
    /// Soft line-wrapping in the full-screen preview, toggled with `w` (remembered for the
    /// session). When on, long lines wrap instead of needing horizontal scroll.
    pub preview_wrap: bool,
    /// Syntax-highlight file content in the preview and diff-context lines (issue #69).
    /// Defaults on; `load_startup_state` turns it off when `NO_COLOR` is set in the environment.
    pub syntax_highlight: bool,
    pub preview_gist_key: Option<(String, String)>,
    /// Screen to return to when leaving the full-screen preview (default: List; set to
    /// GistDetail when a detail-view file preview is launched).
    pub preview_return: Screen,
    /// A `(gist_id, filename)` explicitly chosen for preview (e.g. a number key in the detail
    /// view), taken by the `PreviewContent` IO step; when `None` it falls back to the selected
    /// gist file on the list. Keeps `handle_key` pure: it records the intent, `run_loop` fetches.
    pub preview_request: Option<(String, String)>,
    pub gist_content_cache: crate::lru::LruCache<(String, String), String>,
    pub local_recursive: bool,
    pub skip_dirs: Vec<String>,
    pub scan_depth: u32,
    pub local_scanning: bool,
    pub pins_index: usize,
    pub pins_hscroll: u16,
    pub pins_filtering: bool,
    pub pins_filter_query: String,
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
    /// Which detail-view pane Tab/arrows currently drive (Comments vs Files).
    pub detail_focus: DetailFocus,
    /// Cursor index into the detail gist's files when `detail_focus == Files`.
    pub detail_file_cursor: usize,
    /// Screen to return to after a compaction confirm is cancelled/finished (Gists or GistDetail).
    pub compact_return_screen: Screen,
    /// Monotonic tick advanced once per event-loop iteration (~150ms); drives the in-progress
    /// spinner animation. Wraps freely — only its value modulo the frame count is observed.
    pub spinner_frame: usize,
    /// Per-gist comment counts (`gist_id` → count) from the gist-list fetch, surfaced in the
    /// gist manager rows. Kept off `GistFile` since the count is a gist-level value, not a
    /// per-file one; empty until the first live fetch lands (cached startup gists show 0).
    pub gist_comment_counts: std::collections::HashMap<String, u32>,
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
        let query = self.local_filter_query.to_lowercase();
        if !query.is_empty() {
            ranked.retain(|r| {
                render::local_row_label(&r.candidate.path, &self.cwd)
                    .to_lowercase()
                    .contains(&query)
            });
        }
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
                gist_group_row_label(
                    g,
                    unix_now(),
                    self.gists_sort,
                    self.gist_comment_counts.get(&g.id).copied().unwrap_or(0),
                )
                .chars()
                .count()
            })
            .max()
            .unwrap_or(0)
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16
    }

    /// Highest horizontal-scroll offset for the Pins screen, bounded by the longest
    /// displayed local path (the only variable-length, overflow-prone field in a pin row).
    /// Pure helper modeled on `gists_hscroll_max`.
    fn pins_hscroll_max(&self) -> u16 {
        self.pinned
            .iter()
            .map(|m| crate::config::display_path(&m.local_path).chars().count())
            .max()
            .unwrap_or(0)
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16
    }

    /// Indices into `self.pinned` that match the Pins-screen text filter, in original
    /// order. Empty query → every index. Matched against the cwd/home-shortened local
    /// path plus the gist filename (the meaningful, visible parts of the row).
    pub fn visible_pin_indices(&self) -> Vec<usize> {
        let query = self.pins_filter_query.to_lowercase();
        self.pinned
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                if query.is_empty() {
                    return true;
                }
                let hay = format!(
                    "{} {}",
                    crate::config::display_path(&m.local_path),
                    m.gist_filename
                )
                .to_lowercase();
                hay.contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The true `self.pinned` index of the currently selected Pins row (selection is a
    /// position within the filtered view).
    pub fn selected_pin_index(&self) -> Option<usize> {
        self.visible_pin_indices().get(self.pins_index).copied()
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

    /// Page the diff/preview down by `lines`, clamped to the same bottom as `scroll_diff_down`.
    pub fn scroll_diff_page_down(&mut self, lines: u16) {
        let max = self
            .diff_text
            .lines()
            .count()
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16;
        self.diff_scroll = self.diff_scroll.saturating_add(lines).min(max);
    }

    /// Page the diff/preview up by `lines`, saturating at the top.
    pub fn scroll_diff_page_up(&mut self, lines: u16) {
        self.diff_scroll = self.diff_scroll.saturating_sub(lines);
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
        local_filter_query: String::new(),
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
        preview_wrap: false,
        syntax_highlight: true,
        preview_gist_key: None,
        preview_return: Screen::List,
        preview_request: None,
        // Bound the in-memory preview cache so browsing many/large gists can't grow unbounded;
        // evicted entries are simply re-fetched on demand.
        gist_content_cache: crate::lru::LruCache::new(64),
        local_recursive: false,
        skip_dirs: crate::config::AppConfig::default().skip_dirs,
        scan_depth: crate::config::AppConfig::default().scan_depth,
        local_scanning: false,
        pins_index: 0,
        pins_hscroll: 0,
        pins_filtering: false,
        pins_filter_query: String::new(),
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
        detail_focus: DetailFocus::Files,
        detail_file_cursor: 0,
        compact_return_screen: Screen::Gists,
        spinner_frame: 0,
        gist_comment_counts: std::collections::HashMap::new(),
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
    // Honour NO_COLOR for the syntax-highlight feature only (existing semantic colours stay).
    state.syntax_highlight = std::env::var_os("NO_COLOR").is_none();
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
mod highlight;
mod render;
use render::*;
mod keys;
mod run_loop;
use run_loop::run_loop;

#[cfg(test)]
mod tests;
