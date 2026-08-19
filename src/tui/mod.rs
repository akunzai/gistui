use crate::domain::{
    group_gists, GistComment, GistFile, GistFileRef, GistGroup, GistRevision, LocalCandidate,
    PinnedMapping,
};
use crate::ranking::{RankedGistFile, RankedLocal};
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

/// Active TUI screen. Unit tags for List only; other screens carry payloads so
/// screen-local UI cannot go stale on the root state (issue #242). Not `Copy`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    List,
    /// Unified-diff view; payload owns body/scroll/return.
    Diff(Box<DiffState>),
    /// y/n (and multi-step create) modal; payload owns action, background text, return.
    Confirm(Box<ConfirmState>),
    /// Full-screen file content preview; payload owns title/body/scroll/return.
    Preview(Box<PreviewState>),
    /// Help topic/index state lives on the variant (not a parallel `AppState` field).
    /// Boxed so `HelpState::return_screen: Screen` does not make `Screen` infinite-sized.
    Help(Box<HelpState>),
    /// Pins list cursor/filter/sort; may also sit on Diff/Confirm return paths.
    Pins(Box<PinsState>),
    /// Gist manager list cursor/filter/sort; may sit on detail/revision return paths.
    Gists(Box<GistsManagerState>),
    /// Single-gist detail (files + comments); may sit on `preview_return` / compact restore.
    /// Boxed so `DetailState::{return,compact_return}_screen: Screen` stay finite-sized.
    GistDetail(Box<DetailState>),
    /// Revision history for one gist; payload owns list/cursor/return (and may sit in
    /// Diff/Confirm return while a revision diff/confirm is open).
    Revisions(Box<RevisionState>),
    /// Command palette / context menu overlay; payload owns query, items, origin, selection.
    /// Boxed so `PaletteState::origin_screen: Screen` stays finite-sized.
    Palette(Box<PaletteState>),
    /// Flat settings list (issue #227); payload owns cursor + return path.
    Config(Box<ConfigState>),
}

/// Tag + payload accessors for one `Screen` variant (`is_*` / `*_state` / `*_state_mut`).
macro_rules! screen_payload {
    ($is:ident, $variant:ident, $get:ident, $get_mut:ident, $ty:ty) => {
        pub fn $is(&self) -> bool {
            matches!(self, Screen::$variant(_))
        }

        pub fn $get(&self) -> Option<&$ty> {
            match self {
                Screen::$variant(p) => Some(p),
                _ => None,
            }
        }

        pub fn $get_mut(&mut self) -> Option<&mut $ty> {
            match self {
                Screen::$variant(p) => Some(p),
                _ => None,
            }
        }
    };
}

impl Screen {
    // Tag equality ignoring payloads (e.g. "are we on Help?" regardless of topic).
    screen_payload!(is_help, Help, help_state, help_state_mut, HelpState);
    screen_payload!(
        is_config,
        Config,
        config_state,
        config_state_mut,
        ConfigState
    );
    screen_payload!(
        is_revisions,
        Revisions,
        revision_state,
        revision_state_mut,
        RevisionState
    );
    screen_payload!(is_pins, Pins, pins_state, pins_state_mut, PinsState);
    screen_payload!(
        is_gists,
        Gists,
        gists_state,
        gists_state_mut,
        GistsManagerState
    );
    screen_payload!(
        is_gist_detail,
        GistDetail,
        detail_state,
        detail_state_mut,
        DetailState
    );
    screen_payload!(
        is_palette,
        Palette,
        palette_state,
        palette_state_mut,
        PaletteState
    );
    screen_payload!(
        is_preview,
        Preview,
        preview_state,
        preview_state_mut,
        PreviewState
    );
    screen_payload!(is_diff, Diff, diff_state, diff_state_mut, DiffState);
    screen_payload!(
        is_confirm,
        Confirm,
        confirm_state,
        confirm_state_mut,
        ConfirmState
    );
}

/// Fields shown on [`Screen::Config`] in order (issue #227).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    Theme,
    Mouse,
    CheckUpdates,
    DiffShowFull,
    IgnoreTrailingNewline,
    ScanDepth,
    DiffContext,
}

impl ConfigField {
    pub const ALL: [ConfigField; 7] = [
        ConfigField::Theme,
        ConfigField::Mouse,
        ConfigField::CheckUpdates,
        ConfigField::DiffShowFull,
        ConfigField::IgnoreTrailingNewline,
        ConfigField::ScanDepth,
        ConfigField::DiffContext,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConfigField::Theme => "Theme",
            ConfigField::Mouse => "Mouse support",
            ConfigField::CheckUpdates => "Check for updates",
            ConfigField::DiffShowFull => "Show full diff",
            ConfigField::IgnoreTrailingNewline => "Ignore trailing newline",
            ConfigField::ScanDepth => "Recursive scan depth",
            ConfigField::DiffContext => "Diff context lines",
        }
    }

    pub fn is_numeric(self) -> bool {
        matches!(self, ConfigField::ScanDepth | ConfigField::DiffContext)
    }

    pub fn description(self) -> &'static str {
        match self {
            ConfigField::Theme => "terminal colours",
            ConfigField::Mouse => "click and wheel input",
            ConfigField::CheckUpdates => "daily GitHub version check",
            ConfigField::DiffShowFull => "open Diff expanded",
            ConfigField::IgnoreTrailingNewline => "hide newline-only diffs",
            ConfigField::ScanDepth => "directory levels to scan",
            ConfigField::DiffContext => "unchanged lines around edits",
        }
    }
}

/// Settings-screen state — carried on [`Screen::Config`] (issue #242).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigState {
    pub index: usize,
}

