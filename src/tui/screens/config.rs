//! `Screen::Config` (Settings) — key handling, view-model, paint, and palette items
//! colocated in one file (issue #287, Phase 2).

use crate::tui::keys::{point_in, NavAction};
use crate::tui::view_model::{ChromeVm, ConfigVm};
use crate::tui::{
    AppState, ConfigField, HelpTopic, HitTarget, KeyOutcome, MouseFrame, PaneHit, PaneTarget,
    Screen,
};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::Config;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    1
}

impl AppState {
    pub(crate) fn handle_key_config(&mut self, code: KeyCode) -> KeyOutcome {
        let n = ConfigField::ALL.len();
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.leave();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(c) = self.config_mut() {
                    c.index = c.index.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(c) = self.config_mut() {
                    if c.index + 1 < n {
                        c.index += 1;
                    }
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(outcome) = self.adjust_config_field(true) {
                    return outcome;
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Some(outcome) = self.adjust_config_field(false) {
                    return outcome;
                }
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }

    /// Toggle or nudge the selected Config field and describe the persistence work.
    pub(crate) fn adjust_config_field(&mut self, forward: bool) -> Option<KeyOutcome> {
        let index = self.config().map(|c| c.index).unwrap_or(0);
        let field = ConfigField::ALL
            .get(index)
            .copied()
            .unwrap_or(ConfigField::Theme);
        let change = self.settings.adjust(field, forward)?;
        Some(KeyOutcome::PersistSettings {
            effect: change.effect,
            success_message: format!("{}: {}", field.label(), self.settings.field_value(field)),
        })
    }

    /// Arrow key navigation for `Screen::Config`: moves the field-index selection. Left/Right
    /// are handled in `handle_key_config` (adjusting a field needs `PersistSettings`).
    pub(crate) fn apply_navigation_config(&mut self, action: NavAction) -> bool {
        let Screen::Config(cfg) = &mut self.screen else {
            return false;
        };
        let n = ConfigField::ALL.len();
        match action {
            NavAction::Up => {
                cfg.index = cfg.index.saturating_sub(1);
            }
            NavAction::Down => {
                if cfg.index + 1 < n {
                    cfg.index += 1;
                }
            }
            NavAction::Left | NavAction::Right => {
                // Adjust is handled in handle_key_config (needs PersistSettings).
                return false;
            }
            _ => return false,
        }
        true
    }

    /// Select the clicked field row on `Screen::Config`. Returns `true` when a row was hit.
    pub(crate) fn click_select_config(&mut self, col: u16, row: u16, layout: &MouseFrame) -> bool {
        let Screen::Config(cfg) = &mut self.screen else {
            return false;
        };
        if let Some(hit) = layout.pane(PaneTarget::List) {
            if point_in(hit.rect, col, row) {
                if let Some(idx) = hit.index_at(row, ConfigField::ALL.len()) {
                    cfg.index = idx;
                    return true;
                }
            }
        }
        false
    }
}

/// Config/settings body — usable under Palette-over-Config as well.
pub(crate) fn build_config_vm(state: &AppState) -> ConfigVm {
    let rows = ConfigField::ALL
        .iter()
        .map(|field| {
            let label = field.label();
            let value = state.config_field_value(*field);
            let hint = if field.is_numeric() {
                "←/→"
            } else {
                "Enter"
            };
            format!("  {label:<23} {value:<5} {} ({hint})", field.description())
        })
        .collect();
    ConfigVm {
        rows,
        selected: state.config().map(|c| c.index).unwrap_or(0),
        status: state.status.clone(),
    }
}

pub(crate) fn render_config_vm(
    frame: &mut Frame,
    state: &AppState,
    config: &ConfigVm,
    chrome: &ChromeVm,
    layout: &mut MouseFrame,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(
        frame,
        area,
        &state.settings.theme(),
        chrome.mouse_enabled,
        layout,
    );
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    let items: Vec<ListItem> = config
        .rows
        .iter()
        .map(|row| ListItem::new(row.clone()))
        .collect();
    let mut list_state = ListState::default().with_selected(Some(config.selected));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(crate::tui::render::fit_block_title(
            " Settings ",
            chunks[0].width,
        ))
        .title_bottom(Line::from(
            " Esc close · Enter/←/→ change · saved on change ",
        ))
        .style(state.settings.theme().base_style())
        .border_style(Style::default().fg(state.settings.theme().accent));
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(state.settings.theme().accent)
                .fg(state.settings.theme().fg_on_accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if chrome.mouse_enabled {
        let close = crate::tui::render_close_button(frame, chunks[0], &state.settings.theme());
        layout.register(HitTarget::Close, close);
        layout.register_pane(
            PaneTarget::List,
            PaneHit {
                rect: chunks[0],
                offset: 0,
            },
            config.rows.len(),
        );
    }
    if let Some(ref status) = config.status {
        frame.render_widget(
            Paragraph::new(status.as_str())
                .style(Style::default().fg(state.settings.theme().accent)),
            chunks[1],
        );
    }
    frame.render_widget(
        Paragraph::new(
            " File-only: skip_dirs · ~/.config/gistui/config.toml (or $XDG_CONFIG_HOME)",
        )
        .style(Style::default().fg(state.settings.theme().dim)),
        chunks[2],
    );
}

#[cfg(test)]
mod tests {
    use crate::tui::*;
    use crossterm::event::KeyCode;

    fn config_mut(state: &mut AppState) -> &mut ConfigState {
        if !state.screen.is_config() {
            state.screen = Screen::Config(Box::default());
        }
        state.config_mut().expect("expected Screen::Config")
    }

    #[test]
    fn config_adjust_theme_returns_persist_settings() {
        let mut state = initial_state();
        state.open_config();
        assert_eq!(
            state.settings.theme_choice(),
            crate::config::ThemeChoice::Dark
        );
        let outcome = state.handle_key(KeyCode::Char(' '));
        assert_eq!(
            outcome,
            KeyOutcome::PersistSettings {
                effect: None,
                success_message: "Theme: light".into(),
            }
        );
        assert_eq!(
            state.settings.theme_choice(),
            crate::config::ThemeChoice::Light
        );
    }

    #[test]
    fn config_toggles_show_full_diff() {
        let mut state = initial_state();
        state.open_config();
        config_mut(&mut state).index = ConfigField::ALL
            .iter()
            .position(|field| *field == ConfigField::DiffShowFull)
            .unwrap();
        assert!(!state.settings.diff_show_full());
        assert_eq!(
            state.adjust_config_field(true),
            Some(KeyOutcome::PersistSettings {
                effect: None,
                success_message: "Show full diff: on".into(),
            })
        );
        assert!(state.settings.diff_show_full());
    }
}
