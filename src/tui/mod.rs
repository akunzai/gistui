use crate::domain::{
    group_gists, GistComment, GistFile, GistGroup, GistRevision, LocalCandidate, PinnedMapping,
};
use crate::ranking::{rank_gist_files, rank_local_files, MatchReason, RankedGistFile, RankedLocal};
use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, layout::Rect, widgets::Clear, Terminal};
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    Local,
    Gist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    List,
    Diff,
    Confirm,
    Preview,
    Help,
    Pins,
    Gists,
    /// Single-gist detail: basic info + file list + comments (entered from Gists with Enter).
    GistDetail,
    /// Revision history for one gist (entered with `H` from the list, Gist manager, or Gist detail).
    Revisions,
    /// Unified context menu / command palette overlay (`;` or right-click / `Ctrl+p`).
    Palette,
    /// Flat settings list (issue #227); opened with `C` or the command palette.
    Config,
}

/// Fields shown on [`Screen::Config`] in order (issue #227).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    Theme,
    Mouse,
    CheckUpdates,
    IgnoreTrailingNewline,
    ScanDepth,
    DiffContext,
}

impl ConfigField {
    pub const ALL: [ConfigField; 6] = [
        ConfigField::Theme,
        ConfigField::Mouse,
        ConfigField::CheckUpdates,
        ConfigField::IgnoreTrailingNewline,
        ConfigField::ScanDepth,
        ConfigField::DiffContext,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConfigField::Theme => "Theme",
            ConfigField::Mouse => "Mouse support",
            ConfigField::CheckUpdates => "Check for updates",
            ConfigField::IgnoreTrailingNewline => "Ignore trailing newline",
            ConfigField::ScanDepth => "Recursive scan depth",
            ConfigField::DiffContext => "Diff context lines",
        }
    }

    pub fn is_numeric(self) -> bool {
        matches!(self, ConfigField::ScanDepth | ConfigField::DiffContext)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ConfigState {
    pub index: usize,
    pub return_screen: Screen,
}

/// A help topic — one per key-dense area, plus `About` (version/repo/update info, not tied
/// to a screen). Ordered for the index list and `1`-`9`,`0` quick-jump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpTopic {
    #[default]
    List,
    Pins,
    GistManager,
    GistDetail,
    Diff,
    Preview,
    Upload,
    Revisions,
    General,
    About,
    Config,
}

impl HelpTopic {
    /// All topics in index / quick-jump order.
    pub fn all() -> [HelpTopic; 11] {
        use HelpTopic::*;
        [
            List,
            Pins,
            GistManager,
            GistDetail,
            Revisions,
            Diff,
            Preview,
            Upload,
            Config,
            General,
            About,
        ]
    }

    /// Short title shown in the index and the topic-view block title.
    pub fn title(self) -> &'static str {
        match self {
            HelpTopic::List => "List screen",
            HelpTopic::Pins => "Pinned Mappings",
            HelpTopic::GistManager => "Gist manager",
            HelpTopic::GistDetail => "Gist detail",
            HelpTopic::Revisions => "Revision history",
            HelpTopic::Diff => "Diff view",
            HelpTopic::Preview => "Preview",
            HelpTopic::Upload => "Upload confirmation",
            HelpTopic::Config => "Settings",
            HelpTopic::General => "General",
            HelpTopic::About => "About",
        }
    }

    /// The topic to open when `?` is pressed on a given screen. Non-key-dense screens
    /// fall back to the List topic.
    pub fn for_screen(screen: Screen) -> HelpTopic {
        match screen {
            Screen::Pins => HelpTopic::Pins,
            Screen::Gists => HelpTopic::GistManager,
            Screen::GistDetail => HelpTopic::GistDetail,
            Screen::Revisions => HelpTopic::Revisions,
            Screen::Config => HelpTopic::Config,
            _ => HelpTopic::List,
        }
    }
}

