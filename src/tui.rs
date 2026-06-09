use crate::domain::{group_gists, GistFile, GistGroup, LocalCandidate, PinnedMapping};
use crate::ranking::{rank_gist_files, RankedGistFile};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, List, ListItem, ListState, Padding, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
    Frame, Terminal,
};
use similar::{ChangeTag, TextDiff};
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
    ExecuteDelete,
    ExecuteRemoveFile,
    ApplyDescription,
    RefreshLocals,
    RefreshPreview,
    UnpinAtPin,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub locals: Vec<LocalCandidate>,
    pub gists: Vec<GistFile>,
    pub pinned: Vec<PinnedMapping>,
    pub focus: FocusPane,
    pub local_index: usize,
    pub gist_index: usize,
    pub local_hscroll: u16,
    pub gist_hscroll: u16,
    pub screen: Screen,
    pub pending_action: Option<PendingAction>,
    pub gist_view: GistView,
    pub gist_type_filter: GistTypeFilter,
    pub gist_sort: GistSort,
    pub filtering: bool,
    pub filter_query: String,
    pub diff_previewed: bool,
    pub diff_text: String,
    pub diff_scroll: u16,
    pub diff_hscroll: u16,
    pub diff_identical: bool,
    pub preview_remote: String,
    pub preview_local: PathBuf,
    pub download_target: PathBuf,
    pub cwd: PathBuf,
    pub status: Option<String>,
    pub loading: bool,
    pub preview_title: String,
    pub preview_gist_key: Option<(String, String)>,
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
}

