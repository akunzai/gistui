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
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
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
    ConfirmOverwrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    None,
    Quit,
    PreviewDiff,
    Download,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub locals: Vec<LocalCandidate>,
    pub gists: Vec<GistFile>,
    pub pinned: Vec<PinnedMapping>,
    pub focus: FocusPane,
    pub local_index: usize,
    pub gist_index: usize,
    pub screen: Screen,
    pub diff_previewed: bool,
    pub diff_text: String,
    pub diff_scroll: u16,
    pub preview_remote: String,
    pub preview_local: PathBuf,
    pub status: Option<String>,
}

impl AppState {
    pub fn ranked_gists(&self) -> Vec<RankedGistFile> {
        let Some(local) = self.locals.get(self.local_index) else {
            return Vec::new();
        };
        rank_gist_files(&local.path, &self.gists, &self.pinned)
    }

    pub fn selected_local(&self) -> Option<&LocalCandidate> {
        self.locals.get(self.local_index)
    }

    pub fn selected_gist(&self) -> Option<RankedGistFile> {
        self.ranked_gists().into_iter().nth(self.gist_index)
    }

    pub fn enter_diff(&mut self, diff_text: String, remote: String, local: PathBuf) {
        self.diff_text = diff_text;
        self.preview_remote = remote;
        self.preview_local = local;
        self.diff_previewed = true;
        self.diff_scroll = 0;
        self.status = None;
        self.screen = Screen::Diff;
    }

    pub fn back_to_list(&mut self) {
        self.screen = Screen::List;
        self.diff_text.clear();
        self.preview_remote.clear();
        self.preview_local = PathBuf::new();
        self.diff_scroll = 0;
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

    pub fn handle_key(&mut self, code: KeyCode) -> KeyOutcome {
        match self.screen {
            Screen::List => self.handle_key_list(code),
            Screen::Diff => self.handle_key_diff(code),
            Screen::ConfirmOverwrite => self.handle_key_confirm(code),
        }
    }

    fn handle_key_list(&mut self, code: KeyCode) -> KeyOutcome {
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
                }
                FocusPane::Gist if self.gist_index + 1 < self.ranked_gists().len() => {
                    self.gist_index += 1
                }
                _ => {}
            },
            KeyCode::Up => match self.focus {
                FocusPane::Local if self.local_index > 0 => {
                    self.local_index -= 1;
                    self.gist_index = 0;
                }
                FocusPane::Gist if self.gist_index > 0 => self.gist_index -= 1,
                _ => {}
            },
            KeyCode::Enter
                if self.focus == FocusPane::Gist
                    && self.selected_local().is_some()
                    && self.gist_index < self.ranked_gists().len() =>
            {
                return KeyOutcome::PreviewDiff;
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
            KeyCode::Char('d') => {
                if self.preview_local.exists() {
                    self.screen = Screen::ConfirmOverwrite;
                } else {
                    return KeyOutcome::Download;
                }
            }
            _ => {}
        }
        KeyOutcome::None
    }

    fn handle_key_confirm(&mut self, code: KeyCode) -> KeyOutcome {
        match code {
            KeyCode::Char('q') => return KeyOutcome::Quit,
            KeyCode::Char('y') => return KeyOutcome::Download,
            KeyCode::Char('n') | KeyCode::Esc => self.screen = Screen::Diff,
            _ => {}
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
        screen: Screen::List,
        diff_previewed: false,
        diff_text: String::new(),
        diff_scroll: 0,
        preview_remote: String::new(),
        preview_local: PathBuf::new(),
        status: None,
    }
}

