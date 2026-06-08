use crate::actions::PendingAction;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    Local,
    Gist,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub locals: Vec<LocalCandidate>,
    pub gists: Vec<GistFile>,
    pub pinned: Vec<PinnedMapping>,
    pub focus: FocusPane,
    pub local_index: usize,
    pub gist_index: usize,
    pub diff_previewed: bool,
    pub pending_action: Option<PendingAction>,
}

impl AppState {
    pub fn ranked_gists(&self) -> Vec<RankedGistFile> {
        let Some(local) = self.locals.get(self.local_index) else {
            return Vec::new();
        };
        rank_gist_files(&local.path, &self.gists, &self.pinned)
    }

    pub fn handle_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('q') => return true,
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
            _ => {}
        }
        false
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
        diff_previewed: false,
        pending_action: None,
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
            if state.handle_key(key.code) {
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
}
