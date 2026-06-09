use crate::domain::{GistFile, LocalCandidate, PinnedMapping};
use crate::ranking::{rank_gist_files, RankedGistFile};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
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
    pub diff_previewed: bool,
    pub diff_text: String,
    pub diff_scroll: u16,
    pub diff_hscroll: u16,
    pub preview_remote: String,
    pub preview_local: PathBuf,
    pub download_target: PathBuf,
    pub cwd: PathBuf,
    pub status: Option<String>,
}

impl AppState {
    pub fn ranked_gists(&self) -> Vec<RankedGistFile> {
        let gists: Vec<GistFile> = self
            .gists
            .iter()
            .filter(|g| self.gist_type_filter.matches(g.public))
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

    /// Highest horizontal-scroll offset for the focused pane, based on its longest row
    /// (viewport width is unknown to the pure key logic, mirroring the diff scroll cap).
    fn focused_hscroll_max(&self) -> u16 {
        let longest = match self.focus {
            FocusPane::Local => self
                .locals
                .iter()
                .map(|c| local_row_label(&c.path).chars().count())
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
            Screen::List => self.handle_key_list(code),
            Screen::Diff => self.handle_key_diff(code),
            Screen::Confirm => self.handle_key_confirm(code),
        }
    }

    fn handle_key_list(&mut self, code: KeyCode) -> KeyOutcome {
        // Any key dismisses a lingering status message (e.g. "Downloaded …"). A new
        // status may be set afterwards by the run_loop IO helper for this key.
        self.status = None;
        match code {
            KeyCode::Char('q') => return KeyOutcome::Quit,
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
            KeyCode::Char('d')
                if self.focus == FocusPane::Gist && self.gist_index < self.ranked_gists().len() =>
            {
                return KeyOutcome::DownloadGist;
            }
            KeyCode::Enter
                if self.focus == FocusPane::Gist && self.gist_index < self.ranked_gists().len() =>
            {
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
            KeyCode::Char('u') => {
                let (Some(local), Some(gist)) =
                    (self.selected_local().cloned(), self.selected_gist())
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
                return if has_same_name {
                    KeyOutcome::UploadPreview
                } else {
                    KeyOutcome::UploadAdd
                };
            }
            KeyCode::Char('n') => {
                let Some(local) = self.selected_local().cloned() else {
                    self.status = Some("select a local file to create a gist".into());
                    return KeyOutcome::None;
                };
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
            KeyCode::Char('q') => return KeyOutcome::Quit,
            KeyCode::Esc => self.back_to_list(),
            KeyCode::Down => self.scroll_diff_down(),
            KeyCode::Up => self.scroll_diff_up(),
            KeyCode::Right => self.scroll_diff_right(),
            KeyCode::Left => self.scroll_diff_left(),
            KeyCode::Char('d') => {
                if self.download_target.exists() {
                    self.pending_action = Some(PendingAction::Download);
                    self.screen = Screen::Confirm;
                } else {
                    return KeyOutcome::Download;
                }
            }
            _ => {}
        }
        KeyOutcome::None
    }

    fn handle_key_confirm(&mut self, code: KeyCode) -> KeyOutcome {
        if code == KeyCode::Char('q') {
            return KeyOutcome::Quit;
        }
        match self.pending_action.clone() {
            Some(PendingAction::Download) => match code {
                KeyCode::Char('y') => return KeyOutcome::Download,
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.pending_action = None;
                    self.screen = Screen::Diff;
                }
                _ => {}
            },
            Some(PendingAction::Upload { .. }) => match code {
                KeyCode::Char('y') => return KeyOutcome::Upload,
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.pending_action = None;
                    self.screen = Screen::List;
                }
                _ => {}
            },
            Some(PendingAction::Create { .. }) => match code {
                KeyCode::Char('s') => return KeyOutcome::Create(false),
                KeyCode::Char('p') => return KeyOutcome::Create(true),
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.pending_action = None;
                    self.screen = Screen::List;
                }
                _ => {}
            },
            _ => {
                if matches!(code, KeyCode::Esc | KeyCode::Char('n')) {
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
        diff_previewed: false,
        diff_text: String::new(),
        diff_scroll: 0,
        diff_hscroll: 0,
        preview_remote: String::new(),
        preview_local: PathBuf::new(),
        download_target: PathBuf::new(),
        cwd: PathBuf::from("."),
        status: None,
    }
}

pub fn load_startup_state() -> Result<AppState> {
    let mut state = initial_state();
    let config_path = crate::config::config_path()?;
    let config = crate::config::load_config(&config_path)?;
    let cwd = std::env::current_dir()?;

    state.pinned = config.pinned;
    state.locals = crate::local::discover_local_candidates(&cwd, &state.pinned)?;
    state.cwd = cwd;
    // Start focused on the gist pane: the common flow is to pick a gist and pull it
    // into the cwd, and the gist list is shown even when no local file is selected.
    state.focus = FocusPane::Gist;

    if crate::gh::check_gh_ready().is_ok() {
        if let Ok(gists) =
            crate::gh::fetch_gist_list_json().and_then(|raw| crate::gh::parse_gist_list_json(&raw))
        {
            state.gists = gists;
        }
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

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut state = load_startup_state()?;

    loop {
        terminal.draw(|frame| render(frame, &state))?;

        if let Event::Key(key) = event::read()? {
            match state.handle_key(key.code) {
                KeyOutcome::Quit => break,
                KeyOutcome::PreviewDiff => preview_diff(&mut state),
                KeyOutcome::Download => download(&mut state),
                KeyOutcome::DownloadGist => download_selected(&mut state),
                KeyOutcome::Pin => pin_selected(&mut state),
                KeyOutcome::Unpin => unpin_selected(&mut state),
                KeyOutcome::UploadAdd => upload_add(&mut state),
                KeyOutcome::UploadPreview => upload_preview(&mut state),
                KeyOutcome::Upload => upload_replace(&mut state),
                KeyOutcome::Create(public) => create_gist(&mut state, public),
                KeyOutcome::None => {}
            }
        }
    }

    Ok(())
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
            let diff = crate::diff::unified_diff("local", &local_content, "gist", &remote);
            // Download saves the gist into the current working directory under the
            // gist's own filename, leaving the compared local file untouched.
            let target = state.cwd.join(&gist.filename);
            state.enter_diff(diff, remote, local_path.unwrap_or_default(), target);
        }
        Err(error) => state.set_status(format!("fetch failed: {error}")),
    }
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
                let diff = crate::diff::unified_diff("local", &local_content, "gist", &remote);
                state.enter_diff(diff, remote, target.clone(), target);
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

/// Re-discovers the cwd file list after a write so a freshly downloaded file appears in the
/// Local pane. The current selection is preserved by path when still present.
fn refresh_locals(state: &mut AppState) {
    let selected = state.selected_local().map(|c| c.path.clone());
    if let Ok(locals) = crate::local::discover_local_candidates(&state.cwd, &state.pinned) {
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
            state.set_status(format!("Unpinned {}", local.path.display()));
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
            state.set_status(format!(
                "Uploaded {} to gist {}",
                local.path.display(),
                gist.file.gist_id
            ));
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
            let diff = crate::diff::unified_diff("gist", &remote, "local", &local_content);
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
    };
    let plan = crate::actions::upload_command(&local_path, &target);
    match crate::actions::execute_command(&plan) {
        Ok(_) => {
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
    let plan = crate::actions::create_command(&local_path, public);
    match crate::actions::execute_command(&plan) {
        Ok(_) => {
            let visibility = if public { "public" } else { "secret" };
            state.set_status(format!(
                "Created {} gist from {}",
                visibility,
                local_path.display()
            ));
            state.back_to_list();
            refresh_gists(state);
        }
        Err(error) => {
            state.set_status(format!("create failed: {error}"));
            state.screen = Screen::List;
            state.pending_action = None;
        }
    }
}

fn render(frame: &mut Frame, state: &AppState) {
    match state.screen {
        Screen::List => render_list(frame, state),
        Screen::Diff => render_diff(frame, state, false),
        Screen::Confirm => render_diff(frame, state, true),
    }
}

fn local_row_label(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
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

fn render_list(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)])
        .split(frame.size());
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    // Discovery is scoped to the cwd, so each candidate is a direct child; show just the
    // file name and put the cwd in the pane title as the shared baseline.
    let local_items: Vec<ListItem> = state
        .locals
        .iter()
        .map(|c| ListItem::new(hscroll_str(&local_row_label(&c.path), state.local_hscroll)))
        .collect();
    let local_focused = state.focus == FocusPane::Local;
    let local_selected = (!state.locals.is_empty()).then_some(state.local_index);
    let local_title = format!("Local · {}", state.cwd.display());
    render_pane(
        frame,
        columns[0],
        &local_title,
        local_items,
        local_focused,
        local_selected,
    );

    let ranked = state.ranked_gists();
    let gist_items: Vec<ListItem> = ranked
        .iter()
        .map(|g| {
            ListItem::new(hscroll_str(
                &gist_row_label(g, state.gist_view),
                state.gist_hscroll,
            ))
        })
        .collect();
    let gist_focused = state.focus == FocusPane::Gist;
    let gist_selected = (!ranked.is_empty()).then_some(state.gist_index);
    let gist_title = format!(
        "Gists · {} · {}",
        state.gist_type_filter.label(),
        state.gist_sort.label()
    );
    render_pane(
        frame,
        columns[1],
        &gist_title,
        gist_items,
        gist_focused,
        gist_selected,
    );

    let footer = match &state.status {
        Some(message) => message.clone(),
        None => "Tab  ↑↓ move  ←→ scroll  Enter diff  d download  t view  v type  s sort  q quit"
            .to_string(),
    };
    frame.render_widget(
        Paragraph::new(footer).block(Block::default().title("Commands").borders(Borders::ALL)),
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
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let highlight_style = if focused {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    let title = if focused {
        format!("{title} [focus]")
    } else {
        title.to_string()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .highlight_style(highlight_style)
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_diff(frame: &mut Frame, state: &AppState, confirming: bool) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)])
        .split(frame.size());

    let title = format!(
        "Diff: {}  ->  {}",
        state.preview_local.display(),
        state.download_target.display()
    );
    frame.render_widget(
        Paragraph::new(state.diff_text.clone())
            .scroll((state.diff_scroll, state.diff_hscroll))
            .block(Block::default().title(title).borders(Borders::ALL)),
        chunks[0],
    );

    let footer = if confirming {
        match &state.pending_action {
            Some(PendingAction::Create { local_path }) => format!(
                "Create gist from {}?  s secret  p public  Esc cancel",
                local_path.display()
            ),
            Some(PendingAction::Upload {
                gist_id, filename, ..
            }) => {
                format!("Upload {} to gist {}? (y/n)", filename, gist_id)
            }
            _ => format!("Overwrite {}? (y/n)", state.download_target.display()),
        }
    } else {
        format!(
            "Up/Down/Left/Right scroll  d download -> {}  Esc back  q quit",
            state.download_target.display()
        )
    };
    frame.render_widget(
        Paragraph::new(footer).block(Block::default().title("Commands").borders(Borders::ALL)),
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
            },
            GistFile {
                gist_id: "a".into(),
                description: "".into(),
                filename: "alpha.json".into(),
                public: true,
                updated_at: "2026-09-09T00:00:00Z".into(),
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
            },
            GistFile {
                gist_id: "sec".into(),
                description: "s".into(),
                filename: "b.json".into(),
                public: false,
                updated_at: "x".into(),
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
            },
            GistFile {
                gist_id: "b".into(),
                description: "second long description here".into(),
                filename: "b.json".into(),
                public: false,
                updated_at: "x".into(),
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
            },
            GistFile {
                gist_id: "b".into(),
                description: "second".into(),
                filename: "beta.json".into(),
                public: false,
                updated_at: "x".into(),
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
            },
            GistFile {
                gist_id: "b".into(),
                description: "status".into(),
                filename: "statusline.sh".into(),
                public: false,
                updated_at: "x".into(),
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
    fn enter_in_local_focus_is_noop() {
        let mut state = state_with_selection();
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
    fn q_in_diff_quits() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::Quit);
    }

    #[test]
    fn q_in_list_quits() {
        let mut state = initial_state();
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::Quit);
    }

    #[test]
    fn q_in_confirm_quits() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        state.pending_action = Some(PendingAction::Download);
        state.screen = Screen::Confirm;
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::Quit);
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
}