pub fn load_startup_state() -> Result<AppState> {
    let mut state = initial_state();
    let config_path = crate::config::config_path()?;
    let config = crate::config::load_config(&config_path)?;
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let cwd = std::env::current_dir()?;

    state.pinned = config.pinned;
    state.locals = crate::local::discover_local_candidates(&cwd, &home, &state.pinned)?;

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
        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(5), Constraint::Length(3)])
                .split(frame.size());
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(chunks[0]);

            let local_items: Vec<ListItem> = state
                .locals
                .iter()
                .map(|c| ListItem::new(c.path.display().to_string()))
                .collect();
            frame.render_widget(
                List::new(local_items).block(Block::default().title("Local").borders(Borders::ALL)),
                columns[0],
            );

            let gist_items: Vec<ListItem> = state
                .ranked_gists()
                .into_iter()
                .map(|g| {
                    ListItem::new(format!(
                        "{} / {} ({})",
                        g.file.gist_id, g.file.filename, g.score
                    ))
                })
                .collect();
            frame.render_widget(
                List::new(gist_items).block(Block::default().title("Gists").borders(Borders::ALL)),
                columns[1],
            );

            frame.render_widget(
                Paragraph::new(
                    "Tab switch  Enter diff  u upload  d download  n create  p pin  q quit",
                )
                .block(Block::default().title("Commands").borders(Borders::ALL)),
                chunks[1],
            );
        })?;

        if let Event::Key(key) = event::read()? {
            if state.handle_key(key.code) == KeyOutcome::Quit {
                break;
            }
        }
    }

    Ok(())
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
    fn empty_state_has_no_ranked_gists() {
        let state = initial_state();
        assert!(state.ranked_gists().is_empty());
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
        );
        assert_eq!(state.screen, Screen::Diff);
        assert!(state.diff_previewed);
        assert_eq!(state.preview_remote, "remote body");
        assert_eq!(state.preview_local, PathBuf::from("/tmp/x"));
        assert_eq!(state.diff_scroll, 0);
    }

    #[test]
    fn back_to_list_clears_preview() {
        let mut state = initial_state();
        state.enter_diff("d".into(), "r".into(), PathBuf::from("/tmp/x"));
        state.back_to_list();
        assert_eq!(state.screen, Screen::List);
        assert!(!state.diff_previewed);
        assert!(state.diff_text.is_empty());
        assert!(state.preview_remote.is_empty());
        assert_eq!(state.preview_local, PathBuf::new());
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
        state.enter_diff("l1\nl2\nl3".into(), "r".into(), PathBuf::from("/tmp/x"));
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
    fn esc_in_diff_returns_to_list() {
        let mut state = initial_state();
        state.enter_diff("d".into(), "r".into(), PathBuf::from("/tmp/x"));
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::List);
        assert!(!state.diff_previewed);
    }

    #[test]
    fn d_in_diff_downloads_when_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.json");
        let mut state = initial_state();
        state.enter_diff("d".into(), "r".into(), missing);
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::Download);
    }

    #[test]
    fn d_in_diff_confirms_when_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("exists.json");
        std::fs::write(&existing, "old").unwrap();
        let mut state = initial_state();
        state.enter_diff("d".into(), "r".into(), existing);
        assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::ConfirmOverwrite);
    }

    #[test]
    fn confirm_y_returns_download() {
        let mut state = initial_state();
        state.enter_diff("d".into(), "r".into(), PathBuf::from("/tmp/x"));
        state.screen = Screen::ConfirmOverwrite;
        assert_eq!(state.handle_key(KeyCode::Char('y')), KeyOutcome::Download);
    }

    #[test]
    fn confirm_n_returns_to_diff() {
        let mut state = initial_state();
        state.enter_diff("d".into(), "r".into(), PathBuf::from("/tmp/x"));
        state.screen = Screen::ConfirmOverwrite;
        assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
        assert_eq!(state.screen, Screen::Diff);
    }

    #[test]
    fn q_in_diff_quits() {
        let mut state = initial_state();
        state.enter_diff("d".into(), "r".into(), PathBuf::from("/tmp/x"));
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
        state.enter_diff("d".into(), "r".into(), PathBuf::from("/tmp/x"));
        state.screen = Screen::ConfirmOverwrite;
        assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::Quit);
    }

    #[test]
    fn confirm_esc_returns_to_diff() {
        let mut state = initial_state();
        state.enter_diff("d".into(), "r".into(), PathBuf::from("/tmp/x"));
        state.screen = Screen::ConfirmOverwrite;
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert_eq!(state.screen, Screen::Diff);
    }
}