/// A help topic — one per key-dense area, plus `About` (version/repo/update info, not tied
/// to a screen). Ordered for the index list and `1`-`9`, `g`, `0` quick-jump.
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
    pub fn for_screen(screen: &Screen) -> HelpTopic {
        match screen {
            Screen::List => screens::list::help_topic(),
            Screen::Diff(_) => screens::diff::help_topic(),
            Screen::Confirm(_) => screens::confirm::help_topic(),
            Screen::Preview(_) => screens::preview::help_topic(),
            Screen::Help(_) => screens::help::help_topic(),
            Screen::Pins(_) => screens::pins::help_topic(),
            Screen::Gists(_) => screens::gists::help_topic(),
            Screen::GistDetail(_) => screens::detail::help_topic(),
            Screen::Revisions(_) => screens::revisions::help_topic(),
            Screen::Palette(_) => screens::palette::help_topic(),
            Screen::Config(_) => screens::config::help_topic(),
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

/// Pure key/mouse intent returned by [`AppState::handle_key`]. IO-bearing variants carry
/// the snapshot needed to execute so dispatch does not re-resolve selection (issue #244).
/// Not `Copy` — payloads own small strings/paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyOutcome {
    None,
    Quit,
    /// List-originated local↔gist diff.
    PreviewDiff {
        local_path: Option<PathBuf>,
        file: GistFileRef,
        target: PathBuf,
        /// When true, unified diff is oriented local→gist (local focus).
        upload_orientation: bool,
    },
    /// Download using the open Diff payload (no re-resolve).
    /// `mode` is [`DownloadMode::CreateNew`] when the target is missing, or
    /// [`DownloadMode::overwrite_after_user_confirm`] after Confirm `y` (issue #246).
    Download {
        mode: crate::actions::DownloadMode,
    },
    /// Request a download from the open Diff payload. Dispatch checks the target immediately
    /// before choosing either an overwrite confirmation or a new-file download.
    DownloadRequested {
        target: PathBuf,
    },
    /// Download/fetch a selected gist file (may land on Diff/Confirm).
    DownloadGist {
        file: GistFileRef,
        target: PathBuf,
    },
    Pin {
        local_path: PathBuf,
        gist_id: String,
        filename: String,
    },
    Unpin {
        local_path: PathBuf,
        gist_id: String,
        filename: String,
    },
    UploadAdd {
        local_path: PathBuf,
        gist_id: String,
        filename: String,
    },
    UploadPreview {
        local_path: PathBuf,
        file: GistFileRef,
        /// When true, keep pin-originated `diff_return`; when false, reset to List.
        from_pin_diff: bool,
    },
    /// Confirm-owned upload execute.
    Upload,
    Create(bool),
    PreviewContent {
        file: GistFileRef,
    },
    OpenBrowser {
        gist_id: String,
    },
    EditLocal {
        path: PathBuf,
    },
    EditUpload,
    ExecuteDelete,
    ExecuteRemoveFile,
    OpenGistDetail {
        gist_id: String,
    },
    FetchComments {
        gist_id: String,
    },
    LoadOlderComments {
        gist_id: String,
        page: u32,
    },
    CompactGist {
        gist_id: String,
        label: String,
    },
    ExecuteCompactGist,
    ApplyDescription {
        gist_id: String,
        description: String,
    },
    RefreshLocals,
    OpenRepoUrl {
        url: String,
    },
    RefreshPreview {
        file: GistFileRef,
    },
    UnpinAtPin {
        index: usize,
    },
    SyncPinAuto {
        index: usize,
    },
    SyncPinPush {
        index: usize,
    },
    SyncPinPull {
        index: usize,
    },
    SyncSelectedPair {
        local_path: PathBuf,
        gist_id: String,
        filename: String,
    },
    PreviewPinDiff {
        index: usize,
    },
    PersistDiffContext,
    CopyGistUrl {
        gist_id: String,
    },
    CopyPreviewContent,
    ThemeToggle,
    PersistSettings,
    FetchRevisions {
        gist_id: String,
    },
    RevisionDiffIncremental {
        gist_id: String,
        filename: String,
        child_version: String,
        parent_version: Option<String>,
        old_label: String,
        new_label: String,
        owner_login: String,
    },
    RevisionDiff {
        gist_id: String,
        filename: String,
        version: String,
        old_label: String,
        new_label: String,
        raw_url: Option<String>,
        owner_login: String,
    },
    RestoreRevisionPreview {
        gist_id: String,
        filename: String,
        version: String,
        version_label: String,
        raw_url: Option<String>,
        owner_login: String,
    },
    ExecuteRestoreRevision,
    ToggleGistStar {
        gist_id: String,
        /// True when the next action should star (currently unstarred).
        starring: bool,
    },
    ForkGist {
        gist_id: String,
    },
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

/// Revision-history state — carried on [`Screen::Revisions`] (issue #242).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RevisionState {
    /// Gist whose revisions are shown.
    pub gist_id: Option<String>,
    /// Fetched revision rows (`None` while the initial list fetch is in flight).
    pub entries: Option<Vec<GistRevision>>,
    /// Cursor into `entries` (0 = current head).
    /// Not a [`ListCursor`]: Revisions does not reset hscroll on vertical move and does
    /// not clamp Right to an hmax (issue #274 — out of scope).
    pub index: usize,
    pub hscroll: u16,
    /// File within the gist that preview/diff/restore target.
    pub target_file: String,
    /// Error from the commits-list fetch, if any.
    pub fetch_error: Option<String>,
}

/// Per-screen Help-view state — carried on [`Screen::Help`] (issue #242).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HelpState {
    pub scroll: u16,
    /// The topic shown in the Help screen's topic view.
    pub topic: HelpTopic,
    /// When true the Help screen shows the topic index instead of a topic body.
    pub index_open: bool,
    /// Highlighted row in the Help topic index.
    pub index_sel: usize,
}

/// Pins-screen state — carried on [`Screen::Pins`] (issue #242).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PinsState {
    /// Selection + row hscroll (shared Pins/Gists policy; issue #274).
    pub cursor: ListCursor,
    pub filtering: bool,
    pub filter_query: TextInput,
    pub sort: PinSort,
}