impl AppState {
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
        // No local selected (e.g. an empty directory): list every gist unranked so the
        // user can still preview and download into the cwd.
        let mut ranked = match self.locals.get(self.local_index) {
            Some(local) => rank_gist_files(&local.path, &gists, &self.pinned),
            None => gists
                .into_iter()
                .map(|file| RankedGistFile {
                    file,
                    score: 0,
                    reasons: Vec::new(),
                })
                .collect(),
        };
        self.gist_sort.apply(&mut ranked);
        ranked
    }

    pub fn selected_local(&self) -> Option<&LocalCandidate> {
        self.locals.get(self.local_index)
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
            .map(|g| gist_group_row_label(g).chars().count())
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

    /// Upload intent shared by the list and the diff screen: requires a selected local file
    /// and gist, then branches on whether the gist already holds a file of the local name
    /// (case C: preview + confirm overwrite) or not (case B: add directly).
    fn upload_intent(&mut self) -> KeyOutcome {
        let (Some(local), Some(gist)) = (self.selected_local().cloned(), self.selected_gist())
        else {
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
        }
    }

    fn handle_key_help(&mut self, _code: KeyCode) -> KeyOutcome {
        // Any key (including q) just closes the help overlay back to the list.
        self.screen = Screen::List;
        KeyOutcome::None
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
            KeyCode::Char('x') | KeyCode::Char('d') if !self.pinned.is_empty() => {
                return KeyOutcome::UnpinAtPin;
            }
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
            KeyCode::Char('o') if self.gists_index < groups.len() => {
                return KeyOutcome::OpenBrowser
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

    fn handle_key_preview(&mut self, code: KeyCode) -> KeyOutcome {
        match code {
            // In the preview, q and Esc both return to the list (no accidental app exit).
            KeyCode::Char('q') | KeyCode::Esc => {
                self.screen = Screen::List;
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
        match code {
            // On the main list both q and Esc exit the app.
            KeyCode::Char('q') | KeyCode::Esc => return KeyOutcome::Quit,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    FocusPane::Local => FocusPane::Gist,
                    FocusPane::Gist => FocusPane::Local,
                };
            }
            KeyCode::Down => match self.focus {
                FocusPane::Local if self.local_index + 1 < self.locals.len() => {
                    self.local_index += 1;
                    self.gist_index = 0;
                    self.local_hscroll = 0;
                    self.gist_hscroll = 0;
                }
                FocusPane::Gist if self.gist_index + 1 < self.ranked_gists().len() => {
                    self.gist_index += 1;
                    self.gist_hscroll = 0;
                }
                _ => {}
            },
            KeyCode::Up => match self.focus {
                FocusPane::Local if self.local_index > 0 => {
                    self.local_index -= 1;
                    self.gist_index = 0;
                    self.local_hscroll = 0;
                    self.gist_hscroll = 0;
                }
                FocusPane::Gist if self.gist_index > 0 => {
                    self.gist_index -= 1;
                    self.gist_hscroll = 0;
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
                // Cycle the gist sort: match -> name -> recent -> match.
                self.gist_sort = self.gist_sort.next();
                self.gist_index = 0;
                self.gist_hscroll = 0;
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
                let (Some(local), Some(gist)) =
                    (self.selected_local().cloned(), self.selected_gist())
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
                let Some(local) = self.selected_local().cloned() else {
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
            // In the diff, q and Esc both return to the list (no accidental app exit).
            KeyCode::Char('q') | KeyCode::Esc => self.back_to_list(),
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
            Some(PendingAction::Upload { .. }) => match code {
                KeyCode::Char('y') => return KeyOutcome::Upload,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.pending_action = None;
                    self.screen = Screen::List;
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

pub fn initial_state() -> AppState {
    AppState {
        locals: Vec::new(),
        gists: Vec::new(),
        pinned: Vec::new(),
        focus: FocusPane::Local,
        local_index: 0,
        gist_index: 0,
        local_hscroll: 0,
        gist_hscroll: 0,
        screen: Screen::List,
        pending_action: None,
        gist_view: GistView::Description,
        gist_type_filter: GistTypeFilter::All,
        gist_sort: GistSort::Match,
        filtering: false,
        filter_query: String::new(),
        diff_previewed: false,
        diff_text: String::new(),
        diff_scroll: 0,
        diff_hscroll: 0,
        diff_identical: false,
        preview_remote: String::new(),
        preview_local: PathBuf::new(),
        download_target: PathBuf::new(),
        cwd: PathBuf::from("."),
        status: None,
        loading: false,
        preview_title: String::new(),
        preview_gist_key: None,
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
    let gist_rx = spawn_gist_fetch();
    let mut local_rx: Option<std::sync::mpsc::Receiver<Vec<LocalCandidate>>> = None;

    loop {
        terminal.draw(|frame| render(frame, &state))?;

        // Absorb the background gist list once it arrives.
        if state.loading {
            if let Ok(gists) = gist_rx.try_recv() {
                cache_gists(&gists);
                state.gists = gists;
                state.loading = false;
                if state.gist_index >= state.ranked_gists().len() {
                    state.gist_index = 0;
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

        // Poll so the loop also wakes to check the background fetches, not only on input.
        if !event::poll(std::time::Duration::from_millis(150))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            match state.handle_key(key.code) {
                KeyOutcome::Quit => break,
                KeyOutcome::PreviewDiff => {
                    fetch_then(terminal, &mut state, "Loading diff…", preview_diff)?
                }
                KeyOutcome::Download => download(&mut state),
                KeyOutcome::DownloadGist => {
                    fetch_then(terminal, &mut state, "Downloading…", download_selected)?
                }
                KeyOutcome::Pin => pin_selected(&mut state),
                KeyOutcome::Unpin => unpin_selected(&mut state),
                KeyOutcome::UploadAdd => {
                    fetch_then(terminal, &mut state, "Uploading…", upload_add)?
                }
                KeyOutcome::UploadPreview => {
                    fetch_then(terminal, &mut state, "Loading diff…", upload_preview)?
                }
                KeyOutcome::Upload => {
                    fetch_then(terminal, &mut state, "Uploading…", upload_replace)?
                }
                KeyOutcome::Create(public) => {
                    draw_loading(terminal, &mut state, "Creating gist…")?;
                    create_gist(&mut state, public);
                }
                KeyOutcome::PreviewContent => {
                    fetch_then(terminal, &mut state, "Loading preview…", preview_content)?
                }
                KeyOutcome::RefreshPreview => fetch_then(
                    terminal,
                    &mut state,
                    "Loading preview…",
                    refresh_preview_content,
                )?,
                KeyOutcome::OpenBrowser => open_browser(&mut state),
                KeyOutcome::EditLocal => edit_local(terminal, &mut state)?,
                KeyOutcome::ExecuteDelete => {
                    draw_loading(terminal, &mut state, "Deleting gist…")?;
                    delete_gist(&mut state);
                }
                KeyOutcome::ExecuteRemoveFile => {
                    draw_loading(terminal, &mut state, "Removing file…")?;
                    remove_file(&mut state);
                }
                KeyOutcome::ApplyDescription => {
                    draw_loading(terminal, &mut state, "Updating description…")?;
                    apply_description(&mut state);
                }
                KeyOutcome::RefreshLocals => {
                    draw_loading(terminal, &mut state, "Scanning files…")?;
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
                KeyOutcome::None => {}
            }
        }
    }

    Ok(())
}

/// Draws a "Loading…" frame, then runs a blocking `gh` action. The action overwrites the
/// status with its result, which the next loop iteration draws.
fn fetch_then(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    msg: &str,
    action: fn(&mut AppState),
) -> Result<()> {
    draw_loading(terminal, state, msg)?;
    action(state);
    Ok(())
}

fn draw_loading(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    msg: &str,
) -> Result<()> {
    state.set_status(msg);
    // A blocking `gh` call follows, so this is the last frame drawn until it returns.
    // Render a dedicated centered overlay rather than the current screen, because the
    // Confirm and Gists screens don't surface `status` — without it those flows would
    // look frozen during the call.
    terminal.draw(|frame| render_loading_overlay(frame, msg))?;
    Ok(())
}

/// A centered "Working…" box shown while a blocking `gh` action runs. Animating a
/// spinner would require running the action off-thread (see issue #11); under the
/// current synchronous model this is a single static frame.
fn render_loading_overlay(frame: &mut Frame, msg: &str) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(45),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(frame.area());
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
        ])
        .split(rows[1]);
    frame.render_widget(
        Paragraph::new(format!("⏳ {msg}")).block(
            Block::default()
                .title("Working…")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        ),
        cols[1],
    );
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

fn preview_diff(state: &mut AppState) {
    let Some(ranked) = state.selected_gist() else {
        return;
    };
    // A local file may not be selected (empty cwd); diff against empty content then.
    let local_path = state.selected_local().map(|local| local.path.clone());
    let gist = ranked.file;
    match crate::gh::fetch_gist_file_content(&gist.gist_id, &gist.filename) {
        Ok(remote) => {
            let local_content = local_path
                .as_ref()
                .map(|path| std::fs::read_to_string(path).unwrap_or_default())
                .unwrap_or_default();
            let (local_label, gist_label) = diff_labels(local_path.as_deref(), &gist);
            let diff =
                crate::diff::unified_diff(&local_label, &local_content, &gist_label, &remote);
            // Download saves the gist into the current working directory under the
            // gist's own filename, leaving the compared local file untouched.
            let target = state.cwd.join(&gist.filename);
            let identical = local_content == remote;
            state.enter_diff(diff, remote, local_path.unwrap_or_default(), target);
            state.diff_identical = identical;
        }
        Err(error) => state.set_status(format!("fetch failed: {error}")),
    }
}

fn open_browser(state: &mut AppState) {
    // The gist-level view selects by gist; the main list selects by file row.
    let gist_id = match state.screen {
        Screen::Gists => state.selected_group().map(|g| g.id),
        _ => state.selected_gist().map(|g| g.file.gist_id),
    };
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
    let Some(local) = state.selected_local().cloned() else {
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

fn preview_content(state: &mut AppState) {
    let Some(gist) = state.selected_gist() else {
        return;
    };
    let key = (gist.file.gist_id.clone(), gist.file.filename.clone());
    let content = match state.gist_content_cache.get(&key) {
        Some(cached) => cached.clone(),
        None => match crate::gh::fetch_gist_file_content(&gist.file.gist_id, &gist.file.filename) {
            Ok(content) => {
                state
                    .gist_content_cache
                    .insert(key.clone(), content.clone());
                content
            }
            Err(error) => {
                state.set_status(format!("fetch failed: {error}"));
                return;
            }
        },
    };
    state.preview_title = format!("Preview: {} / {}", gist.file.gist_id, gist.file.filename);
    state.preview_gist_key = Some(key);
    state.diff_text = content;
    state.diff_scroll = 0;
    state.diff_hscroll = 0;
    state.status = None;
    state.screen = Screen::Preview;
}

fn refresh_preview_content(state: &mut AppState) {
    if let Some(key) = state.preview_gist_key.clone() {
        state.gist_content_cache.remove(&key);
    }
    preview_content(state);
}

fn download_selected(state: &mut AppState) {
    let Some(ranked) = state.selected_gist() else {
        return;
    };
    let gist = ranked.file;
    let target = state.cwd.join(&gist.filename);
    match crate::gh::fetch_gist_file_content(&gist.gist_id, &gist.filename) {
        Ok(remote) => {
            if target.exists() {
                // A same-named file already exists: show its diff and require a y/n
                // overwrite confirmation before writing.
                let local_content = std::fs::read_to_string(&target).unwrap_or_default();
                let (local_label, gist_label) = diff_labels(Some(&target), &gist);
                let diff =
                    crate::diff::unified_diff(&local_label, &local_content, &gist_label, &remote);
                let identical = local_content == remote;
                state.enter_diff(diff, remote, target.clone(), target);
                state.diff_identical = identical;
            } else {
                // No collision: download straight into the cwd without forcing a diff.
                match crate::actions::execute_download(&target, &remote, false) {
                    Ok(()) => {
                        state.set_status(format!("Downloaded {}", target.display()));
                        refresh_locals(state);
                    }
                    Err(error) => state.set_status(format!("download failed: {error}")),
                }
            }
        }
        Err(error) => state.set_status(format!("fetch failed: {error}")),
    }
}

fn download(state: &mut AppState) {
    let target = state.download_target.clone();
    let content = state.preview_remote.clone();
    match crate::actions::execute_download(&target, &content, true) {
        Ok(()) => {
            state.set_status(format!("Downloaded {}", target.display()));
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

fn pin_selected(state: &mut AppState) {
    let (Some(local), Some(gist)) = (state.selected_local().cloned(), state.selected_gist()) else {
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
    let Some(local) = state.selected_local().cloned() else {
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

fn upload_add(state: &mut AppState) {
    let (Some(local), Some(gist)) = (state.selected_local().cloned(), state.selected_gist()) else {
        return;
    };
    let plan = crate::actions::upload_add_command(&local.path, &gist.file.gist_id);
    match crate::actions::execute_command(&plan) {
        Ok(_) => {
            if let Some(filename) = upload_local_filename(&local.path) {
                state
                    .gist_content_cache
                    .remove(&(gist.file.gist_id.clone(), filename));
            }
            state.set_status(format!(
                "Uploaded {} to gist {}",
                local.path.display(),
                gist.file.gist_id
            ));
            state.back_to_list();
            refresh_gists(state);
        }
        Err(error) => state.set_status(format!("upload failed: {error}")),
    }
}

fn upload_preview(state: &mut AppState) {
    let (Some(local), Some(gist)) = (state.selected_local().cloned(), state.selected_gist()) else {
        return;
    };
    let Some(filename) = upload_local_filename(&local.path) else {
        state.set_status("local file has no name");
        return;
    };
    match crate::gh::fetch_gist_file_content(&gist.file.gist_id, &filename) {
        Ok(remote) => {
            let local_content = std::fs::read_to_string(&local.path).unwrap_or_default();
            let (local_label, gist_label) = diff_labels(Some(&local.path), &gist.file);
            let diff =
                crate::diff::unified_diff(&gist_label, &remote, &local_label, &local_content);
            state.diff_text = diff;
            state.diff_scroll = 0;
            state.diff_hscroll = 0;
            state.pending_action = Some(PendingAction::Upload {
                gist_id: gist.file.gist_id.clone(),
                filename,
                local_path: local.path.clone(),
            });
            state.status = None;
            state.screen = Screen::Confirm;
        }
        Err(error) => state.set_status(format!("fetch failed: {error}")),
    }
}

fn upload_replace(state: &mut AppState) {
    let Some(PendingAction::Upload {
        gist_id,
        filename,
        local_path,
    }) = state.pending_action.clone()
    else {
        return;
    };
    let target = GistFile {
        gist_id: gist_id.clone(),
        description: String::new(),
        filename: filename.clone(),
        public: false,
        updated_at: String::new(),
        created_at: String::new(),
    };
    let plan = crate::actions::upload_command(&local_path, &target);
    match crate::actions::execute_command(&plan) {
        Ok(_) => {
            state
                .gist_content_cache
                .remove(&(gist_id.clone(), filename.clone()));
            state.set_status(format!("Uploaded {} to gist {}", filename, gist_id));
            state.back_to_list();
            refresh_gists(state);
        }
        Err(error) => {
            state.set_status(format!("upload failed: {error}"));
            state.screen = Screen::Confirm;
        }
    }
}

fn refresh_gists(state: &mut AppState) {
    if let Ok(gists) =
        crate::gh::fetch_gist_list_json().and_then(|raw| crate::gh::parse_gist_list_json(&raw))
    {
        cache_gists(&gists);
        state.gists = gists;
        if state.gist_index >= state.ranked_gists().len() {
            state.gist_index = 0;
        }
    }
}

fn create_gist(state: &mut AppState, public: bool) {
    let Some(PendingAction::Create { local_path }) = state.pending_action.clone() else {
        return;
    };
    let description = state.description_input.clone();
    let plan = crate::actions::create_command(&local_path, public, &description);
    match crate::actions::execute_command(&plan) {
        Ok(_) => {
            let visibility = if public { "public" } else { "secret" };
            state.set_status(format!(
                "Created {} gist from {}",
                visibility,
                local_path.display()
            ));
            state.description_input.clear();
            state.back_to_list();
            refresh_gists(state);
        }
        Err(error) => {
            state.set_status(format!("create failed: {error}"));
            state.screen = Screen::List;
            state.pending_action = None;
            state.description_input.clear();
        }
    }
}

fn remove_file(state: &mut AppState) {
    let Some(PendingAction::RemoveFile {
        gist_id, filename, ..
    }) = state.pending_action.clone()
    else {
        return;
    };
    let plan = crate::actions::remove_file_command(&gist_id, &filename);
    state.back_to_list();
    match crate::actions::execute_command(&plan) {
        Ok(_) => {
            state
                .gist_content_cache
                .remove(&(gist_id.clone(), filename.clone()));
            state.set_status(format!("Removed {filename} from gist {gist_id}"));
            refresh_gists(state);
        }
        Err(error) => state.set_status(format!("remove failed: {error}")),
    }
}

fn apply_description(state: &mut AppState) {
    let Some(group) = state.selected_group() else {
        state.editing_description = false;
        return;
    };
    let gist_id = group.id.clone();
    let description = state.description_input.clone();
    let plan = crate::actions::edit_description_command(&gist_id, &description);
    state.editing_description = false;
    state.description_input.clear();
    match crate::actions::execute_command(&plan) {
        Ok(_) => {
            state.set_status(format!("Updated description for gist {gist_id}"));
            refresh_gists(state);
            let count = state.visible_gist_groups().len();
            if count > 0 && state.gists_index >= count {
                state.gists_index = count - 1;
            }
        }
        Err(error) => state.set_status(format!("description update failed: {error}")),
    }
}

fn delete_gist(state: &mut AppState) {
    let Some(PendingAction::Delete { gist_id, .. }) = state.pending_action.clone() else {
        return;
    };
    let plan = crate::actions::delete_command(&gist_id);
    state.back_to_list();
    match crate::actions::execute_command(&plan) {
        Ok(_) => {
            state.set_status(format!("Deleted gist {gist_id}"));
            refresh_gists(state);
        }
        Err(error) => state.set_status(format!("delete failed: {error}")),
    }
}

fn render(frame: &mut Frame, state: &AppState) {
    match state.screen {
        Screen::List => render_list(frame, state),
        Screen::Diff => render_diff(frame, state, false),
        Screen::Confirm => render_diff(frame, state, true),
        Screen::Preview => render_preview(frame, state),
        Screen::Help => render_help(frame),
        Screen::Pins => render_pins(frame, state),
        Screen::Gists => render_gists(frame, state),
    }
}

fn render_help(frame: &mut Frame) {
    let text = "\
Navigation
  Tab        switch pane (Local / Gists)
  Up/Down    move the selection
  Left/Right scroll a long row horizontally

Gist list
  r          toggle recursive file discovery (skips hidden + configured dirs)
  /          filter by filename or description
  v          cycle visibility: all / public / secret
  s          cycle sort: match / name / recent
  t          toggle row view: description / id

Actions (on the selected local file + gist)
  Enter      diff the local file against the gist
  Space      preview the gist file's content (R in preview to force-refresh)
  d          download the gist into the cwd
  u          upload the local file into the gist
  n          create a new gist from the local file (type a description, then s/p)
  p          pin / unpin the local <-> gist pair
  P          view / manage all pinned mappings (x to unpin)
  X          remove the selected file from its gist (y/n confirm)
  g          open the gist manager (edit description, delete gist)
  e          edit the local file in $EDITOR

Gist manager (g)
  Up/Down    move between gists
  Left/Right scroll a long description horizontally
  /          filter gists by description or id
  s          cycle sort: updated / created
  v          cycle visibility: all / public / secret
  e          edit the gist description (Enter apply, Esc cancel)
  o          open the gist in your web browser
  X          delete the entire gist and all its files (y/n confirm)
  q / Esc    back to the list

General
  Esc / q    close an overlay; from the list, quit the app
  ?          show this help";

    frame.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .title("Help — press any key to close")
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        ),
        frame.area(),
    );
}

fn render_preview(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)])
        .split(frame.area());

    frame.render_widget(
        Paragraph::new(state.diff_text.clone())
            .scroll((state.diff_scroll, state.diff_hscroll))
            .block(
                Block::default()
                    .title(state.preview_title.clone())
                    .borders(Borders::ALL)
                    .padding(Padding::horizontal(1)),
            ),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new("↑↓←→ scroll  ·  R refresh  ·  Esc/q back").block(
            Block::default()
                .title("Commands")
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        ),
        chunks[1],
    );
}

fn render_pins(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    let items: Vec<ListItem> = if state.pinned.is_empty() {
        vec![ListItem::new("(no pinned mappings — use p to pin a pair)")]
    } else {
        state
            .pinned
            .iter()
            .map(|m| {
                ListItem::new(format!(
                    "{}  ↔  {} / {}",
                    m.local_path.display(),
                    m.gist_id,
                    m.gist_filename
                ))
            })
            .collect()
    };

    let selected = (!state.pinned.is_empty()).then_some(state.pins_index);
    let list = List::new(items)
        .block(
            Block::default()
                .title("Pinned Mappings [focus]")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let footer = if state.pinned.is_empty() {
        "Esc/q back".to_string()
    } else {
        "↑↓ move  ·  x unpin  ·  Esc/q back".to_string()
    };
    frame.render_widget(
        Paragraph::new(footer).block(
            Block::default()
                .title("Commands")
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        ),
        chunks[1],
    );
}

fn gist_group_row_label(g: &GistGroup) -> String {
    let desc = if g.description.trim().is_empty() {
        "(no description)".to_string()
    } else {
        g.description.clone()
    };
    let visibility = if g.public { "public" } else { "secret" };
    let date: String = g.updated_at.chars().take(10).collect();
    format!(
        "{}  {}  [{}]  {}f  {}",
        g.id, desc, visibility, g.file_count, date
    )
}

fn render_gists(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    let groups = state.visible_gist_groups();
    let items: Vec<ListItem> = if groups.is_empty() {
        let msg = if state.gist_groups().is_empty() {
            "(no gists)"
        } else {
            "(no gists match the current filter)"
        };
        vec![ListItem::new(msg)]
    } else {
        groups
            .iter()
            .map(|g| ListItem::new(hscroll_str(&gist_group_row_label(g), state.gists_hscroll)))
            .collect()
    };

    let selected = (!groups.is_empty()).then_some(state.gists_index);
    let mut title = format!(
        "Gists [focus]  ·  sort:{}  ·  type:{}",
        state.gists_sort.label(),
        state.gists_type_filter.label()
    );
    if !state.gists_filter_query.is_empty() {
        title.push_str(&format!("  ·  /{}", state.gists_filter_query));
    }
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let (ftitle, footer) = if state.editing_description {
        (
            "Edit description (Enter apply · Esc cancel)",
            format!("{}_", state.description_input),
        )
    } else if state.gists_filtering {
        (
            "Filter (Enter keep · Esc clear)",
            format!("/{}_", state.gists_filter_query),
        )
    } else {
        (
            "Commands",
            "↑↓ move · ←→ scroll · / filter · s sort · v type · e desc · o browser · X delete · q back"
                .to_string(),
        )
    };
    frame.render_widget(
        Paragraph::new(footer).block(
            Block::default()
                .title(ftitle)
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        ),
        chunks[1],
    );
}

fn local_row_label(path: &std::path::Path, cwd: &std::path::Path) -> String {
    path.strip_prefix(cwd).unwrap_or(path).display().to_string()
}

fn hscroll_str(text: &str, offset: u16) -> String {
    text.chars().skip(offset as usize).collect()
}

/// Match strength as stars. Mirrors the ranking tiers (exact-filename/pinned = 1000+,
/// path hint = 250+); a recent-only score of 1 is too weak to be worth a star.
fn match_stars(score: u16) -> &'static str {
    match score {
        s if s >= 1000 => "⭐⭐⭐",
        s if s >= 250 => "⭐⭐",
        s if s >= 2 => "⭐",
        _ => "",
    }
}

fn gist_row_label(g: &RankedGistFile, view: GistView) -> String {
    let stars = match_stars(g.score);
    let prefix = if stars.is_empty() {
        String::new()
    } else {
        format!("{stars} ")
    };
    match view {
        GistView::Description => {
            if g.file.description.trim().is_empty() {
                format!("{prefix}{}", g.file.filename)
            } else {
                format!("{prefix}{} — {}", g.file.filename, g.file.description)
            }
        }
        GistView::Id => format!("{prefix}{} / {}", g.file.gist_id, g.file.filename),
    }
}

/// Command hint tailored to the focused pane: local-file actions on the left, gist actions
/// on the right, plus the always-available navigation/help/quit keys. The footer word-wraps
/// it to the terminal width.
fn commands_hint(focus: FocusPane) -> String {
    // Focus-relevant common keys only; the full reference lives in the `?` help overlay.
    let mut items = vec!["Tab panes", "↑↓ move", "Enter diff"];
    match focus {
        FocusPane::Local => items.extend(["r recursive", "e edit", "n create", "P pins"]),
        FocusPane::Gist => items.extend([
            "Space preview",
            "d download",
            "u upload",
            "X remove file",
            "g gists",
        ]),
    }
    items.extend(["? help", "Esc/q quit"]);
    items.join("  ·  ")
}

/// Greedy word-wrap line count, matching how `Paragraph` with `Wrap { trim: true }` breaks
/// space-separated words at `width`. Used to size the footer block to its content.
fn wrap_line_count(text: &str, width: u16) -> u16 {
    if width == 0 {
        return 1;
    }
    let width = width as usize;
    let mut lines: u16 = 1;
    let mut col = 0usize;
    for word in text.split_whitespace() {
        let w = word.chars().count();
        if col == 0 {
            col = w.min(width);
        } else if col + 1 + w <= width {
            col += 1 + w;
        } else {
            lines = lines.saturating_add(1);
            col = w.min(width);
        }
    }
    lines
}

fn render_list(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    let footer_body = if state.filtering {
        format!(
            "filter: {}_   (Enter apply · Esc clear)",
            state.filter_query
        )
    } else {
        match &state.status {
            Some(message) => message.clone(),
            None => commands_hint(state.focus),
        }
    };
    // Width inside the footer block: minus 2 borders and 2 horizontal padding columns.
    let footer_lines = wrap_line_count(&footer_body, area.width.saturating_sub(4)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(footer_lines + 2)])
        .split(area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    // Show each candidate's path relative to cwd; in flat mode this is just the filename,
    // in recursive mode it includes the subdirectory (e.g. src/utils/helpers.rs).
    let local_items: Vec<ListItem> = if state.local_scanning && state.locals.is_empty() {
        vec![ListItem::new("(scanning…)")]
    } else if state.locals.is_empty() {
        vec![ListItem::new("(no files in this directory)")]
    } else {
        state
            .locals
            .iter()
            .map(|c| {
                ListItem::new(hscroll_str(
                    &local_row_label(&c.path, &state.cwd),
                    state.local_hscroll,
                ))
            })
            .collect()
    };
    let local_focused = state.focus == FocusPane::Local;
    let local_selected = (!state.locals.is_empty()).then_some(state.local_index);
    let recursive_marker = if state.local_recursive { " [↓]" } else { "" };
    let scanning_marker = if state.local_scanning { " …" } else { "" };
    let local_title = format!(
        "Local · {}{}{}",
        state.cwd.display(),
        recursive_marker,
        scanning_marker
    );
    render_pane(
        frame,
        columns[0],
        &local_title,
        local_items,
        local_focused,
        local_selected,
    );

    let ranked = state.ranked_gists();
    let gist_items: Vec<ListItem> = if state.loading && ranked.is_empty() {
        vec![ListItem::new("Loading gists…")]
    } else if ranked.is_empty() {
        let message = if !state.filter_query.is_empty() {
            "(no gists match the filter)"
        } else {
            "(no gists)"
        };
        vec![ListItem::new(message)]
    } else {
        ranked
            .iter()
            .map(|g| {
                ListItem::new(hscroll_str(
                    &gist_row_label(g, state.gist_view),
                    state.gist_hscroll,
                ))
            })
            .collect()
    };
    let gist_focused = state.focus == FocusPane::Gist;
    let gist_selected = (!ranked.is_empty()).then_some(state.gist_index);
    let mut gist_title = format!(
        "Gists · {} · {}",
        state.gist_type_filter.label(),
        state.gist_sort.label()
    );
    if !state.filter_query.is_empty() {
        gist_title.push_str(&format!(" · /{}", state.filter_query));
    }
    render_pane(
        frame,
        columns[1],
        &gist_title,
        gist_items,
        gist_focused,
        gist_selected,
    );

    frame.render_widget(
        Paragraph::new(footer_body).wrap(Wrap { trim: true }).block(
            Block::default()
                .title("Commands")
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        ),
        chunks[1],
    );
}

fn render_pane(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: Vec<ListItem>,
    focused: bool,
    selected: Option<usize>,
) {
    let item_count = items.len();
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    // Dim the unfocused pane so the active side is obvious at a glance.
    let base_style = if focused {
        Style::default()
    } else {
        Style::default().fg(Color::DarkGray)
    };
    // Focused selection is a solid bar (whole row); unfocused just bolds the row.
    let highlight_style = if focused {
        Style::default()
            .bg(Color::Cyan)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    let title = if focused {
        format!("{title} [focus]")
    } else {
        title.to_string()
    };

    let list = List::new(items)
        .style(base_style)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style)
                .padding(Padding::horizontal(1)),
        )
        .highlight_style(highlight_style)
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, area, &mut list_state);

    // Show a scrollbar when the list overflows its viewport.
    let viewport = area.height.saturating_sub(2) as usize;
    if viewport > 0 && item_count > viewport {
        let mut scrollbar_state = ScrollbarState::new(item_count).position(selected.unwrap_or(0));
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

/// Builds the visible, coloured slice of a unified diff (additions green, deletions red,
/// `---`/`+++` headers bold). Scrolling is applied here by hand — skip `vscroll` lines and
/// drop `hscroll` leading chars per line — rather than via `Paragraph::scroll`, whose
/// styled-line handling leaves redraw artifacts in ratatui 0.26.
/// Skips `hscroll` characters across an ordered list of spans, preserving styles.
fn apply_hscroll_spans(spans: Vec<Span<'static>>, hscroll: usize) -> Line<'static> {
    let mut skip = hscroll;
    let visible: Vec<Span<'static>> = spans
        .into_iter()
        .filter_map(|span| {
            let len = span.content.chars().count();
            if skip >= len {
                skip -= len;
                None
            } else {
                let content: String = span.content.chars().skip(skip).collect();
                skip = 0;
                if content.is_empty() {
                    None
                } else {
                    Some(Span::styled(content, span.style))
                }
            }
        })
        .collect();
    Line::from(visible)
}

/// Del line with word-level highlighting: changed words bold-red, unchanged words plain red.
fn inline_del_line(del_line: &str, ins_line: &str, hscroll: usize) -> Line<'static> {
    let del_content = del_line.get(1..).unwrap_or("");
    let ins_content = ins_line.get(1..).unwrap_or("");
    let mut spans = vec![Span::styled("-", Style::default().fg(Color::Red))];
    for change in TextDiff::from_words(del_content, ins_content).iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            ChangeTag::Equal => spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(Color::Red),
            )),
            ChangeTag::Insert => {}
        }
    }
    apply_hscroll_spans(spans, hscroll)
}

/// Ins line with word-level highlighting: changed words bold-green, unchanged words plain green.
fn inline_ins_line(del_line: &str, ins_line: &str, hscroll: usize) -> Line<'static> {
    let del_content = del_line.get(1..).unwrap_or("");
    let ins_content = ins_line.get(1..).unwrap_or("");
    let mut spans = vec![Span::styled("+", Style::default().fg(Color::Green))];
    for change in TextDiff::from_words(del_content, ins_content).iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => spans.push(Span::styled(
                change.value().to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
            ChangeTag::Equal => spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(Color::Green),
            )),
            ChangeTag::Delete => {}
        }
    }
    apply_hscroll_spans(spans, hscroll)
}

/// Builds the visible, coloured slice of a unified diff. Adjacent `-`/`+` line pairs receive
/// word-level inline highlighting (changed words bold, unchanged words dim) so small edits are
/// easy to spot. Scrolling is applied by hand — skip `vscroll` lines and drop `hscroll` leading
/// chars per line — rather than via `Paragraph::scroll`, whose styled-line handling leaves
/// redraw artifacts in ratatui 0.26.
fn diff_view(text: &str, vscroll: u16, hscroll: u16) -> Text<'static> {
    let raw: Vec<&str> = text.lines().collect();
    let hscroll = hscroll as usize;
    let mut result: Vec<Line<'static>> = Vec::with_capacity(raw.len());

    let mut i = 0;
    while i < raw.len() {
        let line = raw[i];
        let is_del = line.starts_with('-') && !line.starts_with("---");
        let is_ins = line.starts_with('+') && !line.starts_with("+++");

        if is_del || is_ins {
            // Collect the contiguous del run then ins run.
            let del_start = i;
            while i < raw.len() && raw[i].starts_with('-') && !raw[i].starts_with("---") {
                i += 1;
            }
            let del_lines = &raw[del_start..i];

            let ins_start = i;
            while i < raw.len() && raw[i].starts_with('+') && !raw[i].starts_with("+++") {
                i += 1;
            }
            let ins_lines = &raw[ins_start..i];

            let pair_count = del_lines.len().min(ins_lines.len());

            // Del lines: paired ones get inline highlighting, extras plain red.
            for (j, &dl) in del_lines.iter().enumerate() {
                if j < pair_count {
                    result.push(inline_del_line(dl, ins_lines[j], hscroll));
                } else {
                    let visible: String = dl.chars().skip(hscroll).collect();
                    result.push(Line::styled(visible, Style::default().fg(Color::Red)));
                }
            }
            // Ins lines: paired ones get inline highlighting, extras plain green.
            for (j, &il) in ins_lines.iter().enumerate() {
                if j < pair_count {
                    result.push(inline_ins_line(del_lines[j], il, hscroll));
                } else {
                    let visible: String = il.chars().skip(hscroll).collect();
                    result.push(Line::styled(visible, Style::default().fg(Color::Green)));
                }
            }
        } else {
            let style = if line.starts_with("+++") || line.starts_with("---") {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let visible: String = line.chars().skip(hscroll).collect();
            result.push(Line::styled(visible, style));
            i += 1;
        }
    }

    Text::from(
        result
            .into_iter()
            .skip(vscroll as usize)
            .collect::<Vec<_>>(),
    )
}

fn render_diff(frame: &mut Frame, state: &AppState, confirming: bool) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)])
        .split(frame.area());

    // The gist id, filenames, and both sides' mtimes live in the diff's `--- / +++` header
    // lines (see `diff_labels`); the title stays concise and avoids repeating a path.
    let title = match &state.pending_action {
        Some(PendingAction::Upload {
            gist_id, filename, ..
        }) => format!("Upload → gist {gist_id} / {filename}"),
        Some(PendingAction::Create { local_path }) => {
            format!("Create gist from {}", local_path.display())
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
            let label = if state.diff_identical {
                "Diff (identical)"
            } else {
                "Diff"
            };
            if state.preview_local.as_os_str().is_empty()
                || state.preview_local == state.download_target
            {
                format!("{label} → {}", state.download_target.display())
            } else {
                format!(
                    "{label}: {} → {}",
                    state.preview_local.display(),
                    state.download_target.display()
                )
            }
        }
    };
    frame.render_widget(
        Paragraph::new(diff_view(
            &state.diff_text,
            state.diff_scroll,
            state.diff_hscroll,
        ))
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        ),
        chunks[0],
    );

    let footer = if confirming {
        match &state.pending_action {
            Some(PendingAction::Create { .. }) if state.editing_description => {
                format!(
                    "Description (optional): {}_   ·  Enter next  ·  Esc cancel",
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
                    local_path.display()
                )
            }
            Some(PendingAction::Upload {
                gist_id, filename, ..
            }) => {
                format!("Upload {} to gist {}? (y/n)", filename, gist_id)
            }
            Some(PendingAction::Delete { gist_id, label }) => {
                format!("Permanently delete \"{label}\" ({gist_id})? (y/n)")
            }
            Some(PendingAction::RemoveFile {
                gist_id, filename, ..
            }) => {
                format!("Remove {filename} from gist {gist_id}? (y/n)")
            }
            _ => format!("Overwrite {}? (y/n)", state.download_target.display()),
        }
    } else if state.diff_identical {
        "Files are identical — nothing to sync  ·  ↑↓←→ scroll  ·  Esc/q back".to_string()
    } else {
        "↑↓←→ scroll  ·  d download  ·  u upload  ·  Esc/q back".to_string()
    };
    frame.render_widget(
        Paragraph::new(footer).block(
            Block::default()
                .title("Commands")
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        ),
        chunks[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn tab_switches_focus() {
        let mut state = initial_state();
        assert_eq!(state.focus, FocusPane::Local);
        state.handle_key(KeyCode::Tab);
        assert_eq!(state.focus, FocusPane::Gist);
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
    fn gist_row_label_switches_with_view() {
        let g = RankedGistFile {
            file: GistFile {
                gist_id: "abc".into(),
                description: "My Ghostty config".into(),
                filename: "config".into(),
                public: true,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
            score: 1,
            reasons: Vec::new(),
        };
        // A recent-only score of 1 is too weak to earn a star.
        assert_eq!(
            gist_row_label(&g, GistView::Description),
            "config — My Ghostty config"
        );
        assert_eq!(gist_row_label(&g, GistView::Id), "abc / config");
    }

    #[test]
    fn strong_match_prefixes_stars() {
        let g = RankedGistFile {
            file: GistFile {
                gist_id: "abc".into(),
                description: "cfg".into(),
                filename: "config".into(),
                public: true,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
            score: 1000,
            reasons: Vec::new(),
        };
        assert_eq!(
            gist_row_label(&g, GistView::Description),
            "⭐⭐⭐ config — cfg"
        );
        assert_eq!(gist_row_label(&g, GistView::Id), "⭐⭐⭐ abc / config");
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
        assert_eq!(state.gist_type_filter, GistTypeFilter::All);
    }

    #[test]
    fn s_cycles_gist_sort() {
        let mut state = initial_state();
        assert_eq!(state.gist_sort, GistSort::Match);
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(state.gist_sort, GistSort::Name);
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(state.gist_sort, GistSort::Recent);
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(state.gist_sort, GistSort::Match);
    }

    #[test]
    fn sort_by_name_and_recent_reorders_gists() {
        let mut state = initial_state();
        state.gists = vec![
            GistFile {
                gist_id: "z".into(),
                description: "".into(),
                filename: "zeta.json".into(),
                public: true,
                updated_at: "2026-01-01T00:00:00Z".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
            },
            GistFile {
                gist_id: "a".into(),
                description: "".into(),
                filename: "alpha.json".into(),
                public: true,
                updated_at: "2026-09-09T00:00:00Z".into(),
                created_at: "2026-09-09T00:00:00Z".into(),
            },
        ];
        // No local selected -> Match keeps gh list order (zeta, alpha).
        assert_eq!(state.ranked_gists()[0].file.filename, "zeta.json");

        state.gist_sort = GistSort::Name;
        assert_eq!(state.ranked_gists()[0].file.filename, "alpha.json");

        state.gist_sort = GistSort::Recent;
        assert_eq!(state.ranked_gists()[0].file.filename, "alpha.json");
        assert_eq!(state.ranked_gists()[1].file.filename, "zeta.json");
    }

    #[test]
    fn gist_type_filter_limits_ranked_gists() {
        let mut state = initial_state();
        state.gists = vec![
            GistFile {
                gist_id: "pub".into(),
                description: "p".into(),
                filename: "a.json".into(),
                public: true,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
            GistFile {
                gist_id: "sec".into(),
                description: "s".into(),
                filename: "b.json".into(),
                public: false,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
        ];
        assert_eq!(state.ranked_gists().len(), 2);

        state.gist_type_filter = GistTypeFilter::Public;
        let only_public = state.ranked_gists();
        assert_eq!(only_public.len(), 1);
        assert_eq!(only_public[0].file.gist_id, "pub");

        state.gist_type_filter = GistTypeFilter::Secret;
        let only_secret = state.ranked_gists();
        assert_eq!(only_secret.len(), 1);
        assert_eq!(only_secret[0].file.gist_id, "sec");
    }

    fn state_with_two_gists() -> AppState {
        let mut state = initial_state();
        state.gists = vec![
            GistFile {
                gist_id: "a".into(),
                description: "My Ghostty config".into(),
                filename: "config.ghostty".into(),
                public: true,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
            GistFile {
                gist_id: "b".into(),
                description: "SSH config".into(),
                filename: "ssh_config".into(),
                public: false,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
        ];
        state.focus = FocusPane::Gist;
        state
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
    fn confirm_screen_scrolls_diff() {
        let mut state = initial_state();
        state.pending_action = Some(PendingAction::Download);
        state.screen = Screen::Confirm;
        state.diff_text = "l1\nl2\nl3".into();
        assert_eq!(state.handle_key(KeyCode::Down), KeyOutcome::None);
        assert_eq!(state.diff_scroll, 1);
        state.handle_key(KeyCode::Up);
        assert_eq!(state.diff_scroll, 0);
    }

    #[test]
    fn space_on_selected_gist_returns_preview_content() {
        let mut state = state_with_two_gists();
        assert_eq!(
            state.handle_key(KeyCode::Char(' ')),
            KeyOutcome::PreviewContent
        );
    }

    #[test]
    fn space_without_gist_is_noop() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char(' ')), KeyOutcome::None);
    }

    #[test]
    fn esc_in_preview_returns_to_list_and_clears() {
        let mut state = initial_state();
        state.screen = Screen::Preview;
        state.diff_text = "raw content".into();
        state.preview_title = "Preview: a / x".into();
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert!(state.diff_text.is_empty());
        assert!(state.preview_title.is_empty());
    }

    #[test]
    fn preview_scrolls_with_arrows() {
        let mut state = initial_state();
        state.screen = Screen::Preview;
        state.diff_text = "l1\nl2\nl3".into();
        state.handle_key(KeyCode::Down);
        assert_eq!(state.diff_scroll, 1);
        state.handle_key(KeyCode::Up);
        assert_eq!(state.diff_scroll, 0);
    }

    #[test]
    fn question_opens_help_and_any_key_closes_it() {
        let mut state = initial_state();
        state.handle_key(KeyCode::Char('?'));
        assert_eq!(state.screen, Screen::Help);
        assert_eq!(state.handle_key(KeyCode::Char('x')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn q_in_help_closes_to_list() {
        let mut state = initial_state();
        state.screen = Screen::Help;
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn diff_view_applies_vertical_and_horizontal_scroll() {
        let text = "--- a\n+++ b\nabcdef\n more";
        let v = diff_view(text, 2, 2); // skip 2 lines, drop 2 leading chars
        assert_eq!(v.lines.len(), 2);
        assert_eq!(v.lines[0].spans[0].content, "cdef");
    }

    #[test]
    fn diff_view_inline_highlights_changed_words() {
        // A single-line modification: "hello world" → "hello planet"
        let text = "--- a\n+++ b\n-hello world\n+hello planet\n";
        let v = diff_view(text, 2, 0); // skip header lines
                                       // del line: span 0 is "-", unchanged word "hello " is plain red,
                                       //           changed word "world" is bold red
        assert_eq!(v.lines.len(), 2);
        let del = &v.lines[0];
        let sign = del.spans.iter().find(|s| s.content == "-").unwrap();
        assert_eq!(sign.style.fg, Some(Color::Red));
        // "world" is the changed word — should be bold
        let world = del
            .spans
            .iter()
            .find(|s| s.content.trim() == "world")
            .unwrap();
        assert!(world.style.add_modifier.contains(Modifier::BOLD));
        // "hello " is unchanged — should NOT be bold
        let hello = del
            .spans
            .iter()
            .find(|s| s.content.starts_with("hello"))
            .unwrap();
        assert!(!hello.style.add_modifier.contains(Modifier::BOLD));
        // ins line: "planet" should be bold green
        let ins = &v.lines[1];
        let planet = ins
            .spans
            .iter()
            .find(|s| s.content.trim() == "planet")
            .unwrap();
        assert!(planet.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn format_unix_utc_known_instants() {
        assert_eq!(format_unix_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_unix_utc(1_780_656_360), "2026-06-05 10:46 UTC");
    }

    #[test]
    fn gist_time_label_normalises_rfc3339() {
        assert_eq!(
            gist_time_label("2026-06-08T11:06:18Z"),
            "2026-06-08 11:06 UTC"
        );
        assert_eq!(gist_time_label(""), "unknown");
        assert_eq!(gist_time_label("short"), "short");
    }

    #[test]
    fn commands_hint_is_focus_aware() {
        let local = commands_hint(FocusPane::Local);
        assert!(local.contains("e edit"));
        assert!(local.contains("n create"));
        assert!(!local.contains("d download"));

        let gist = commands_hint(FocusPane::Gist);
        assert!(gist.contains("d download"));
        assert!(gist.contains("g gists"));
        assert!(!gist.contains("e edit"));

        // Always-available keys appear in both.
        for hint in [local, gist] {
            assert!(hint.contains("? help"));
            assert!(hint.contains("Esc/q quit"));
        }
    }

    #[test]
    fn wrap_line_count_is_responsive_to_width() {
        let text = "aaa bbb ccc";
        assert_eq!(wrap_line_count(text, 100), 1);
        assert_eq!(wrap_line_count(text, 7), 2);
        assert_eq!(wrap_line_count(text, 3), 3);
        assert_eq!(wrap_line_count(text, 0), 1);
    }

    #[test]
    fn match_stars_tiers() {
        assert_eq!(match_stars(0), "");
        assert_eq!(match_stars(1), "");
        assert_eq!(match_stars(250), "⭐⭐");
        assert_eq!(match_stars(1000), "⭐⭐⭐");
        assert_eq!(match_stars(10_001), "⭐⭐⭐");
    }

    #[test]
    fn gist_row_label_falls_back_to_filename_when_description_empty() {
        let g = RankedGistFile {
            file: GistFile {
                gist_id: "abc".into(),
                description: "  ".into(),
                filename: "config".into(),
                public: true,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
            score: 0,
            reasons: Vec::new(),
        };
        assert_eq!(gist_row_label(&g, GistView::Description), "config");
    }

    #[test]
    fn left_right_scrolls_focused_gist_pane() {
        let mut state = initial_state();
        state.gists = vec![GistFile {
            gist_id: "a".into(),
            description: "a fairly long description for scrolling".into(),
            filename: "f.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
        }];
        state.focus = FocusPane::Gist;
        assert_eq!(state.gist_hscroll, 0);
        state.handle_key(KeyCode::Left); // saturates at 0
        assert_eq!(state.gist_hscroll, 0);
        state.handle_key(KeyCode::Right);
        state.handle_key(KeyCode::Right);
        assert_eq!(state.gist_hscroll, 2);
        state.handle_key(KeyCode::Left);
        assert_eq!(state.gist_hscroll, 1);
    }

    #[test]
    fn gist_hscroll_caps_at_longest_row() {
        let mut state = initial_state();
        state.gists = vec![GistFile {
            gist_id: "a".into(),
            description: "tiny".into(),
            filename: "f".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
        }];
        state.focus = FocusPane::Gist;
        let row = gist_row_label(&state.ranked_gists()[0], state.gist_view);
        let max = (row.chars().count() - 1) as u16;
        for _ in 0..200 {
            state.handle_key(KeyCode::Right);
        }
        assert_eq!(state.gist_hscroll, max);
    }

    #[test]
    fn moving_gist_selection_resets_hscroll() {
        let mut state = initial_state();
        state.gists = vec![
            GistFile {
                gist_id: "a".into(),
                description: "first long description here".into(),
                filename: "a.json".into(),
                public: false,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
            GistFile {
                gist_id: "b".into(),
                description: "second long description here".into(),
                filename: "b.json".into(),
                public: false,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
        ];
        state.focus = FocusPane::Gist;
        state.handle_key(KeyCode::Right);
        assert_eq!(state.gist_hscroll, 1);
        state.handle_key(KeyCode::Down);
        assert_eq!(state.gist_hscroll, 0);
    }

    #[test]
    fn empty_state_has_no_ranked_gists() {
        let state = initial_state();
        assert!(state.ranked_gists().is_empty());
    }

    #[test]
    fn no_local_selected_lists_all_gists_unranked() {
        let mut state = initial_state();
        state.gists = vec![
            GistFile {
                gist_id: "a".into(),
                description: "first".into(),
                filename: "alpha.json".into(),
                public: false,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
            GistFile {
                gist_id: "b".into(),
                description: "second".into(),
                filename: "beta.json".into(),
                public: false,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
        ];
        let ranked = state.ranked_gists();
        assert_eq!(ranked.len(), 2);
        // Order preserved (unranked) and no scoring applied.
        assert_eq!(ranked[0].file.filename, "alpha.json");
        assert_eq!(ranked[0].score, 0);
        assert!(ranked[0].reasons.is_empty());
    }

    #[test]
    fn enter_with_no_local_but_gist_selected_returns_preview() {
        let mut state = initial_state();
        state.gists = vec![GistFile {
            gist_id: "a".into(),
            description: "first".into(),
            filename: "alpha.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
        }];
        state.focus = FocusPane::Gist;
        assert!(state.locals.is_empty());
        assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::PreviewDiff);
    }

    #[test]
    fn local_selection_changes_ranked_gists() {
        let mut state = initial_state();
        state.locals = vec![
            LocalCandidate {
                path: PathBuf::from("/tmp/settings.json"),
                pinned: false,
            },
            LocalCandidate {
                path: PathBuf::from("/tmp/statusline.sh"),
                pinned: false,
            },
        ];
        state.gists = vec![
            GistFile {
                gist_id: "a".into(),
                description: "settings".into(),
                filename: "settings.json".into(),
                public: false,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
            GistFile {
                gist_id: "b".into(),
                description: "status".into(),
                filename: "statusline.sh".into(),
                public: false,
                updated_at: "x".into(),
                created_at: "x".into(),
            },
        ];

        assert_eq!(state.ranked_gists()[0].file.filename, "settings.json");
        state.handle_key(KeyCode::Down);
        assert_eq!(state.ranked_gists()[0].file.filename, "statusline.sh");
    }

    #[test]
    fn changing_local_selection_resets_gist_index() {
        let mut state = initial_state();
        state.locals = vec![
            LocalCandidate {
                path: PathBuf::from("/tmp/a.json"),
                pinned: false,
            },
            LocalCandidate {
                path: PathBuf::from("/tmp/b.json"),
                pinned: false,
            },
        ];
        state.gist_index = 2;
        state.handle_key(KeyCode::Down); // move local selection down
        assert_eq!(state.gist_index, 0);
    }

    fn state_with_selection() -> AppState {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/settings.json"),
            pinned: false,
        }];
        state.gists = vec![GistFile {
            gist_id: "a".into(),
            description: "settings".into(),
            filename: "settings.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
        }];
        state.focus = FocusPane::Gist;
        state
    }

    #[test]
    fn enter_diff_sets_diff_screen() {
        let mut state = initial_state();
        state.enter_diff(
            "the diff".into(),
            "remote body".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/cwd/x"),
        );
        assert_eq!(state.screen, Screen::Diff);
        assert!(state.diff_previewed);
        assert_eq!(state.preview_remote, "remote body");
        assert_eq!(state.preview_local, PathBuf::from("/tmp/x"));
        assert_eq!(state.download_target, PathBuf::from("/tmp/cwd/x"));
        assert_eq!(state.diff_scroll, 0);
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
        assert!(!state.diff_previewed);
        assert!(state.diff_text.is_empty());
        assert!(state.preview_remote.is_empty());
        assert_eq!(state.preview_local, PathBuf::new());
        assert_eq!(state.download_target, PathBuf::new());
    }

    #[test]
    fn enter_in_gist_focus_with_selection_returns_preview() {
        let mut state = state_with_selection();
        assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::PreviewDiff);
    }

    #[test]
    fn enter_in_local_focus_previews_top_gist() {
        let mut state = state_with_selection();
        state.focus = FocusPane::Local;
        assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::PreviewDiff);
    }

    #[test]
    fn enter_with_no_gists_is_noop_in_local_focus() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/x"),
            pinned: false,
        }];
        state.focus = FocusPane::Local;
        assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
    }

    #[test]
    fn d_in_gist_focus_returns_download_gist() {
        let mut state = state_with_selection();
        assert_eq!(
            state.handle_key(KeyCode::Char('d')),
            KeyOutcome::DownloadGist
        );
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
            pinned: false,
        }];
        state.focus = FocusPane::Gist;
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
    }

    #[test]
    fn enter_without_gists_is_noop() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/x"),
            pinned: false,
        }];
        state.focus = FocusPane::Gist;
        assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
    }

    #[test]
    fn diff_scroll_respects_bounds() {
        let mut state = initial_state();
        state.enter_diff(
            "l1\nl2\nl3".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        assert_eq!(state.diff_scroll, 0);
        state.handle_key(KeyCode::Up); // stays at 0
        assert_eq!(state.diff_scroll, 0);
        state.handle_key(KeyCode::Down);
        assert_eq!(state.diff_scroll, 1);
        state.handle_key(KeyCode::Down);
        assert_eq!(state.diff_scroll, 2);
        state.handle_key(KeyCode::Down); // capped at lines-1 = 2
        assert_eq!(state.diff_scroll, 2);
        state.handle_key(KeyCode::Up);
        assert_eq!(state.diff_scroll, 1);
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
        state.diff_identical = true;
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
        assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::None);
        // Scrolling and leaving still work.
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn diff_hscroll_respects_bounds() {
        let mut state = initial_state();
        // Longest line is "abcd" (4 chars) -> max offset 3.
        state.enter_diff(
            "abcd\nab".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        assert_eq!(state.diff_hscroll, 0);
        state.handle_key(KeyCode::Left); // stays at 0
        assert_eq!(state.diff_hscroll, 0);
        for _ in 0..10 {
            state.handle_key(KeyCode::Right);
        }
        assert_eq!(state.diff_hscroll, 3);
        state.handle_key(KeyCode::Left);
        assert_eq!(state.diff_hscroll, 2);
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
        assert!(!state.diff_previewed);
    }

    #[test]
    fn d_in_diff_downloads_when_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.json");
        let mut state = initial_state();
        state.enter_diff("d".into(), "r".into(), PathBuf::from("/tmp/local"), missing);
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::Download);
    }

    #[test]
    fn d_in_diff_confirms_when_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("exists.json");
        std::fs::write(&existing, "old").unwrap();
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/local"),
            existing,
        );
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::Confirm);
    }

    #[test]
    fn confirm_y_returns_download() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        state.pending_action = Some(PendingAction::Download);
        state.screen = Screen::Confirm;
        assert_eq!(state.handle_key(KeyCode::Char('y')), KeyOutcome::Download);
    }

    #[test]
    fn confirm_n_returns_to_diff() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        state.pending_action = Some(PendingAction::Download);
        state.screen = Screen::Confirm;
        assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::Diff);
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
    fn q_in_list_quits() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::Quit);
    }

    #[test]
    fn esc_in_list_quits() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::Quit);
    }

    #[test]
    fn q_in_confirm_cancels_without_quitting() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        state.pending_action = Some(PendingAction::Download);
        state.screen = Screen::Confirm;
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::Diff);
    }

    #[test]
    fn confirm_esc_returns_to_diff() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        state.pending_action = Some(PendingAction::Download);
        state.screen = Screen::Confirm;
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::Diff);
    }

    #[test]
    fn d_in_diff_on_existing_sets_download_pending() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("exists.json");
        std::fs::write(&existing, "old").unwrap();
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/local"),
            existing,
        );
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::Confirm);
        assert_eq!(state.pending_action, Some(PendingAction::Download));
    }

    #[test]
    fn p_pins_unpinned_pair_then_unpins() {
        let mut state = state_with_selection();
        assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::Pin);
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("/tmp/settings.json"),
            gist_id: "a".into(),
            gist_filename: "settings.json".into(),
            direction: None,
            last_seen_hash: None,
        }];
        assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::Unpin);
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
            pinned: false,
        }];
        state.gists = vec![GistFile {
            gist_id: "a".into(),
            description: "x".into(),
            filename: "settings.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
        }];
        state.focus = FocusPane::Gist;
        assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::UploadAdd);
    }

    #[test]
    fn u_previews_when_gist_has_same_filename() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/settings.json"),
            pinned: false,
        }];
        state.gists = vec![GistFile {
            gist_id: "a".into(),
            description: "x".into(),
            filename: "settings.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
        }];
        state.focus = FocusPane::Gist;
        assert_eq!(
            state.handle_key(KeyCode::Char('u')),
            KeyOutcome::UploadPreview
        );
    }

    #[test]
    fn u_without_selection_is_noop() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::None);
    }

    #[test]
    fn o_in_gist_view_opens_browser() {
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists;
        assert_eq!(
            state.handle_key(KeyCode::Char('o')),
            KeyOutcome::OpenBrowser
        );
    }

    #[test]
    fn o_on_main_list_is_noop_now_that_browser_moved_to_gist_view() {
        let mut state = state_with_two_gists();
        assert_eq!(state.handle_key(KeyCode::Char('o')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn e_edits_local_with_file_selected() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/config"),
            pinned: false,
        }];
        assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::EditLocal);
    }

    #[test]
    fn e_without_local_is_noop() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::None);
    }

    #[test]
    fn u_in_diff_screen_returns_upload_intent() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/config"),
            pinned: false,
        }];
        state.gists = vec![GistFile {
            gist_id: "a".into(),
            description: "x".into(),
            filename: "settings.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
        }];
        state.screen = Screen::Diff;
        // The gist has no "config" file -> case B -> add directly.
        assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::UploadAdd);
    }

    #[test]
    fn confirm_upload_y_returns_upload() {
        let mut state = initial_state();
        state.pending_action = Some(PendingAction::Upload {
            gist_id: "a".into(),
            filename: "settings.json".into(),
            local_path: PathBuf::from("/tmp/settings.json"),
        });
        state.screen = Screen::Confirm;
        assert_eq!(state.handle_key(KeyCode::Char('y')), KeyOutcome::Upload);
    }

    #[test]
    fn n_opens_create_confirm() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/config.toml"),
            pinned: false,
        }];
        assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::Confirm);
        assert_eq!(
            state.pending_action,
            Some(PendingAction::Create {
                local_path: PathBuf::from("/tmp/config.toml")
            })
        );
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
    fn x_removes_selected_file_from_a_multifile_gist() {
        let mut state = initial_state();
        state.focus = FocusPane::Gist;
        state.gists = vec![
            GistFile {
                gist_id: "abc123".into(),
                description: "my notes".into(),
                filename: "a.md".into(),
                public: false,
                updated_at: "2026-01-01T00:00:00Z".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
            },
            GistFile {
                gist_id: "abc123".into(),
                description: "my notes".into(),
                filename: "b.md".into(),
                public: false,
                updated_at: "2026-01-01T00:00:00Z".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
            },
        ];
        // X stages a single-file removal (not a whole-gist delete) and asks to confirm.
        assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::Confirm);
        assert_eq!(
            state.pending_action,
            Some(PendingAction::RemoveFile {
                gist_id: "abc123".into(),
                filename: "a.md".into(),
                label: "my notes".into(),
            })
        );
    }

    #[test]
    fn x_on_a_gists_only_file_is_blocked() {
        let mut state = initial_state();
        state.focus = FocusPane::Gist;
        state.gists = vec![GistFile {
            gist_id: "abc123".into(),
            description: String::new(),
            filename: "notes.md".into(),
            public: false,
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        }];
        // Removing the only file would leave a fileless gist, which GitHub forbids.
        assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert!(state.pending_action.is_none());
        assert!(state.status.as_deref().unwrap().contains("only file"));
    }

    #[test]
    fn remove_file_confirm_y_returns_execute_remove_file() {
        let mut state = initial_state();
        state.pending_action = Some(PendingAction::RemoveFile {
            gist_id: "abc123".into(),
            filename: "a.md".into(),
            label: "my notes".into(),
        });
        state.screen = Screen::Confirm;
        assert_eq!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::ExecuteRemoveFile
        );
    }

    #[test]
    fn delete_confirm_y_returns_execute_delete() {
        let mut state = initial_state();
        state.pending_action = Some(PendingAction::Delete {
            gist_id: "abc123".into(),
            label: "my notes".into(),
        });
        state.screen = Screen::Confirm;
        assert_eq!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::ExecuteDelete
        );
    }

    #[test]
    fn delete_confirm_n_returns_to_list() {
        let mut state = initial_state();
        state.pending_action = Some(PendingAction::Delete {
            gist_id: "abc123".into(),
            label: "my notes".into(),
        });
        state.screen = Screen::Confirm;
        assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert!(state.pending_action.is_none());
    }

    #[test]
    fn g_opens_gist_view_landing_on_the_selected_files_gist() {
        let mut state = state_with_two_gists();
        // Select the second gist's row in the main (file) list, then jump to the
        // gist-level view; it should land on that same gist.
        state.gist_index = 1;
        assert_eq!(state.handle_key(KeyCode::Char('g')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::Gists);
        assert_eq!(state.gists_index, 1);
        assert_eq!(state.selected_group().unwrap().id, "b");
    }

    #[test]
    fn g_with_no_gists_is_blocked() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char('g')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn gist_view_e_edits_description_with_prefill_and_enter_applies() {
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists;
        state.gists_index = 0;
        state.handle_key(KeyCode::Char('e'));
        assert!(state.editing_description);
        // Prefilled with the current description.
        assert_eq!(state.description_input, "My Ghostty config");
        state.handle_key(KeyCode::Char('!'));
        assert_eq!(state.description_input, "My Ghostty config!");
        assert_eq!(
            state.handle_key(KeyCode::Enter),
            KeyOutcome::ApplyDescription
        );
    }

    #[test]
    fn gist_view_esc_cancels_description_edit() {
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists;
        state.handle_key(KeyCode::Char('e'));
        assert!(state.editing_description);
        state.handle_key(KeyCode::Esc);
        assert!(!state.editing_description);
        assert!(state.description_input.is_empty());
    }

    #[test]
    fn gist_view_x_stages_whole_gist_delete() {
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists;
        state.gists_index = 1;
        assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::Confirm);
        assert_eq!(
            state.pending_action,
            Some(PendingAction::Delete {
                gist_id: "b".into(),
                label: "SSH config".into(),
            })
        );
    }

    #[test]
    fn gist_view_q_returns_to_list() {
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists;
        state.handle_key(KeyCode::Char('q'));
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn gist_view_v_cycles_visibility_filter() {
        // state_with_two_gists: gist "a" is public, gist "b" is secret.
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists;
        assert_eq!(state.visible_gist_groups().len(), 2);

        state.handle_key(KeyCode::Char('v')); // -> public
        let vis = state.visible_gist_groups();
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].id, "a");

        state.handle_key(KeyCode::Char('v')); // -> secret
        let vis = state.visible_gist_groups();
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].id, "b");

        state.handle_key(KeyCode::Char('v')); // -> all
        assert_eq!(state.visible_gist_groups().len(), 2);
    }

    #[test]
    fn gist_view_filter_narrows_then_esc_clears() {
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists;
        state.handle_key(KeyCode::Char('/'));
        assert!(state.gists_filtering);
        for c in "ssh".chars() {
            state.handle_key(KeyCode::Char(c));
        }
        let vis = state.visible_gist_groups();
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].id, "b"); // "SSH config"

        state.handle_key(KeyCode::Esc);
        assert!(!state.gists_filtering);
        assert!(state.gists_filter_query.is_empty());
        assert_eq!(state.visible_gist_groups().len(), 2);
    }

    #[test]
    fn gist_view_s_cycles_sort_updated_then_created() {
        let mut state = initial_state();
        state.screen = Screen::Gists;
        state.gists = vec![
            GistFile {
                gist_id: "old-upd".into(),
                description: "x".into(),
                filename: "f".into(),
                public: false,
                updated_at: "2026-01-01T00:00:00Z".into(),
                created_at: "2026-12-01T00:00:00Z".into(),
            },
            GistFile {
                gist_id: "new-upd".into(),
                description: "y".into(),
                filename: "g".into(),
                public: false,
                updated_at: "2026-06-01T00:00:00Z".into(),
                created_at: "2026-02-01T00:00:00Z".into(),
            },
        ];
        // Default: sort by updated (newest first).
        assert_eq!(state.gists_sort, GistGroupSort::Updated);
        assert_eq!(state.visible_gist_groups()[0].id, "new-upd");
        // s -> sort by created (newest created first).
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(state.gists_sort, GistGroupSort::Created);
        assert_eq!(state.visible_gist_groups()[0].id, "old-upd");
    }

    #[test]
    fn gist_view_left_right_scrolls_horizontally() {
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists;
        assert_eq!(state.gists_hscroll, 0);
        state.handle_key(KeyCode::Right);
        assert_eq!(state.gists_hscroll, 1);
        state.handle_key(KeyCode::Left);
        assert_eq!(state.gists_hscroll, 0);
        // Left at the origin saturates at 0.
        state.handle_key(KeyCode::Left);
        assert_eq!(state.gists_hscroll, 0);
    }

    #[test]
    fn create_confirm_s_and_p_choose_visibility() {
        let mut state = initial_state();
        state.pending_action = Some(PendingAction::Create {
            local_path: PathBuf::from("/tmp/config.toml"),
        });
        state.screen = Screen::Confirm;
        assert_eq!(
            state.handle_key(KeyCode::Char('s')),
            KeyOutcome::Create(false)
        );

        state.pending_action = Some(PendingAction::Create {
            local_path: PathBuf::from("/tmp/config.toml"),
        });
        state.screen = Screen::Confirm;
        assert_eq!(
            state.handle_key(KeyCode::Char('p')),
            KeyOutcome::Create(true)
        );
    }

    #[test]
    fn create_confirm_esc_cancels() {
        let mut state = initial_state();
        state.pending_action = Some(PendingAction::Create {
            local_path: PathBuf::from("/tmp/config.toml"),
        });
        state.screen = Screen::Confirm;
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert_eq!(state.pending_action, None);
    }

    fn state_ready_to_create() -> AppState {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("/tmp/config.toml"),
            pinned: false,
        }];
        state
    }

    #[test]
    fn n_starts_create_in_the_description_editor() {
        let mut state = state_ready_to_create();
        state.handle_key(KeyCode::Char('n'));
        assert_eq!(state.screen, Screen::Confirm);
        assert!(state.editing_description);
        // While editing, letters (incl. s/p) are typed into the description, not
        // interpreted as the visibility choice.
        for c in "notes".chars() {
            assert_eq!(state.handle_key(KeyCode::Char(c)), KeyOutcome::None);
        }
        assert_eq!(state.description_input, "notes");
    }

    #[test]
    fn create_enter_advances_to_visibility_then_s_creates() {
        let mut state = state_ready_to_create();
        state.handle_key(KeyCode::Char('n'));
        state.handle_key(KeyCode::Char('h'));
        state.handle_key(KeyCode::Char('i'));
        // Enter ends the description step (does not create yet).
        assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
        assert!(!state.editing_description);
        assert_eq!(state.description_input, "hi");
        // Now s/p choose visibility and trigger the create.
        assert_eq!(
            state.handle_key(KeyCode::Char('s')),
            KeyOutcome::Create(false)
        );
    }

    #[test]
    fn create_esc_while_editing_description_cancels() {
        let mut state = state_ready_to_create();
        state.handle_key(KeyCode::Char('n'));
        state.handle_key(KeyCode::Char('x'));
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert_eq!(state.pending_action, None);
        assert!(!state.editing_description);
        assert!(state.description_input.is_empty());
    }
}