/// Which tab `Screen::GistDetail` shows, and which the navigation keys drive: the file list
/// or the comments (only one is visible at a time). Defaults to `Files` — the gist's primary
/// content — with the comments one `Tab` away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailFocus {
    Comments,
    #[default]
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
    RestoreRevision {
        gist_id: String,
        filename: String,
        version: String,
        version_label: String,
        content: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GistView {
    Description,
    Id,
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

impl Default for GistGroupSort {
    /// `Updated` mirrors the gh list's default updated-first order.
    fn default() -> Self {
        GistGroupSort::Updated
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

cycling_enum! {
    /// Sort order for the Pins screen. `Default` keeps config/insertion order; the
    /// others sort the visible rows by the named field.
    pub enum PinSort {
        Default => "default",
        Local => "local",
        Gist => "gist",
    }
}

impl Default for PinSort {
    /// The `Default` variant (config/insertion order) is the natural default.
    fn default() -> Self {
        PinSort::Default
    }
}

cycling_enum! {
    /// Visibility/type filter for the gist panes, cycled with `v`. `next`/`label` come from
    /// the macro; the filtering helpers live in a separate `impl` block below.
    pub enum GistTypeFilter {
        All => "all",
        Public => "public",
        Secret => "secret",
        Starred => "starred",
        Forked => "forked",
    }
}

impl Default for GistTypeFilter {
    /// `All` (no filtering) is the natural default.
    fn default() -> Self {
        GistTypeFilter::All
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
    pub fn uses_starred_source(self) -> bool {
        self == GistTypeFilter::Starred
    }

    pub fn matches_file(self, file: &GistFile) -> bool {
        match self {
            GistTypeFilter::All | GistTypeFilter::Starred => true,
            GistTypeFilter::Public => file.public,
            GistTypeFilter::Secret => !file.public,
            GistTypeFilter::Forked => file.is_fork(),
        }
    }

    pub fn matches_group(self, group: &GistGroup) -> bool {
        match self {
            GistTypeFilter::All | GistTypeFilter::Starred => true,
            GistTypeFilter::Public => group.public,
            GistTypeFilter::Secret => !group.public,
            GistTypeFilter::Forked => group.fork_of_id.is_some(),
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
    /// Open the selected gist's detail screen (comments load when the Comments tab is opened).
    OpenGistDetail,
    /// Fetch comments for the gist shown on `Screen::GistDetail` (lazy, on Comments tab).
    FetchComments,
    LoadOlderComments,
    /// Analyse the selected Gist-manager gist's revision count, then ask to confirm a compaction.
    CompactGist,
    /// Run the confirmed compaction (clone → squash → force-push) on the pending gist.
    ExecuteCompactGist,
    ApplyDescription,
    RefreshLocals,
    OpenRepoUrl,
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
    /// Toggle the colour theme between dark and light and persist to config (`T`, global).
    ThemeToggle,
    /// Persist settings changed on [`Screen::Config`] (creates config.toml only after a change).
    PersistSettings,
    /// Fetch the revision list for the gist opened on `Screen::Revisions`.
    FetchRevisions,
    /// Diff the target file: parent revision → selected revision (incremental).
    RevisionDiffIncremental,
    /// Diff the target file: selected revision vs current head.
    RevisionDiff,
    /// Fetch revision + head content and stage a restore confirm.
    RestoreRevisionPreview,
    /// Apply a confirmed single-file revision restore (`PATCH`).
    ExecuteRestoreRevision,
    /// Star or unstar the context gist (`*`).
    ToggleGistStar,
    /// Fork the context gist into the authenticated account (`F`).
    ForkGist,
}

/// A clickable list pane recorded by `render` for the current frame.
/// `offset` is ratatui's first-visible-item index, captured after the list renders.
#[derive(Debug, Clone, Copy)]
pub struct PaneHit {
    pub rect: Rect,
    pub offset: usize,
}

impl PaneHit {
    /// Map an absolute terminal `row` to a list index, or `None` for border rows,
    /// rows past the last item, or an empty list. `visible_len` is the count of
    /// currently visible rows (e.g. `visible_locals().len()` / `ranked_gists().len()`).
    pub fn index_at(&self, row: u16, visible_len: usize) -> Option<usize> {
        let top = self.rect.y + 1; // skip the top border
        let bottom = self.rect.bottom().saturating_sub(1); // exclusive of bottom border
        if row < top || row >= bottom {
            return None;
        }
        let idx = self.offset + (row - top) as usize;
        (idx < visible_len).then_some(idx)
    }
}

/// Per-frame mouse hit regions, owned by `run_loop`, filled by `render`.
#[derive(Debug, Default, Clone)]
pub struct MouseLayout {
    pub local: Option<PaneHit>,
    pub gist: Option<PaneHit>,
    /// Single-list screens (Gists / Pins / Revisions) and the Help topic index.
    pub list: Option<PaneHit>,
    /// GistDetail file list (Files tab).
    pub detail_files: Option<PaneHit>,
    /// GistDetail "Files" / "Comments" tab headers (clickable to switch focus).
    pub detail_tab_files: Option<Rect>,
    pub detail_tab_comments: Option<Rect>,
    pub close_button: Option<Rect>,
    /// GistDetail Comments: the clickable "load older" affordance line.
    pub comments_load_older: Option<Rect>,
    /// GistDetail Comments: max useful vertical scroll (set by render; used by run_loop
    /// to honour a one-shot scroll-to-bottom after the newest page loads).
    pub comments_max_scroll: Option<u16>,
    pub repo_link: Option<Rect>,
    /// Cross-screen top-bar shortcut hit-rects — `(G)ists`, `(P)ins`, `(C)onfig`, `(?)Help`.
    /// Set by `render_top_bar` on every screen except the transient `Confirm` y/n modal.
    pub top_bar_gists: Option<Rect>,
    pub top_bar_pins: Option<Rect>,
    pub top_bar_config: Option<Rect>,
    pub top_bar_help: Option<Rect>,
    /// Palette overlay: one hit-rect per visible row, plus the `[✕]` close button.
    pub palette_rows: Vec<Rect>,
    pub palette_close: Option<Rect>,
}

/// A classified mouse intent handed to the pure `handle_mouse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseInput {
    ScrollUp,
    ScrollDown,
    Click { col: u16, row: u16 },
    DoubleClick { col: u16, row: u16 },
    RightClick { col: u16, row: u16 },
}

/// Max gap between two left-clicks on the same cell to count as a double-click.
pub const DOUBLE_CLICK_MS: u128 = 400;

/// Classify a left-button press as a single or double click. `prev` is the (col,row) of
/// the previous left press; `elapsed_ms` is the time since it. Pure: the caller (run_loop)
/// owns the clock and supplies the elapsed milliseconds.
pub fn classify_click(
    prev: Option<(u16, u16)>,
    elapsed_ms: u128,
    col: u16,
    row: u16,
) -> MouseInput {
    if prev == Some((col, row)) && elapsed_ms <= DOUBLE_CLICK_MS {
        MouseInput::DoubleClick { col, row }
    } else {
        MouseInput::Click { col, row }
    }
}

/// Per-screen upload-diff state (the `u` flow). Data only — the upload methods
/// (`init_upload_state`, `content_to_upload`, `update_upload_diff`) stay on `AppState`.
#[derive(Debug, Clone, Default)]
pub struct UploadState {
    pub original_content: String,
    pub edited_content: Option<String>,
    pub json_pretty: bool,
    pub json_sort: bool,
    pub remote_content: Option<String>,
    pub local_label: Option<String>,
    pub gist_label: Option<String>,
    /// True while a GUI-editor background watch (see `run_loop::spawn_upload_edit_watch`) is
    /// live-updating the diff. Gates `y`/`e` in `handle_key_confirm` — the upload can't be
    /// confirmed, and a second editor instance can't be spawned, until the editor closes.
    pub watching: bool,
}

/// Per-screen revision-history state (`Screen::Revisions`). Data only — the revision
/// methods stay on `AppState`.
#[derive(Debug, Clone, Default)]
pub struct RevisionState {
    /// Gist whose revisions are shown.
    pub gist_id: Option<String>,
    /// Fetched revision rows (`None` while the initial list fetch is in flight).
    pub entries: Option<Vec<GistRevision>>,
    /// Cursor into `entries` (0 = current head).
    pub index: usize,
    pub hscroll: u16,
    /// File within the gist that preview/diff/restore target.
    pub target_file: String,
    /// Where `q`/`Esc` returns from `Screen::Revisions`.
    pub return_screen: Screen,
    /// Error from the commits-list fetch, if any.
    pub fetch_error: Option<String>,
}

/// Per-screen Help-view state (`Screen::Help`). Data only — the help methods stay on `AppState`.
#[derive(Debug, Clone, Default)]
pub struct HelpState {
    pub scroll: u16,
    /// Screen to return to when leaving Help (mirrors `preview_return` / `diff_return`).
    pub return_screen: Screen,
    /// The topic shown in the Help screen's topic view.
    pub topic: HelpTopic,
    /// When true the Help screen shows the topic index instead of a topic body.
    pub index_open: bool,
    /// Highlighted row in the Help topic index.
    pub index_sel: usize,
}

/// Pins-screen state (`Screen::Pins`). Data only — the pins methods stay on `AppState`.
#[derive(Debug, Clone, Default)]
pub struct PinsState {
    pub index: usize,
    pub hscroll: u16,
    pub filtering: bool,
    pub filter_query: TextInput,
    pub sort: PinSort,
}

/// Gist-manager screen state (`Screen::Gists`). Named `gist_manager` on `AppState` because
/// the `gists` field name is taken by the gist list `Vec`. Data only — methods stay on `AppState`.
#[derive(Debug, Clone, Default)]
pub struct GistsManagerState {
    pub index: usize,
    pub hscroll: u16,
    pub sort: GistGroupSort,
    pub type_filter: GistTypeFilter,
    pub filtering: bool,
    pub filter_query: TextInput,
}

/// Gist-detail screen state (`Screen::GistDetail`), including its Comments tab. Data only —
/// the detail/comment methods stay on `AppState`. The `comments_*` count/paging fields keep
/// their prefix so they don't collide with the `comments` Vec.
#[derive(Debug, Clone, Default)]
pub struct DetailState {
    /// The gist currently shown; also guards stale comment responses.
    pub gist_id: Option<String>,
    /// Comments: `None` until the Comments tab is opened; `Some` is the fetched result.
    pub comments: Option<Vec<GistComment>>,
    /// True while a comment fetch is in flight (after the user opens the Comments tab).
    pub comments_loading: bool,
    /// Comment-fetch error message, if any.
    pub comments_error: Option<String>,
    /// Exact total comment count (from the per_page=1 probe); for the title only.
    pub comments_total: Option<u32>,
    /// Smallest 1-based page index currently loaded. 0 = none loaded yet.
    pub comments_loaded_oldest_page: u32,
    /// A "load older" request is in flight (distinct from the initial load).
    pub comments_loading_more: bool,
    /// One-shot: run_loop scrolls the comments pane to the bottom on the next draw.
    pub comments_scroll_to_bottom: bool,
    /// Comment-pane scroll offset.
    pub scroll: u16,
    /// Which detail-view pane Tab/arrows currently drive (Comments vs Files).
    pub focus: DetailFocus,
    /// Cursor index into the detail gist's files when `focus == Files`.
    pub file_cursor: usize,
    /// Screen to return to after a compaction confirm is cancelled/finished (Gists or GistDetail).
    pub compact_return_screen: Screen,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub locals: Vec<LocalCandidate>,
    pub gists: Vec<GistFile>,
    /// Starred gists from `GET /gists/starred` (may include others' gists).
    pub starred_gists: Vec<GistFile>,
    pub starred_gist_ids: std::collections::HashSet<String>,
    pub current_user_login: Option<String>,
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
    pub filter_query: TextInput,
    /// Text filter for the LOCAL pane (List screen). Independent of `filter_query`
    /// (the gist pane), so both panes can be filtered at once. Matched against the
    /// cwd-relative display label, i.e. the exact string shown in the local list.
    pub local_filter_query: TextInput,
    pub diff_previewed: bool,
    pub diff_text: String,
    pub diff_scroll: u16,
    pub diff_hscroll: u16,
    /// Soft-wrap long lines in the diff view instead of horizontal scrolling (`w` toggles;
    /// session-scoped, mirrors `preview_wrap`).
    pub diff_wrap: bool,
    pub diff_identical: bool,
    /// Unchanged context lines kept around each change in the diff view (from config).
    pub diff_context: u32,
    /// When true the diff view shows the full file; when false it collapses to
    /// `diff_context` lines. Toggled with `c` and persisted to config.
    pub diff_show_full: bool,
    /// Treat a file-final-newline-only delta as no change in the diff view and the
    /// overwrite-confirm gate (from config; default `true`).
    pub ignore_trailing_newline: bool,
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
    /// Config preference for mouse (before CLI force-off). Edited on [`Screen::Config`].
    pub config_mouse: bool,
    /// Whether mouse capture is active this session (config `mouse` AND-NOT `--no-mouse`).
    /// Gates the `Event::Mouse` branch and the close-button rendering.
    pub mouse_enabled: bool,
    /// CLI `--no-mouse` for the process (re-applied when config mouse toggles).
    pub no_mouse_cli: bool,
    /// Config preference for daily update checks. Edited on [`Screen::Config`].
    pub config_check_updates: bool,
    /// Whether the startup update check runs this session (config `check_updates` AND-NOT
    /// `--no-update-check`).
    pub update_check_enabled: bool,
    /// CLI `--no-update-check` for the process.
    pub no_update_check_cli: bool,
    /// Config screen navigation state.
    pub config: ConfigState,
    /// Newer release version found by the background check, if any (footer hint on the List).
    pub update_available: Option<String>,
    /// How this binary was installed — resolved once at startup so the update hint can show
    /// the right upgrade command without per-frame IO.
    pub install_method: crate::upgrade::InstallMethod,
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
    /// Generation token for the in-flight local scan. Bumped on each
    /// [`Self::begin_local_scan`]; absorb ignores results with a mismatched id so a
    /// slow older scan cannot clobber a newer one (issue #221).
    pub local_scan_generation: u64,
    pub pins: PinsState,
    pub gist_manager: GistsManagerState,
    pub editing_description: bool,
    pub description_input: TextInput,
    pub bg_task_msg: Option<String>,
    /// Generation token for the in-flight `spawn_bg` task. Bumped on spawn and on
    /// cancel; absorb ignores outcomes stamped with an older id (issue #221).
    pub bg_task_generation: u64,
    /// Set after the first `q`/`Esc` on the main list; a second press confirms the quit. Any
    /// other key clears it. Prevents an accidental single-key exit.
    pub quit_armed: bool,
    pub help: HelpState,
    pub upload: UploadState,
    /// gist_id of the active download (set when entering the diff Confirm for a pull).
    pub download_gist_id: Option<String>,
    /// filename of the active download (set when entering the diff Confirm for a pull).
    pub download_gist_filename: Option<String>,
    /// Screen to return to when leaving the diff (default: List; set to Pins for pin diffs).
    pub diff_return: Screen,
    pub detail: DetailState,
    /// Monotonic tick advanced once per event-loop iteration (~150ms); drives the in-progress
    /// spinner animation. Wraps freely — only its value modulo the frame count is observed.
    pub spinner_frame: usize,
    /// Per-gist comment counts (`gist_id` → count) from the gist-list fetch, surfaced in the
    /// gist manager rows. Kept off `GistFile` since the count is a gist-level value, not a
    /// per-file one; empty until the first live fetch lands (cached startup gists show 0).
    pub gist_comment_counts: std::collections::HashMap<String, u32>,
    /// Per-gist fork counts (`gist_id` → how many users forked it), from `/gists/{id}/forks`.
    pub gist_fork_counts: std::collections::HashMap<String, u32>,
    /// Per-gist stargazer counts (`gist_id` → count), from GraphQL `stargazerCount`.
    pub gist_star_counts: std::collections::HashMap<String, u32>,
    /// Active theme selection (persisted to config when toggled with `T`).
    pub theme_choice: crate::config::ThemeChoice,
    /// Resolved colour palette for the current theme choice (from config).
    pub theme: Theme,
    pub revision: RevisionState,
    pub palette: PaletteState,
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
            .upload
            .edited_content
            .as_ref()
            .unwrap_or(&self.upload.original_content);
        if let Some(local_path) = self.upload_local_path() {
            if is_json_file(&local_path) {
                if let Ok(transformed) = crate::domain::transform_json(
                    base,
                    self.upload.json_pretty,
                    self.upload.json_sort,
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
            .upload
            .remote_content
            .as_ref()
            .cloned()
            .unwrap_or_default();
        let local_label = self.upload.local_label.clone().unwrap_or_default();
        let gist_label = self.upload.gist_label.clone().unwrap_or_default();

        let diff = crate::diff::unified_diff(
            &gist_label,
            &remote,
            &local_label,
            &local_content,
            self.ignore_trailing_newline,
        );
        self.diff_text = diff;
    }

    /// Prime the upload-diff state from the local file. Returns the read error instead of
    /// silently defaulting to empty content — an unreadable/deleted/non-UTF-8 file would
    /// otherwise render the whole gist as additions, so the caller must surface it and abort
    /// the upload rather than show a bogus diff.
    pub fn init_upload_state(
        &mut self,
        local_path: &std::path::Path,
        remote_content: Option<String>,
        local_label: String,
        gist_label: String,
    ) -> std::io::Result<()> {
        // Cap before buffering: multi-GB locals must not be read into the upload redact buffer.
        if let Some(remote) = remote_content.as_ref() {
            crate::domain::ensure_text_size(remote.len() as u64)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        }
        self.upload.original_content = crate::domain::read_text_file_capped(local_path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.upload.edited_content = None;
        self.upload.json_pretty = false;
        self.upload.json_sort = false;
        self.upload.remote_content = remote_content;
        self.upload.local_label = Some(local_label);
        self.upload.gist_label = Some(gist_label);
        self.update_upload_diff();
        Ok(())
    }

    /// Mark a new background task as in-flight and return its generation id.
    pub fn begin_bg_task(&mut self) -> u64 {
        self.bg_task_generation = self.bg_task_generation.wrapping_add(1);
        self.bg_task_generation
    }

    /// Invalidate any in-flight background task (Esc cancel). Clears the status overlay
    /// and bumps the generation so a late completion cannot mutate state.
    pub fn invalidate_bg_task(&mut self) {
        self.bg_task_generation = self.bg_task_generation.wrapping_add(1);
        self.bg_task_msg = None;
    }

    /// Whether `generation` is still the current background-task id.
    pub fn is_current_bg_generation(&self, generation: u64) -> bool {
        generation == self.bg_task_generation
    }

    /// Mark a new local scan as in-flight and return its generation id.
    pub fn begin_local_scan(&mut self) -> u64 {
        self.local_scan_generation = self.local_scan_generation.wrapping_add(1);
        self.local_scan_generation
    }

    /// Whether `generation` is still the current local-scan id.
    pub fn is_current_local_scan_generation(&self, generation: u64) -> bool {
        generation == self.local_scan_generation
    }

    /// Apply a completed local scan when `generation` matches. Returns `false` (and leaves
    /// state unchanged) for a stale/superseded result.
    pub fn apply_local_scan_if_current(
        &mut self,
        generation: u64,
        locals: Vec<LocalCandidate>,
    ) -> bool {
        if !self.is_current_local_scan_generation(generation) {
            return false;
        }
        let selected = self.selected_local().map(|c| c.path.clone());
        self.locals = locals;
        self.local_index = selected
            .and_then(|path| self.locals.iter().position(|c| c.path == path))
            .unwrap_or(0)
            .min(self.locals.len().saturating_sub(1));
        if self.gist_index >= self.ranked_gists().len() {
            self.gist_index = 0;
        }
        self.local_scanning = false;
        true
    }

    /// Applies a background upload-edit-watch event (see `bg::UploadEditWatchEvent`) to
    /// upload state. Discarded (no-op) if the Confirm/Upload context has since moved on — the
    /// user left Confirm, a different upload edit session is now in progress, or the current
    /// session isn't actively watching (e.g. the user cancelled with `n`, which stops the
    /// watch flag but does not kill the background thread; that thread's stale events must not
    /// leak into a later, unrelated Confirm session for the same gist/file) — identified by
    /// comparing the event's `gist_id`/`filename` against the current `PendingAction::Upload`
    /// and requiring `self.upload.watching`.
    fn apply_upload_edit_event(&mut self, event: bg::UploadEditWatchEvent) {
        use bg::UploadEditWatchEvent as Ev;
        let (event_gist_id, event_filename) = match &event {
            Ev::ContentChanged {
                gist_id, filename, ..
            }
            | Ev::EditorClosed {
                gist_id, filename, ..
            }
            | Ev::ReadError {
                gist_id, filename, ..
            } => (gist_id.as_str(), filename.as_str()),
        };
        let context_matches = self.screen == Screen::Confirm
            && self.upload.watching
            && matches!(
                &self.pending_action,
                Some(PendingAction::Upload { gist_id, filename, .. })
                    if gist_id == event_gist_id && filename == event_filename
            );
        if !context_matches {
            return;
        }

        match event {
            Ev::ContentChanged { content, .. } => {
                self.upload.edited_content = Some(content);
                self.update_upload_diff();
            }
            Ev::EditorClosed { content, .. } => {
                self.upload.edited_content = Some(content);
                self.update_upload_diff();
                self.upload.watching = false;
                self.set_status("Edited redact buffer");
            }
            Ev::ReadError { message, .. } => {
                self.upload.watching = false;
                self.set_status(format!("failed to read edited file: {message}"));
            }
        }
    }

    fn list_gist_source(&self) -> &[GistFile] {
        if self.gist_type_filter.uses_starred_source() {
            &self.starred_gists
        } else {
            &self.gists
        }
    }

    fn manager_gist_source(&self) -> &[GistFile] {
        if self.gist_manager.type_filter.uses_starred_source() {
            &self.starred_gists
        } else {
            &self.gists
        }
    }

    /// `owner.login` for a gist id from the in-memory owned or starred lists.
    /// Iterator over every in-memory gist file — owned first, then starred. The shared base
    /// for the many lookups that must search both lists.
    fn all_gist_files(&self) -> impl Iterator<Item = &GistFile> {
        // A gist you own *and* starred is fetched by both `/gists` and `/gists/starred`,
        // so it appears in both lists. Owned takes precedence; skip the starred copy to
        // avoid showing each of its files twice in the detail view (issue #188).
        let owned_ids: std::collections::HashSet<&str> =
            self.gists.iter().map(|g| g.gist_id.as_str()).collect();
        self.gists.iter().chain(
            self.starred_gists
                .iter()
                .filter(move |g| !owned_ids.contains(g.gist_id.as_str())),
        )
    }

    pub fn gist_owner_login(&self, gist_id: &str) -> String {
        self.all_gist_files()
            .find(|g| g.gist_id == gist_id)
            .map(|g| g.owner_login.clone())
            .unwrap_or_default()
    }

    /// `raw_url` from the in-memory gist lists for a `(gist_id, filename)` pair.
    pub fn gist_file_raw_url(&self, gist_id: &str, filename: &str) -> Option<String> {
        self.all_gist_files()
            .find(|g| g.gist_id == gist_id && g.filename == filename)
            .and_then(|g| g.raw_url.clone())
    }

    pub fn gist_is_owned(&self, gist_id: &str) -> bool {
        if let Some(me) = self.current_user_login.as_deref() {
            self.all_gist_files()
                .find(|g| g.gist_id == gist_id)
                .is_some_and(|g| g.is_owned_by(me))
        } else {
            self.gists.iter().any(|g| g.gist_id == gist_id)
        }
    }

    pub fn gist_is_starred(&self, gist_id: &str) -> bool {
        self.starred_gist_ids.contains(gist_id)
    }

    /// Per-gist comment, stargazer, and fork counts for row/detail labels.
    pub fn gist_counts(&self, gist_id: &str) -> (u32, u32, u32) {
        (
            self.gist_comment_counts.get(gist_id).copied().unwrap_or(0),
            self.gist_star_counts.get(gist_id).copied().unwrap_or(0),
            self.gist_fork_counts.get(gist_id).copied().unwrap_or(0),
        )
    }

    /// Gists you have starred (unique ids from the starred list fetch).
    pub fn starred_gist_count(&self) -> usize {
        self.starred_gist_ids.len()
    }

    /// Owned gists that are forks of an upstream gist.
    pub fn owned_fork_gist_count(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        for g in &self.gists {
            if g.is_fork() {
                seen.insert(g.gist_id.as_str());
            }
        }
        seen.len()
    }

    /// Block mutating actions on gists you do not own. Returns `true` when blocked.
    pub fn block_if_foreign_gist(&mut self, gist_id: &str, pin: bool) -> bool {
        if self.gist_is_owned(gist_id) {
            return false;
        }
        let message = if pin {
            "cannot pin — not your gist"
        } else {
            "read-only — not your gist (* star; open detail and F to fork)"
        };
        self.set_status(message.to_string());
        true
    }

    /// Filtered owned/starred gist file rows (no ranking/sort). Shared by
    /// [`Self::ranked_gists`] and [`Self::list_pane_snapshots`].
    fn filtered_gist_files(&self) -> Vec<GistFile> {
        let query = self.filter_query.to_lowercase();
        self.list_gist_source()
            .iter()
            .filter(|g| self.gist_type_filter.matches_file(g))
            .filter(|g| {
                query.is_empty()
                    || g.filename.to_lowercase().contains(&query)
                    || g.description.to_lowercase().contains(&query)
            })
            .cloned()
            .collect()
    }

    /// Rank/sort gist files for a known local path (or unranked when `local_path` is
    /// `None` / anchor is Gist). Does **not** call `selected_local` / `visible_locals`.
    fn rank_gist_files_for(&self, local_path: Option<&std::path::Path>) -> Vec<RankedGistFile> {
        let gists = self.filtered_gist_files();
        let mut ranked = match local_path {
            Some(path) => rank_gist_files(path, &gists, &self.pinned),
            None => unranked_gists(gists),
        };
        self.gist_sort.apply(&mut ranked);
        ranked
    }

    /// Local rows with optional reverse-rank against a known gist file. Does **not**
    /// call `selected_gist` / `ranked_gists`.
    fn rank_local_files_for(&self, gist: Option<&GistFile>) -> Vec<RankedLocal> {
        let mut ranked = match gist {
            Some(file) => rank_local_files(file, &self.locals, &self.pinned),
            None => unranked_locals(&self.locals),
        };
        let query = self.local_filter_query.to_lowercase();
        if !query.is_empty() {
            ranked.retain(|r| {
                local_row_label(&r.candidate.path, &self.cwd)
                    .to_lowercase()
                    .contains(&query)
            });
        }
        self.local_sort.apply(&mut ranked);
        ranked
    }

    pub fn ranked_gists(&self) -> Vec<RankedGistFile> {
        // Anchor-driven ranking: the gist pane is ranked against the selected local file
        // only while the LOCAL pane is the anchor (anchor == Local). When the gist pane
        // is the anchor it uses its own sort (no ranking), which also breaks the
        // otherwise-mutual dependency with `visible_locals`.
        // NOTE: only evaluate the local selection inside the anchor==Local branch.
        // Computing it eagerly would recurse: selected_local -> visible_locals ->
        // selected_gist -> ranked_gists.
        let local_path = if self.anchor == FocusPane::Local {
            self.rank_local_files_for(None)
                .get(self.local_index)
                .map(|r| r.candidate.path.clone())
        } else {
            None
        };
        self.rank_gist_files_for(local_path.as_deref())
    }

    /// The local file list after sorting (and, while the gist pane drives, reverse ranking
    /// against the selected gist). Single source of truth for the local pane's order,
    /// selection, and rendering — mirrors `ranked_gists`.
    pub fn visible_locals(&self) -> Vec<RankedLocal> {
        // Mirror of `ranked_gists`: only evaluate the gist selection in the anchor==Gist
        // branch to avoid recursing back through `ranked_gists` -> `selected_local`.
        let gist = if self.anchor == FocusPane::Gist {
            self.rank_gist_files_for(None)
                .into_iter()
                .nth(self.gist_index)
                .map(|r| r.file)
        } else {
            None
        };
        self.rank_local_files_for(gist.as_ref())
    }

    /// Build both list-pane orderings with **one** construction of each list (issue #224 /
    /// shape #1 from #154). Prefer this when a key/palette/render path needs both sides or
    /// multiple selected rows — avoids N full filter+rank+sort passes per call site.
    ///
    /// Expansion order follows the anchor so the mutual recursion is not entered:
    /// - `Local` anchor: locals (driver) first, then gists ranked on the selected local
    /// - `Gist` anchor: gists (driver) first, then locals reverse-ranked on the selected gist
    ///
    /// Public `ranked_gists` / `visible_locals` / `selected_*` stay pure recomputes so unit
    /// tests keep the no-stale-cache contract; hot handlers should call this instead.
    pub fn list_pane_snapshots(&self) -> (Vec<RankedLocal>, Vec<RankedGistFile>) {
        match self.anchor {
            FocusPane::Local => {
                let locals = self.rank_local_files_for(None);
                let path = locals
                    .get(self.local_index)
                    .map(|r| r.candidate.path.as_path());
                let gists = self.rank_gist_files_for(path);
                (locals, gists)
            }
            FocusPane::Gist => {
                let gists = self.rank_gist_files_for(None);
                let gist = gists.get(self.gist_index).map(|r| &r.file);
                let locals = self.rank_local_files_for(gist);
                (locals, gists)
            }
        }
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

    /// All gists collapsed to one entry each (raw, unfiltered) from the owned list.
    pub fn gist_groups(&self) -> Vec<GistGroup> {
        group_gists(&self.gists)
    }

    /// The gist-level view's rows after the visibility filter, text filter, and sort
    /// are applied. This is the single source of truth for navigation, selection, and
    /// rendering in `Screen::Gists`.
    pub fn visible_gist_groups(&self) -> Vec<GistGroup> {
        let query = self.gist_manager.filter_query.to_lowercase();
        let mut groups: Vec<GistGroup> = group_gists(self.manager_gist_source())
            .into_iter()
            .filter(|g| self.gist_manager.type_filter.matches_group(g))
            .filter(|g| {
                query.is_empty()
                    || g.description.to_lowercase().contains(&query)
                    || g.id.to_lowercase().contains(&query)
            })
            .collect();
        match self.gist_manager.sort {
            GistGroupSort::Updated => groups.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
            GistGroupSort::Created => groups.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        }
        groups
    }

    /// The gist highlighted in the gist-level view.
    pub fn selected_group(&self) -> Option<GistGroup> {
        self.visible_gist_groups()
            .into_iter()
            .nth(self.gist_manager.index)
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
                    self.gist_manager.sort,
                    (
                        self.gist_comment_counts.get(&g.id).copied().unwrap_or(0),
                        self.gist_star_counts.get(&g.id).copied().unwrap_or(0),
                        self.gist_fork_counts.get(&g.id).copied().unwrap_or(0),
                    ),
                    self.gist_is_starred(&g.id),
                    self.current_user_login.as_deref(),
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

    /// Indices into `self.pinned` that match the Pins-screen text filter, in sort order.
    /// Empty query → every index. Matched against the cwd/home-shortened local path plus
    /// the gist filename (the meaningful, visible parts of the row).
    pub fn visible_pin_indices(&self) -> Vec<usize> {
        let query = self.pins.filter_query.to_lowercase();
        let mut indices: Vec<usize> = self
            .pinned
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
            .collect();
        match self.pins.sort {
            PinSort::Default => {}
            PinSort::Local => indices.sort_by(|&a, &b| {
                crate::config::display_path(&self.pinned[a].local_path)
                    .cmp(&crate::config::display_path(&self.pinned[b].local_path))
            }),
            PinSort::Gist => indices.sort_by(|&a, &b| {
                self.pinned[a]
                    .gist_filename
                    .cmp(&self.pinned[b].gist_filename)
            }),
        }
        indices
    }

    /// The true `self.pinned` index of the currently selected Pins row (selection is a
    /// position within the filtered view).
    pub fn selected_pin_index(&self) -> Option<usize> {
        self.visible_pin_indices().get(self.pins.index).copied()
    }

    /// Number of files the given gist holds in the current in-memory list. Used to guard
    /// against removing a gist's only file (GitHub forbids a fileless gist).
    fn gist_file_count(&self, gist_id: &str) -> usize {
        self.all_gist_files()
            .filter(|g| g.gist_id == gist_id)
            .count()
    }

    /// Filenames the given gist holds in the current in-memory list (gh order).
    pub fn gist_filenames(&self, gist_id: &str) -> Vec<String> {
        self.all_gist_files()
            .filter(|g| g.gist_id == gist_id)
            .map(|g| g.filename.clone())
            .collect()
    }

    pub fn gist_file_content_type(&self, gist_id: &str, filename: &str) -> Option<String> {
        self.all_gist_files()
            .find(|g| g.gist_id == gist_id && g.filename == filename)
            .and_then(|g| g.content_type.clone())
    }

    pub fn gist_file_is_text_previewable(&self, gist_id: &str, filename: &str) -> bool {
        crate::domain::gist_file_is_text_previewable(
            filename,
            self.gist_file_content_type(gist_id, filename).as_deref(),
        )
    }

    /// Returns true when preview/diff should be blocked for this gist file (sets `status`).
    pub fn block_if_non_previewable_gist_file(&mut self, gist_id: &str, filename: &str) -> bool {
        if self.gist_file_is_text_previewable(gist_id, filename) {
            return false;
        }
        self.status = Some(crate::domain::non_previewable_status(
            filename,
            self.gist_file_content_type(gist_id, filename).as_deref(),
        ));
        true
    }

    /// Like [`Self::block_if_non_previewable_gist_file`], but also rejects binary-looking local files.
    pub fn block_if_non_previewable_diff(
        &mut self,
        gist_id: &str,
        filename: &str,
        local_path: Option<&std::path::Path>,
    ) -> bool {
        if self.block_if_non_previewable_gist_file(gist_id, filename) {
            return true;
        }
        if let Some(path) = local_path {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if !crate::domain::gist_file_is_text_previewable(name, None) {
                    self.status =
                        Some("cannot diff — local file looks binary (use d to download)".into());
                    return true;
                }
            }
        }
        false
    }

    /// Detail-view file labels; non-text files are tagged `(binary)`.
    pub fn gist_file_display_names(&self, gist_id: &str) -> Vec<String> {
        self.gist_filenames(gist_id)
            .into_iter()
            .map(|f| {
                if self.gist_file_is_text_previewable(gist_id, &f) {
                    f
                } else {
                    format!("{f} (binary)")
                }
            })
            .collect()
    }

    /// Look up a gist group by id (unaffected by filtering); used by detail + confirm background.
    /// Open `Screen::Revisions` for the gist on `return_screen`. Returns false when no gist
    /// is selected or the gist has no files.
    pub fn open_revisions(&mut self, return_screen: Screen) -> bool {
        // Snapshot the selected gist once for the List path; it feeds both `gist_id` and
        // `target_file` below, avoiding a second `ranked_gists` recompute (perf-1, #154).
        let selected_list_gist = match return_screen {
            Screen::List => self.selected_gist(),
            _ => None,
        };
        let gist_id = match return_screen {
            Screen::List => selected_list_gist.as_ref().map(|g| g.file.gist_id.clone()),
            Screen::GistDetail => self.detail.gist_id.clone(),
            Screen::Gists => self.selected_group().map(|g| g.id.clone()),
            _ => None,
        };
        let Some(gist_id) = gist_id else {
            return false;
        };
        let filenames = self.gist_filenames(&gist_id);
        let target_file = match return_screen {
            Screen::List => selected_list_gist
                .as_ref()
                .map(|g| g.file.filename.clone())
                .filter(|f| filenames.iter().any(|name| name == f)),
            Screen::GistDetail => filenames
                .into_iter()
                .nth(self.detail.file_cursor)
                .or_else(|| self.gist_filenames(&gist_id).first().cloned()),
            Screen::Gists => filenames.first().cloned(),
            _ => None,
        };
        let Some(target_file) = target_file else {
            return false;
        };
        self.revision.gist_id = Some(gist_id);
        self.revision.target_file = target_file;
        self.revision.return_screen = return_screen;
        self.revision.index = 0;
        self.revision.hscroll = 0;
        self.revision.entries = None;
        self.revision.fetch_error = None;
        self.screen = Screen::Revisions;
        true
    }

    pub fn selected_revision(&self) -> Option<&GistRevision> {
        let entries = self.revision.entries.as_ref()?;
        entries.get(self.revision.index)
    }

    /// Advance `revision_target_file` to the next filename in this gist (wraps). Returns
    /// false when the gist has at most one file.
    pub fn cycle_revision_target_file(&mut self) -> bool {
        let Some(gist_id) = self.revision.gist_id.clone() else {
            return false;
        };
        let files = self.gist_filenames(&gist_id);
        if files.len() <= 1 {
            return false;
        }
        let current = files
            .iter()
            .position(|f| f == &self.revision.target_file)
            .unwrap_or(0);
        self.revision.target_file = files[(current + 1) % files.len()].clone();
        true
    }

    /// True when the diff view supports local↔gist download/upload (`d`/`u`). Revision-history
    /// diffs (returning to `Screen::Revisions`) are read-only comparisons.
    pub fn diff_allows_sync(&self) -> bool {
        self.diff_return != Screen::Revisions
    }

    /// Footer label for the revision-history target file, including `(n/total)` when multi-file.
    pub fn revision_target_file_label(&self) -> String {
        let Some(gist_id) = self.revision.gist_id.as_deref() else {
            return self.revision.target_file.clone();
        };
        let files = self.gist_filenames(gist_id);
        if files.len() <= 1 {
            return self.revision.target_file.clone();
        }
        let pos = files
            .iter()
            .position(|f| f == &self.revision.target_file)
            .map(|i| i + 1)
            .unwrap_or(1);
        format!("{} ({pos}/{})", self.revision.target_file, files.len())
    }

    pub fn group_by_id(&self, gist_id: &str) -> Option<GistGroup> {
        let files: Vec<GistFile> = self
            .all_gist_files()
            .filter(|g| g.gist_id == gist_id)
            .cloned()
            .collect();
        group_gists(&files).into_iter().find(|g| g.id == gist_id)
    }

    /// The gist the current screen acts on: the gist-level cursor on `Gists`, the
    /// viewed gist on `GistDetail`, otherwise the gist owning the selected file row.
    /// Screen-aware so IO actions (open-in-browser, compact) target what the user sees.
    pub fn context_gist_id(&self) -> Option<String> {
        match self.screen {
            Screen::Gists => self.selected_group().map(|g| g.id),
            Screen::GistDetail => self.detail.gist_id.clone(),
            _ => self.selected_gist().map(|g| g.file.gist_id),
        }
    }

    /// Upload intent shared by the list and the diff screen: requires a selected local file
    /// and gist, then branches on whether the gist already holds a file of the local name
    /// (case C: preview + confirm overwrite) or not (case B: add directly).
    /// True when we're in the diff screen launched from a Pins context (pin diff or pin pull).
    /// In this state `preview_local` holds the pin's local file and `download_gist_id/filename`
    /// hold the pin's gist identity, so upload/download should use those instead of the
    /// Files-view selection which may point to a completely different pair.
    pub fn is_pin_diff_context(&self) -> bool {
        self.screen == Screen::Diff
            && !self.preview_local.as_os_str().is_empty()
            && self.download_gist_id.is_some()
    }

    fn upload_intent(&mut self) -> KeyOutcome {
        if let Some(gist) = self.selected_gist() {
            if self.block_if_foreign_gist(&gist.file.gist_id, false) {
                return KeyOutcome::None;
            }
        }
        if self.is_pin_diff_context() {
            let Some(local_filename) = self
                .preview_local
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
            else {
                self.status = Some("local file has no name".into());
                return KeyOutcome::None;
            };
            let gist_id = self.download_gist_id.as_deref().unwrap_or_default();
            let has_same_name = self
                .gists
                .iter()
                .any(|g| g.gist_id == gist_id && g.filename == local_filename);
            return if has_same_name {
                KeyOutcome::UploadPreview
            } else {
                KeyOutcome::UploadAdd
            };
        }
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
        let max = self.diff_vscroll_max();
        if self.diff_scroll < max {
            self.diff_scroll += 1;
        }
    }

    /// Bottom clamp for the diff/preview vertical scroll: the last addressable line index.
    fn diff_vscroll_max(&self) -> u16 {
        self.diff_text
            .lines()
            .count()
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16
    }

    pub fn scroll_diff_up(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_sub(1);
    }

    /// Page the diff/preview down by `lines`, clamped to the same bottom as `scroll_diff_down`.
    pub fn scroll_diff_page_down(&mut self, lines: u16) {
        let max = self.diff_vscroll_max();
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
        starred_gists: Vec::new(),
        starred_gist_ids: std::collections::HashSet::new(),
        current_user_login: None,
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
        filter_query: TextInput::default(),
        local_filter_query: TextInput::default(),
        diff_previewed: false,
        diff_text: String::new(),
        diff_scroll: 0,
        diff_hscroll: 0,
        diff_wrap: false,
        diff_identical: false,
        diff_context: 3,
        diff_show_full: false,
        ignore_trailing_newline: true,
        preview_remote: String::new(),
        preview_local: PathBuf::new(),
        download_target: PathBuf::new(),
        cwd: PathBuf::from("."),
        status: None,
        loading: false,
        preview_title: String::new(),
        preview_wrap: false,
        syntax_highlight: true,
        config_mouse: true,
        mouse_enabled: true,
        no_mouse_cli: false,
        config_check_updates: true,
        update_check_enabled: true,
        no_update_check_cli: false,
        config: ConfigState::default(),
        update_available: None,
        install_method: crate::upgrade::InstallMethod::Standalone,
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
        local_scan_generation: 0,
        pins: PinsState::default(),
        gist_manager: GistsManagerState::default(),
        editing_description: false,
        description_input: TextInput::default(),
        bg_task_msg: None,
        bg_task_generation: 0,
        quit_armed: false,
        help: HelpState::default(),
        upload: UploadState::default(),
        download_gist_id: None,
        download_gist_filename: None,
        diff_return: Screen::List,
        detail: DetailState {
            compact_return_screen: Screen::Gists,
            ..Default::default()
        },
        spinner_frame: 0,
        gist_comment_counts: std::collections::HashMap::new(),
        gist_fork_counts: std::collections::HashMap::new(),
        gist_star_counts: std::collections::HashMap::new(),
        theme_choice: crate::config::ThemeChoice::Dark,
        theme: Theme::DARK,
        revision: RevisionState {
            return_screen: Screen::GistDetail,
            ..Default::default()
        },
        palette: PaletteState::default(),
    }
}

pub fn load_startup_state(no_mouse: bool, no_update_check: bool) -> Result<AppState> {
    let mut state = initial_state();
    let config_path = crate::config::config_path()?;
    let config = crate::config::load_config(&config_path)?;
    let cwd = std::env::current_dir()?;

    state.pinned = config.pinned;
    state.skip_dirs = config.skip_dirs;
    state.scan_depth = config.scan_depth;
    state.diff_context = config.diff_context;
    state.diff_show_full = config.diff_show_full;
    state.ignore_trailing_newline = config.ignore_trailing_newline;
    state.theme_choice = config.theme;
    state.theme = Theme::for_choice(config.theme);
    // Honour NO_COLOR for the syntax-highlight feature only (existing semantic colours stay).
    state.syntax_highlight = std::env::var_os("NO_COLOR").is_none();
    state.config_mouse = config.mouse;
    state.no_mouse_cli = no_mouse;
    state.mouse_enabled = crate::config::resolve_mouse_enabled(config.mouse, no_mouse);
    state.config_check_updates = config.check_updates;
    state.no_update_check_cli = no_update_check;
    state.update_check_enabled =
        crate::config::resolve_update_check(config.check_updates, no_update_check);
    // Surface a previously-seen newer release immediately (even when the daily check is
    // throttled), so the hint persists across launches without re-hitting the network.
    if state.update_check_enabled {
        if let Ok(exe) = std::env::current_exe() {
            state.install_method = crate::upgrade::detect_install_method(&exe);
        }
        if let Ok(path) = crate::update_check::state_path() {
            let seen = crate::update_check::load_state(&path).latest_seen;
            state.update_available =
                crate::update_check::is_newer(&seen, env!("CARGO_PKG_VERSION"));
        }
    }
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
    // Show last-known gists (owned + starred + counts) from cache; background fetch refreshes.
    if let Ok(path) = crate::cache::cache_path() {
        if let Some(cache) = crate::cache::load_gist_cache(&path) {
            state.starred_gist_ids = cache.starred_ids_set();
            state.gists = cache.owned;
            state.starred_gists = cache.starred;
            state.current_user_login = cache.user_login;
            state.gist_comment_counts = cache.comment_counts;
            state.gist_fork_counts = cache.fork_counts;
            state.gist_star_counts = cache.star_counts;
        }
    }

    Ok(state)
}

pub fn run(no_mouse: bool, no_update_check: bool) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, no_mouse, no_update_check);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    // Always emitted (harmless if capture was never enabled), so it runs even on error.
    let _ = execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture
    );

    result
}

impl AppState {
    /// Resolve a pin's absolute local path against `cwd`.
    fn pin_local_abs(&self, m: &crate::domain::PinnedMapping) -> PathBuf {
        if m.local_path.is_absolute() {
            m.local_path.clone()
        } else {
            self.cwd.join(&m.local_path)
        }
    }

    /// `(local_ts, remote_ts)` Unix-seconds for `pinned[index]`. The remote side comes
    /// from the matching gist's in-memory `updated_at`; the local side prefers the
    /// discovered candidate's mtime and falls back to stat-ing the path on disk.
    pub fn pin_mtimes(&self, index: usize) -> (Option<u64>, Option<u64>) {
        let Some(m) = self.pinned.get(index) else {
            return (None, None);
        };
        let local_abs = self.pin_local_abs(m);
        let local_ts = self
            .locals
            .iter()
            .find_map(|c| {
                let cabs = if c.path.is_absolute() {
                    c.path.clone()
                } else {
                    self.cwd.join(&c.path)
                };
                (cabs == local_abs).then_some(c.modified).flatten()
            })
            // Pins can point outside cwd (or into skipped/too-deep dirs), so they
            // never appear in `self.locals`. Fall back to stat-ing the path so the
            // Pins list and sync status still reflect the real mtime.
            .or_else(|| crate::local::file_mtime_secs(&local_abs));
        let remote_ts = self.gists.iter().find_map(|g| {
            (g.gist_id == m.gist_id && g.filename == m.gist_filename)
                .then(|| crate::domain::parse_rfc3339_to_unix(&g.updated_at))
                .flatten()
        });
        (local_ts, remote_ts)
    }

    /// Derive the [`SyncStatus`] for `pinned[index]` from in-memory mtimes, with a
    /// content-hash fallback: when the timestamps disagree (`Push`/`Pull`) but the local
    /// file's current content still hashes to the pin's `last_seen_hash` baseline, report
    /// `InSync` instead — the timestamp drifted but nothing actually changed. The extra
    /// file read only happens for this ambiguous, has-a-baseline case, not on every pin.
    pub fn pin_sync_status(&self, index: usize) -> crate::domain::SyncStatus {
        let (local_ts, remote_ts) = self.pin_mtimes(index);
        let status = crate::domain::sync_status(local_ts, remote_ts);
        if !matches!(
            status,
            crate::domain::SyncStatus::Push | crate::domain::SyncStatus::Pull
        ) {
            return status;
        }
        let Some(m) = self.pinned.get(index) else {
            return status;
        };
        let Some(baseline) = m.last_seen_hash.as_deref() else {
            return status;
        };
        let local_abs = self.pin_local_abs(m);
        match std::fs::read(&local_abs) {
            Ok(bytes) if crate::domain::sha256_hex(&bytes) == baseline => {
                crate::domain::SyncStatus::InSync
            }
            _ => status,
        }
    }
}

/// The result of the initial newest-first comment load: the newest page plus the metadata
/// needed to page backwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialComments {
    pub comments: Vec<GistComment>,
    pub total: u32,
    pub oldest_page: u32,
}

impl AppState {
    /// Reset comment-pagination state (called when (re)opening a gist detail or switching
    /// the loaded gist), so a fresh Tab re-fetches from the newest page.
    pub fn reset_comment_pagination(&mut self) {
        self.detail.comments = None;
        self.detail.comments_loading = false;
        self.detail.comments_error = None;
        self.detail.comments_total = None;
        self.detail.comments_loaded_oldest_page = 0;
        self.detail.comments_loading_more = false;
        self.detail.comments_scroll_to_bottom = false;
    }

    /// Apply the initial newest-page load. Ignored if the user navigated to another gist
    /// (stale response). On success, requests a one-shot scroll-to-bottom so the newest
    /// comment is visible.
    pub fn apply_initial_comments(
        &mut self,
        gist_id: &str,
        result: Result<InitialComments, String>,
    ) {
        if self.detail.gist_id.as_deref() != Some(gist_id) {
            return;
        }
        self.detail.comments_loading = false;
        match result {
            Ok(init) => {
                self.detail.comments_total = Some(init.total);
                self.detail.comments_loaded_oldest_page = init.oldest_page;
                self.detail.comments = Some(init.comments);
                self.detail.comments_scroll_to_bottom = true;
            }
            Err(error) => {
                self.detail.comments_error = Some(error);
            }
        }
    }

    /// Apply a "load older" page: prepend it (older comments sort first) and bump
    /// `detail_scroll` by the prepended line count so the viewport stays put. Ignored on
    /// stale gist.
    pub fn apply_older_comments(
        &mut self,
        gist_id: &str,
        result: Result<Vec<GistComment>, String>,
    ) {
        if self.detail.gist_id.as_deref() != Some(gist_id) {
            return;
        }
        self.detail.comments_loading_more = false;
        match result {
            Ok(mut older) => {
                let added = comment_lines_count(&older);
                if let Some(existing) = self.detail.comments.as_mut() {
                    older.append(existing);
                    *existing = older;
                } else {
                    self.detail.comments = Some(older);
                }
                self.detail.comments_loaded_oldest_page = self
                    .detail
                    .comments_loaded_oldest_page
                    .saturating_sub(1)
                    .max(1);
                self.detail.scroll = self.detail.scroll.saturating_add(added);
            }
            Err(error) => {
                self.detail.comments_error = Some(error);
            }
        }
    }

    /// Whether a "load older" action should be offered: comments are loaded, an older page
    /// exists, and no load is already in flight.
    pub fn can_load_older_comments(&self) -> bool {
        self.detail.comments.is_some()
            && self.detail.comments_loaded_oldest_page > 1
            && !self.detail.comments_loading_more
            && !self.detail.comments_loading
    }
}

/// Draw a centered, bordered box over the current frame, sized to fit `body` (clamped to
/// the frame) and wiped clean with `Clear` so whatever is behind it doesn't bleed through.
/// This is the shared "centered window" primitive behind both the loading overlay and the
/// confirm prompt.
mod highlight;
mod palette;
use palette::{PaletteItem, PaletteMode, PaletteState};

mod render;
use render::*;
mod text;
use text::{comment_lines_count, local_row_label};
mod bg;
mod dispatch;
mod keys;
mod run_loop;
use run_loop::run_loop;
mod text_input;
pub use text_input::{EditResult, TextInput};
mod theme;
pub use theme::Theme;

#[cfg(test)]
mod tests;