/// Full-screen content preview — carried on [`Screen::Preview`] (issue #242).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreviewState {
    pub title: String,
    /// Body text shown in the preview pane.
    pub text: String,
    pub scroll: u16,
    pub hscroll: u16,
    /// `(gist_id, filename)` for refresh / copy-url context.
    pub gist_key: Option<(String, String)>,
}

/// Unified-diff view — carried on [`Screen::Diff`] (issue #242).
/// Owns body, scroll, pairing paths, and optional pin gist identity so inactive Diff
/// state cannot linger on the root (including while parked under Confirm).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiffState {
    pub text: String,
    pub scroll: u16,
    pub hscroll: u16,
    /// Remote file body (download source content).
    pub remote_content: String,
    /// Local path paired in the diff (empty for revision-only comparisons).
    pub local_path: PathBuf,
    /// Path a download would write to.
    pub download_target: PathBuf,
    /// True when local and remote content compare equal under config rules.
    pub identical: bool,
    /// Optional gist file identity (pin diffs / attributed pulls).
    pub gist_id: Option<String>,
    pub gist_filename: Option<String>,
}

/// Confirm modal — carried on [`Screen::Confirm`] (issue #242).
/// Owns the pending action and the background text (real diff or short message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmState {
    pub action: PendingAction,
    /// Background body (unified diff or explanatory message).
    pub text: String,
    pub scroll: u16,
    pub hscroll: u16,
}

impl Default for ConfirmState {
    fn default() -> Self {
        Self {
            action: PendingAction::Download,
            text: String::new(),
            scroll: 0,
            hscroll: 0,
        }
    }
}

/// Gist-manager screen state — carried on [`Screen::Gists`] (issue #242). Named
/// `gist_manager` in accessors because the `gists` field name is taken by the gist list
/// `Vec`. Data only — methods stay on `AppState`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GistsManagerState {
    /// Selection + row hscroll (shared Pins/Gists policy; issue #274).
    pub cursor: ListCursor,
    pub sort: GistGroupSort,
    pub type_filter: GistTypeFilter,
    pub filtering: bool,
    pub filter_query: TextInput,
}

/// Gist-detail screen state — carried on [`Screen::GistDetail`] (issue #242). Data only —
/// the detail/comment methods stay on `AppState`. The `comments_*` count/paging fields keep
/// their prefix so they don't collide with the `comments` Vec.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    /// Soft-wrap long lines in the diff view instead of horizontal scrolling (`w` toggles;
    /// session-scoped, mirrors `preview_wrap`).
    pub diff_wrap: bool,
    /// Unchanged context lines kept around each change in the diff view (from config).
    pub diff_context: u32,
    /// When true the diff view shows the full file; when false it collapses to
    /// `diff_context` lines. Toggled with `c` and persisted to config.
    pub diff_show_full: bool,
    /// Treat a file-final-newline-only delta as no change in the diff view and the
    /// overwrite-confirm gate (from config; default `true`).
    pub ignore_trailing_newline: bool,
    pub cwd: PathBuf,
    pub status: Option<String>,
    pub loading: bool,
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
    /// Newer release version found by the background check, if any (footer hint on the List).
    pub update_available: Option<String>,
    /// How this binary was installed — resolved once at startup so the update hint can show
    /// the right upgrade command without per-frame IO.
    pub install_method: crate::upgrade::InstallMethod,
    pub(crate) gist_content_cache: crate::lru::LruCache<(String, String), String>,
    pub local_recursive: bool,
    pub skip_dirs: Vec<String>,
    pub scan_depth: u32,
    pub local_scanning: bool,
    /// Generation token for the in-flight local scan. Bumped on each
    /// [`Self::begin_local_scan`]; absorb ignores results with a mismatched id so a
    /// slow older scan cannot clobber a newer one (issue #221).
    pub local_scan_generation: u64,

    pub editing_description: bool,
    pub description_input: TextInput,
    pub bg_task_msg: Option<String>,
    /// Generation token for the in-flight `spawn_bg` task. Bumped on spawn and on
    /// cancel; absorb ignores outcomes stamped with an older id (issue #221).
    pub bg_task_generation: u64,
    /// Set after the first `q`/`Esc` on the main list; a second press confirms the quit. Any
    /// other key clears it. Prevents an accidental single-key exit.
    pub quit_armed: bool,
    pub upload: UploadState,
    /// Staged pin/pull gist identity for the next [`Self::enter_diff`] (consumed into
    /// [`DiffState`]). Live identity while Diff/Confirm is open lives on the payload.
    pub staged_diff_gist: Option<(String, String)>,
    /// Navigation history (issue #271): every [`Self::enter`] pushes the screen being left;
    /// every [`Self::leave`] pops back to it. Flat — a screen's own return path never lives on
    /// its payload.
    pub nav_stack: Vec<Screen>,
    /// Staged return target for a screen entered asynchronously (issue #271): set at the
    /// triggering keypress (before the background fetch that will eventually call
    /// [`Self::enter`] completes), since `self.screen` may have changed by the time that
    /// happens. [`Self::enter`] consumes it in place of the live screen when present.
    pub pending_return: Option<Screen>,
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

    /// Presentation cache for the Pins list: sync status + mtimes, filled by
    /// [`Self::refresh_pin_sync_cache`] (impure). The pure view-model builder and Pins paint
    /// read only this cache — never `fs::read` / hash on the draw path (issue #241).
    pub pin_sync_cache: Vec<PinSyncCacheEntry>,
    /// When true, the next Pins draw (or explicit refresh) must rebuild [`Self::pin_sync_cache`].
    pub pin_sync_cache_dirty: bool,
}

/// Generates a `Screen`-payload accessor pair: an immutable getter that searches the current
/// screen, palette origin, and `nav_stack` via [`AppState::find_screen`] (issue #242), and a
/// mutable counterpart doing the same via `find_screen_mut`. The `live_only` variant instead
/// restricts the mutable half to the literal active `Screen::$is` payload only — no
/// palette-origin/`nav_stack` search — for the two payloads (`help`/`config`) whose mutators
/// are intentionally narrower than their getters (issue #300).
macro_rules! screen_payload_accessor {
    (
        $(#[$meta:meta])*
        $get:ident, $get_mut:ident, $is:ident, $ty:ident, $state:ident, $state_mut:ident
    ) => {
        $(#[$meta])*
        pub fn $get(&self) -> Option<&$ty> {
            self.find_screen(Screen::$is).and_then(Screen::$state)
        }

        pub fn $get_mut(&mut self) -> Option<&mut $ty> {
            self.find_screen_mut(Screen::$is).and_then(Screen::$state_mut)
        }
    };
    (
        $(#[$meta:meta])*
        $get:ident, $get_mut:ident, $is:ident, $ty:ident, $state:ident, $state_mut:ident, live_only
    ) => {
        $(#[$meta])*
        pub fn $get(&self) -> Option<&$ty> {
            self.find_screen(Screen::$is).and_then(Screen::$state)
        }

        /// `live_only`: mutable payload only when this is the literal active screen — not via
        /// palette origin or `nav_stack` (issue #300).
        pub fn $get_mut(&mut self) -> Option<&mut $ty> {
            self.screen.$state_mut()
        }
    };
}

impl AppState {
    /// Finds the nearest screen (current, then Palette's origin if the overlay is open, then
    /// `nav_stack` from most to least recent) matching `tag` (issue #271). Replaces the
    /// hand-nested "walk return_screen" chains each payload accessor used to write for itself —
    /// a payload's return path no longer lives on its own struct, so there's nothing left to
    /// walk except the shared history.
    fn find_screen(&self, tag: fn(&Screen) -> bool) -> Option<&Screen> {
        let start = match &self.screen {
            Screen::Palette(p) => &p.origin_screen,
            s => s,
        };
        std::iter::once(start)
            .chain(self.nav_stack.iter().rev())
            .find(|s| tag(s))
    }

    fn find_screen_mut(&mut self, tag: fn(&Screen) -> bool) -> Option<&mut Screen> {
        let start = match &mut self.screen {
            Screen::Palette(p) => &mut p.origin_screen,
            s => s,
        };
        std::iter::once(start)
            .chain(self.nav_stack.iter_mut().rev())
            .find(|s| tag(s))
    }

    /// True when List, Pins, or the Gists manager has an active filter query (issue #319).
    /// Centralizes the 3-way guard that was duplicated across `keys.rs` and `palette.rs`.
    pub fn is_any_filtering(&self) -> bool {
        self.filtering
            || self.pins().is_some_and(|p| p.filtering)
            || self.gist_manager().is_some_and(|g| g.filtering)
    }

    screen_payload_accessor! {
        /// Help payload when Help is active, or when the palette is open over Help (issue #242).
        help, help_mut, is_help, HelpState, help_state, help_state_mut,
        live_only // help_mut doesn't follow nav_stack/palette origin — see macro doc above.
    }

    screen_payload_accessor! {
        /// Config payload when Settings is active, or when the palette is open over Config.
        config, config_mut, is_config, ConfigState, config_state, config_state_mut,
        live_only // config_mut doesn't follow nav_stack/palette origin — see macro doc above.
    }

    screen_payload_accessor! {
        /// Revision payload when Revisions is active, parked on the Diff/Confirm/Preview return
        /// path, or under palette origin (issue #242).
        revision, revision_mut, is_revisions, RevisionState, revision_state, revision_state_mut
    }

    screen_payload_accessor! {
        /// Pins payload when Pins is active, parked on the Diff/Confirm/Preview return path, or
        /// under palette origin (issue #242).
        pins, pins_mut, is_pins, PinsState, pins_state, pins_state_mut
    }

    screen_payload_accessor! {
        /// Gist-manager payload when Gists is active, parked on a detail/revision return path, or
        /// under palette origin (issue #242).
        gist_manager, gist_manager_mut, is_gists, GistsManagerState, gists_state, gists_state_mut
    }

    screen_payload_accessor! {
        /// Detail payload when GistDetail is active, parked on a preview/revision/diff/confirm
        /// return path, palette origin, or help/config return (issue #242).
        detail, detail_mut, is_gist_detail, DetailState, detail_state, detail_state_mut
    }

    /// Palette payload when the overlay is open (issue #242).
    pub fn palette(&self) -> Option<&PaletteState> {
        self.screen.palette_state()
    }

    pub fn palette_mut(&mut self) -> Option<&mut PaletteState> {
        self.screen.palette_state_mut()
    }

    screen_payload_accessor! {
        /// Preview payload when Preview is active, or under palette origin (issue #242).
        preview, preview_mut, is_preview, PreviewState, preview_state, preview_state_mut
    }

    /// Navigate to `new_screen`, remembering how to get back (issue #271). The screen being
    /// left is pushed onto [`Self::nav_stack`] — or, if a [`Self::pending_return`] was staged
    /// (an async entry: the triggering keypress ran before `self.screen` necessarily still
    /// matched what the user meant), that staged screen is pushed instead and the live one is
    /// discarded.
    pub fn enter(&mut self, new_screen: Screen) {
        let live = std::mem::replace(&mut self.screen, new_screen);
        let prev = self.pending_return.take().unwrap_or(live);
        self.nav_stack.push(prev);
    }

    /// Leave the current screen, restoring whatever [`Self::enter`] pushed for it. Falls back
    /// to [`Screen::List`] if the stack is empty (defensive: every real transition goes through
    /// [`Self::enter`], but a few call sites still assign `self.screen` directly).
    pub fn leave(&mut self) {
        self.screen = self.nav_stack.pop().unwrap_or_default();
    }

    /// Enter full-screen content preview with the given body and gist identity.
    pub fn enter_preview(
        &mut self,
        title: String,
        text: String,
        gist_key: Option<(String, String)>,
    ) {
        self.status = None;
        self.enter(Screen::Preview(Box::new(PreviewState {
            title,
            text,
            scroll: 0,
            hscroll: 0,
            gist_key,
        })));
    }

    screen_payload_accessor! {
        /// Diff payload when Diff is active, parked on the Confirm cancel path, or palette origin
        /// (issue #242).
        diff, diff_mut, is_diff, DiffState, diff_state, diff_state_mut
    }

    screen_payload_accessor! {
        /// Confirm payload when Confirm is active, or under palette origin (issue #242).
        confirm, confirm_mut, is_confirm, ConfirmState, confirm_state, confirm_state_mut
    }

    /// Pending action while Confirm is open.
    pub fn pending_action(&self) -> Option<&PendingAction> {
        self.confirm().map(|c| &c.action)
    }

    /// Background body text for Diff or Confirm (not Preview).
    pub fn diff_body_text(&self) -> &str {
        match &self.screen {
            Screen::Diff(d) => d.text.as_str(),
            Screen::Confirm(c) => c.text.as_str(),
            _ => "",
        }
    }

    pub fn diff_body_text_mut(&mut self) -> Option<&mut String> {
        match &mut self.screen {
            Screen::Diff(d) => Some(&mut d.text),
            Screen::Confirm(c) => Some(&mut c.text),
            _ => None,
        }
    }

    pub fn diff_scroll(&self) -> u16 {
        match &self.screen {
            Screen::Diff(d) => d.scroll,
            Screen::Confirm(c) => c.scroll,
            _ => 0,
        }
    }

    pub fn diff_hscroll(&self) -> u16 {
        match &self.screen {
            Screen::Diff(d) => d.hscroll,
            Screen::Confirm(c) => c.hscroll,
            _ => 0,
        }
    }

    /// Open Confirm with background `text`. Cancel path is whatever [`Self::enter`] captures —
    /// the staged [`Self::pending_return`] if this follows an async fetch, else the live screen.
    pub fn enter_confirm(&mut self, action: PendingAction, text: String) {
        self.status = None;
        self.enter(Screen::Confirm(Box::new(ConfirmState {
            action,
            text,
            scroll: 0,
            hscroll: 0,
        })));
    }

    /// Open Confirm from the active Diff (download overwrite gate). Cancel restores Diff.
    pub fn enter_confirm_from_diff(&mut self, action: PendingAction) {
        let Screen::Diff(d) = std::mem::replace(&mut self.screen, Screen::List) else {
            return;
        };
        self.status = None;
        let (text, scroll, hscroll) = (d.text.clone(), d.scroll, d.hscroll);
        // Park full Diff (pairing + identity) so cancel/download IO still have it; not a
        // `pending_return` case (synchronous — `d` came straight off `self.screen`), so push it
        // directly rather than going through `enter()`.
        self.nav_stack.push(Screen::Diff(d));
        self.screen = Screen::Confirm(Box::new(ConfirmState {
            action,
            text,
            scroll,
            hscroll,
        }));
    }

    /// Leave Confirm for a Download cancel: restore the parked Diff payload.
    pub fn cancel_confirm_to_diff(&mut self) {
        let Screen::Confirm(c) = std::mem::replace(&mut self.screen, Screen::List) else {
            return;
        };
        self.leave();
        if let Screen::Diff(ref mut d) = self.screen {
            d.text = c.text;
            d.scroll = c.scroll;
            d.hscroll = c.hscroll;
        }
    }

    /// Leave Confirm restoring whatever [`Self::enter`] parked for it (upload/compact/etc.).
    pub fn cancel_confirm(&mut self) {
        self.leave();
    }

    /// Leave Confirm after executing a whole-gist delete. Like [`Self::cancel_confirm`], but
    /// pops once more if that lands on `GistDetail` — the just-deleted gist's own (now-stale)
    /// detail view, since `GistDetail`'s delete key only ever targets itself.
    pub fn cancel_confirm_after_delete(&mut self) {
        self.cancel_confirm();
        if self.screen.is_gist_detail() {
            self.leave();
        }
    }

    pub fn upload_local_path(&self) -> Option<std::path::PathBuf> {
        match self.pending_action() {
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
        if let Some(body) = self.diff_body_text_mut() {
            *body = diff;
        }
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
        let context_matches = self.screen.is_confirm()
            && self.upload.watching
            && matches!(
                self.pending_action(),
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
        let starred = self
            .gist_manager()
            .is_some_and(|g| g.type_filter.uses_starred_source());
        if starred {
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

    /// All gists collapsed to one entry each (raw, unfiltered) from the owned list.
    pub fn gist_groups(&self) -> Vec<GistGroup> {
        group_gists(&self.gists)
    }

    /// The gist-level view's rows after the visibility filter, text filter, and sort
    /// are applied. This is the single source of truth for navigation, selection, and
    /// rendering in `Screen::Gists`.
    pub fn visible_gist_groups(&self) -> Vec<GistGroup> {
        let gm = self.gist_manager().cloned().unwrap_or_default();
        let query = gm.filter_query.to_lowercase();
        let mut groups: Vec<GistGroup> = group_gists(self.manager_gist_source())
            .into_iter()
            .filter(|g| gm.type_filter.matches_group(g))
            .filter(|g| {
                query.is_empty()
                    || g.description.to_lowercase().contains(&query)
                    || g.id.to_lowercase().contains(&query)
            })
            .collect();
        match gm.sort {
            GistGroupSort::Updated => groups.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
            GistGroupSort::Created => groups.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        }
        groups
    }

    /// The gist highlighted in the gist-level view.
    pub fn selected_group(&self) -> Option<GistGroup> {
        let idx = self.gist_manager().map(|g| g.cursor.index).unwrap_or(0);
        self.visible_gist_groups().into_iter().nth(idx)
    }

    /// Highest horizontal-scroll offset for the gist-level view, based on its selected
    /// visible row (mirrors `focused_hscroll_max` for the main panes; issue #341).
    fn gists_hscroll_max(&self) -> u16 {
        let sort = self.gist_manager().map(|g| g.sort).unwrap_or_default();
        let idx = self.gist_manager().map(|g| g.cursor.index).unwrap_or(0);
        self.visible_gist_groups()
            .get(idx)
            .map(|g| {
                gist_group_row_label(
                    g,
                    unix_now(),
                    sort,
                    (
                        self.gist_comment_counts.get(&g.id).copied().unwrap_or(0),
                        self.gist_star_counts.get(&g.id).copied().unwrap_or(0),
                        self.gist_fork_counts.get(&g.id).copied().unwrap_or(0),
                    ),
                    self.gist_is_starred(&g.id),
                    self.current_user_login.as_deref(),
                )
            })
            .map(|t| hscroll_max_for_text(&t))
            .unwrap_or(0)
    }

    /// Highest horizontal-scroll offset for the Pins screen, bounded by the selected
    /// row's displayed local path (the only variable-length, overflow-prone field).
    /// Pure helper modeled on `gists_hscroll_max`.
    fn pins_hscroll_max(&self) -> u16 {
        let idx = self.pins().map(|p| p.cursor.index).unwrap_or(0);
        self.visible_pin_indices()
            .get(idx)
            .and_then(|&i| self.pinned.get(i))
            .map(|m| hscroll_max_for_text(&crate::config::display_path(&m.local_path)))
            .unwrap_or(0)
    }

    /// Indices into `self.pinned` that match the Pins-screen text filter, in sort order.
    /// Empty query → every index. Matched against the cwd/home-shortened local path plus
    /// the gist filename (the meaningful, visible parts of the row).
    pub fn visible_pin_indices(&self) -> Vec<usize> {
        let query = self
            .pins()
            .map(|p| p.filter_query.to_lowercase())
            .unwrap_or_default();
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
        match self.pins().map(|p| p.sort).unwrap_or_default() {
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
        let idx = self.pins().map(|p| p.cursor.index).unwrap_or(0);
        self.visible_pin_indices().get(idx).copied()
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
    /// Open `Screen::Revisions` for the gist on the current screen (List, GistDetail, or Gists).
    /// Returns false when no gist is selected or the gist has no files.
    pub fn open_revisions(&mut self) -> bool {
        // Snapshot the selected gist once for the List path; it feeds both `gist_id` and
        // `target_file` below, avoiding a second `ranked_gists` recompute (perf-1, #154).
        let selected_list_gist = match self.screen {
            Screen::List => self.selected_gist(),
            _ => None,
        };
        let gist_id = match &self.screen {
            Screen::List => selected_list_gist.as_ref().map(|g| g.file.gist_id.clone()),
            Screen::GistDetail(d) => d.gist_id.clone(),
            Screen::Gists(_) => self.selected_group().map(|g| g.id.clone()),
            _ => None,
        };
        let Some(gist_id) = gist_id else {
            return false;
        };
        let filenames = self.gist_filenames(&gist_id);
        let target_file = match &self.screen {
            Screen::List => selected_list_gist
                .as_ref()
                .map(|g| g.file.filename.clone())
                .filter(|f| filenames.iter().any(|name| name == f)),
            Screen::GistDetail(d) => filenames
                .into_iter()
                .nth(d.file_cursor)
                .or_else(|| self.gist_filenames(&gist_id).first().cloned()),
            Screen::Gists(_) => filenames.first().cloned(),
            _ => None,
        };
        let Some(target_file) = target_file else {
            return false;
        };
        self.enter(Screen::Revisions(Box::new(RevisionState {
            gist_id: Some(gist_id),
            target_file,
            index: 0,
            hscroll: 0,
            entries: None,
            fetch_error: None,
        })));
        true
    }

    /// True when the diff view supports local↔gist download/upload (`d`/`u`). Revision-history
    /// diffs (returning to `Screen::Revisions`) are read-only comparisons. Checks the top of
    /// `nav_stack` directly (not [`Self::diff`]'s deep search) — this only makes sense while
    /// Diff is the live screen, so it's the immediate parent, not wherever else Diff might be
    /// parked.
    pub fn diff_allows_sync(&self) -> bool {
        !self.nav_stack.last().is_some_and(Screen::is_revisions)
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
        match &self.screen {
            Screen::Gists(_) => self.selected_group().map(|g| g.id),
            Screen::GistDetail(d) => d.gist_id.clone(),
            _ => self
                .detail()
                .and_then(|d| d.gist_id.clone())
                .or_else(|| self.selected_gist().map(|g| g.file.gist_id)),
        }
    }

    /// Upload intent shared by the list and the diff screen: requires a selected local file
    /// and gist, then branches on whether the gist already holds a file of the local name
    /// (case C: preview + confirm overwrite) or not (case B: add directly).
    /// True when we're in the diff screen launched from a Pins context (pin diff or pin pull).
    /// In this state DiffState holds the pin's local path and gist identity, so upload/download
    /// should use those instead of the Files-view selection which may point elsewhere.
    pub fn is_pin_diff_context(&self) -> bool {
        self.diff()
            .is_some_and(|d| !d.local_path.as_os_str().is_empty() && d.gist_id.is_some())
    }

    fn upload_intent(&mut self) -> KeyOutcome {
        if let Some(gist) = self.selected_gist() {
            if self.block_if_foreign_gist(&gist.file.gist_id, false) {
                return KeyOutcome::None;
            }
        }
        if self.is_pin_diff_context() {
            let local_path = self.preview_local();
            let Some(local_filename) = local_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
            else {
                self.status = Some("local file has no name".into());
                return KeyOutcome::None;
            };
            let gist_id = self.download_gist_id().unwrap_or_default().to_string();
            let raw_url = self.gist_file_raw_url(&gist_id, &local_filename);
            let has_same_name = self
                .gists
                .iter()
                .any(|g| g.gist_id == gist_id && g.filename == local_filename);
            return if has_same_name {
                KeyOutcome::UploadPreview {
                    local_path,
                    file: GistFileRef::new(gist_id, local_filename, raw_url),
                    from_pin_diff: true,
                }
            } else {
                KeyOutcome::UploadAdd {
                    local_path,
                    gist_id,
                    filename: local_filename,
                }
            };
        }
        let (Some(local), Some(gist)) = self.selected_pair() else {
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
        let local_path = local.path.clone();
        let gist_id = gist.file.gist_id.clone();
        let raw_url = gist.file.raw_url.clone();
        let has_same_name = self
            .gists
            .iter()
            .any(|g| g.gist_id == gist_id && g.filename == local_filename);
        if has_same_name {
            KeyOutcome::UploadPreview {
                local_path,
                file: GistFileRef::new(gist_id, local_filename, raw_url),
                from_pin_diff: false,
            }
        } else {
            KeyOutcome::UploadAdd {
                local_path,
                gist_id,
                filename: local_filename,
            }
        }
    }

    /// Highest horizontal-scroll offset for the focused pane's **selected** row
    /// (viewport width is unknown to the pure key logic, mirroring the diff scroll cap).
    ///
    /// Must measure the **same** string the list paint path draws (`gist_row_display` /
    /// local label + pin mark), not a star-less or mark-less variant — otherwise starred
    /// or pinned rows cannot scroll far enough to reveal their trailing characters (#247).
    /// Capping to the selected row (not the longest in the pane) is issue #341.
    fn focused_hscroll_max(&self) -> u16 {
        let (visible_locals, ranked) = self.list_pane_snapshots();
        let text = match self.focus {
            FocusPane::Local => visible_locals
                .get(self.local_index)
                .map(|r| marked_row_text(local_row_label(&r.candidate.path, &self.cwd), r.mark)),
            FocusPane::Gist => ranked
                .get(self.gist_index)
                .map(|g| marked_row_text(gist_row_display(g, self.gist_view, self), g.mark)),
        };
        text.map(|t| hscroll_max_for_text(&t)).unwrap_or(0)
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
        let (gist_id, gist_filename) = match self.staged_diff_gist.take() {
            Some((id, name)) => (Some(id), Some(name)),
            None => (None, None),
        };
        self.status = None;
        self.enter(Screen::Diff(Box::new(DiffState {
            text: diff_text,
            scroll: 0,
            hscroll: 0,
            remote_content: remote,
            local_path: local,
            download_target: target,
            identical: false,
            gist_id,
            gist_filename,
        })));
    }

    /// True while a Diff payload is live (active Diff or parked under Confirm).
    pub fn diff_previewed(&self) -> bool {
        self.diff().is_some()
    }

    /// Local/remote content compare equal under the open Diff (or parked Diff).
    pub fn diff_identical(&self) -> bool {
        self.diff().is_some_and(|d| d.identical)
    }

    pub fn download_target(&self) -> PathBuf {
        self.diff()
            .map(|d| d.download_target.clone())
            .unwrap_or_default()
    }

    pub fn preview_local(&self) -> PathBuf {
        self.diff()
            .map(|d| d.local_path.clone())
            .unwrap_or_default()
    }

    pub fn preview_remote(&self) -> &str {
        self.diff().map(|d| d.remote_content.as_str()).unwrap_or("")
    }

    pub fn download_gist_id(&self) -> Option<&str> {
        self.diff().and_then(|d| d.gist_id.as_deref())
    }

    pub fn download_gist_filename(&self) -> Option<&str> {
        self.diff().and_then(|d| d.gist_filename.as_deref())
    }

    pub fn back_to_list(&mut self) {
        self.screen = Screen::List;
        // Diff pairing identity lives on the payload; leaving drops it.
        self.staged_diff_gist = None;
        self.pending_return = None;
        // A hard reset, not a `leave()` — nothing on the stack corresponds to List, so discard
        // it rather than let stale entries resurface on some later, unrelated `leave()`.
        self.nav_stack.clear();
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some(message.into());
    }

    pub fn scroll_diff_down(&mut self) {
        let max = self.diff_vscroll_max();
        if let Some(p) = self.preview_mut() {
            if p.scroll < max {
                p.scroll += 1;
            }
            return;
        }
        match &mut self.screen {
            Screen::Diff(d) if d.scroll < max => d.scroll += 1,
            Screen::Confirm(c) if c.scroll < max => c.scroll += 1,
            _ => {}
        }
    }

    /// Bottom clamp for the diff/preview vertical scroll: the last addressable line index.
    fn diff_vscroll_max(&self) -> u16 {
        let text = self
            .preview()
            .map(|p| p.text.as_str())
            .unwrap_or_else(|| self.diff_body_text());
        text.lines()
            .count()
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16
    }

    pub fn scroll_diff_up(&mut self) {
        if let Some(p) = self.preview_mut() {
            p.scroll = p.scroll.saturating_sub(1);
            return;
        }
        match &mut self.screen {
            Screen::Diff(d) => d.scroll = d.scroll.saturating_sub(1),
            Screen::Confirm(c) => c.scroll = c.scroll.saturating_sub(1),
            _ => {}
        }
    }

    /// Page the diff/preview down by `lines`, clamped to the same bottom as `scroll_diff_down`.
    pub fn scroll_diff_page_down(&mut self, lines: u16) {
        let max = self.diff_vscroll_max();
        if let Some(p) = self.preview_mut() {
            p.scroll = p.scroll.saturating_add(lines).min(max);
            return;
        }
        match &mut self.screen {
            Screen::Diff(d) => d.scroll = d.scroll.saturating_add(lines).min(max),
            Screen::Confirm(c) => c.scroll = c.scroll.saturating_add(lines).min(max),
            _ => {}
        }
    }

    /// Page the diff/preview up by `lines`, saturating at the top.
    pub fn scroll_diff_page_up(&mut self, lines: u16) {
        if let Some(p) = self.preview_mut() {
            p.scroll = p.scroll.saturating_sub(lines);
            return;
        }
        match &mut self.screen {
            Screen::Diff(d) => d.scroll = d.scroll.saturating_sub(lines),
            Screen::Confirm(c) => c.scroll = c.scroll.saturating_sub(lines),
            _ => {}
        }
    }

    pub fn scroll_diff_right(&mut self) {
        if let Some(p) = self.preview_mut() {
            let max = hscroll_max_among(p.text.lines());
            if p.hscroll < max {
                p.hscroll += 1;
            }
            return;
        }
        let max = hscroll_max_among(self.diff_body_text().lines());
        match &mut self.screen {
            Screen::Diff(d) if d.hscroll < max => d.hscroll += 1,
            Screen::Confirm(c) if c.hscroll < max => c.hscroll += 1,
            _ => {}
        }
    }

    pub fn scroll_diff_left(&mut self) {
        if let Some(p) = self.preview_mut() {
            p.hscroll = p.hscroll.saturating_sub(1);
            return;
        }
        match &mut self.screen {
            Screen::Diff(d) => d.hscroll = d.hscroll.saturating_sub(1),
            Screen::Confirm(c) => c.hscroll = c.hscroll.saturating_sub(1),
            _ => {}
        }
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
        gist_view: GistView::Description,
        gist_type_filter: GistTypeFilter::All,
        gist_sort: GistSort::Match,
        local_sort: LocalSort::Match,
        filtering: false,
        filter_query: TextInput::default(),
        local_filter_query: TextInput::default(),
        diff_wrap: false,
        diff_context: 3,
        diff_show_full: false,
        ignore_trailing_newline: true,
        cwd: PathBuf::from("."),
        status: None,
        loading: false,
        preview_wrap: false,
        syntax_highlight: true,
        config_mouse: true,
        mouse_enabled: true,
        no_mouse_cli: false,
        config_check_updates: true,
        update_check_enabled: true,
        no_update_check_cli: false,
        update_available: None,
        install_method: crate::upgrade::InstallMethod::Standalone,
        // Bound the in-memory preview cache so browsing many/large gists can't grow unbounded;
        // evicted entries are simply re-fetched on demand.
        gist_content_cache: crate::lru::LruCache::new(64),
        local_recursive: false,
        skip_dirs: crate::config::AppConfig::default().skip_dirs,
        scan_depth: crate::config::AppConfig::default().scan_depth,
        local_scanning: false,
        local_scan_generation: 0,

        editing_description: false,
        description_input: TextInput::default(),
        bg_task_msg: None,
        bg_task_generation: 0,
        quit_armed: false,
        upload: UploadState::default(),
        staged_diff_gist: None,
        nav_stack: Vec::new(),
        pending_return: None,
        spinner_frame: 0,
        gist_comment_counts: std::collections::HashMap::new(),
        gist_fork_counts: std::collections::HashMap::new(),
        gist_star_counts: std::collections::HashMap::new(),
        theme_choice: crate::config::ThemeChoice::Dark,
        theme: Theme::DARK,
        pin_sync_cache: Vec::new(),
        pin_sync_cache_dirty: true,
    }
}

pub fn load_startup_state(no_mouse: bool, no_update_check: bool) -> Result<AppState> {
    let mut state = initial_state();
    let config_path = crate::config::config_path()?;
    let config = crate::config::load_config(&config_path)?;
    let cwd = std::env::current_dir()?;

    state.pinned = config.pinned;
    state.mark_pin_sync_cache_dirty();
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
    // `--no-mouse` / `--no-update-check` force off; no flag forces on (edit config instead).
    state.mouse_enabled = config.mouse && !no_mouse;
    state.config_check_updates = config.check_updates;
    state.no_update_check_cli = no_update_check;
    state.update_check_enabled = config.check_updates && !no_update_check;
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

/// Draw a centered, bordered box over the current frame, sized to fit `body` (clamped to
/// the frame) and wiped clean with `Clear` so whatever is behind it doesn't bleed through.
/// This is the shared "centered window" primitive behind both the loading overlay and the
/// confirm prompt.
mod highlight;
mod palette;
#[cfg(test)]
use palette::PaletteItem;
use palette::{PaletteMode, PaletteState};

mod render;
use render::*;
mod screens;
pub use screens::detail::InitialComments;
mod text;
use text::{hscroll_max_among, hscroll_max_for_text, local_row_label};
mod bg;
mod dispatch;
mod keys;
mod list_cursor;
pub(crate) use list_cursor::ListCursor;
mod list_ranking;
mod pin_sync;
pub use pin_sync::PinSyncCacheEntry;
mod run_loop;
use run_loop::run_loop;
mod text_input;
pub use text_input::{EditResult, TextInput};
mod theme;
pub use theme::Theme;
mod view_model;
pub(crate) use view_model::{build_view_model, gist_row_display, ScreenVm};
// Only exercised by tests.rs's `use super::*` — view_model.rs's own build_*_vm functions call
// these directly without needing the re-export.
#[cfg(test)]
pub(crate) use screens::confirm::confirm_modal_style;
#[cfg(test)]
pub(crate) use screens::detail::detail_focus_tab;
#[cfg(test)]
pub(crate) use screens::diff::{diff_footer, diff_title};
#[cfg(test)]
pub(crate) use screens::pins::{pin_row_label, PinLabelParams};
#[cfg(test)]
pub(crate) use view_model::confirm_prompt;

#[cfg(test)]
mod tests;
