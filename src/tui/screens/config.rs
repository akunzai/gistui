//! `Screen::Config` (Settings) — key handling, view-model, paint, and palette items
//! colocated in one file (issue #287, Phase 2).

use crate::tui::keys::{point_in, NavAction};
use crate::tui::view_model::{ChromeVm, ConfigVm};
use crate::tui::{
    AppState, ConfigField, HelpTopic, KeyOutcome, MouseLayout, PaneHit, Screen, Theme,
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
                if self.adjust_config_field(true) {
                    return KeyOutcome::PersistSettings;
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.adjust_config_field(false) {
                    return KeyOutcome::PersistSettings;
                }
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }

    /// Toggle or nudge the selected Config field. Returns true when a value changed
    /// (caller should persist). Pure aside from mutating `self`.
    pub(crate) fn adjust_config_field(&mut self, forward: bool) -> bool {
        let index = self.config().map(|c| c.index).unwrap_or(0);
        let field = ConfigField::ALL
            .get(index)
            .copied()
            .unwrap_or(ConfigField::Theme);
        match field {
            ConfigField::Theme => {
                self.theme_choice = match self.theme_choice {
                    crate::config::ThemeChoice::Dark => crate::config::ThemeChoice::Light,
                    crate::config::ThemeChoice::Light => crate::config::ThemeChoice::Dark,
                };
                self.theme = Theme::for_choice(self.theme_choice);
                true
            }
            ConfigField::Mouse => {
                self.config_mouse = !self.config_mouse;
                self.mouse_enabled = self.config_mouse && !self.no_mouse_cli;
                true
            }
            ConfigField::CheckUpdates => {
                self.config_check_updates = !self.config_check_updates;
                self.update_check_enabled = self.config_check_updates && !self.no_update_check_cli;
                true
            }
            ConfigField::DiffShowFull => {
                self.diff_show_full = !self.diff_show_full;
                true
            }
            ConfigField::IgnoreTrailingNewline => {
                self.ignore_trailing_newline = !self.ignore_trailing_newline;
                true
            }
            ConfigField::ScanDepth => {
                let next = if forward {
                    self.scan_depth.saturating_add(1).min(20)
                } else {
                    self.scan_depth.saturating_sub(1)
                };
                if next == self.scan_depth {
                    return false;
                }
                self.scan_depth = next;
                true
            }
            ConfigField::DiffContext => {
                let next = if forward {
                    self.diff_context.saturating_add(1).min(50)
                } else {
                    self.diff_context.saturating_sub(1)
                };
                if next == self.diff_context {
                    return false;
                }
                self.diff_context = next;
                true
            }
        }
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
    pub(crate) fn click_select_config(&mut self, col: u16, row: u16, layout: &MouseLayout) -> bool {
        let Screen::Config(cfg) = &mut self.screen else {
            return false;
        };
        if let Some(hit) = layout.list {
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
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
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
        .style(state.theme.base_style())
        .border_style(Style::default().fg(state.theme.accent));
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(state.theme.accent)
                .fg(state.theme.fg_on_accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if chrome.mouse_enabled {
        layout.close_button = Some(crate::tui::render_close_button(
            frame,
            chunks[0],
            &state.theme,
        ));
        layout.list = Some(PaneHit {
            rect: chunks[0],
            offset: 0,
        });
    }
    if let Some(ref status) = config.status {
        frame.render_widget(
            Paragraph::new(status.as_str()).style(Style::default().fg(state.theme.accent)),
            chunks[1],
        );
    }
    frame.render_widget(
        Paragraph::new(
            " File-only: skip_dirs · ~/.config/gistui/config.toml (or $XDG_CONFIG_HOME)",
        )
        .style(Style::default().fg(state.theme.dim)),
        chunks[2],
    );
}

pub(crate) fn config_palette_items() -> Vec<crate::tui::palette::PaletteItem> {
    use crate::tui::palette::key_item;
    vec![
        key_item("Enter", "Toggle / increase value", KeyCode::Enter, true),
        key_item("h/l", "Decrease / increase value", KeyCode::Char('l'), true),
        key_item("Esc", "Close settings", KeyCode::Esc, true),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::*;

    fn config_mut(state: &mut AppState) -> &mut ConfigState {
        if !state.screen.is_config() {
            state.screen = Screen::Config(Box::default());
        }
        state.config_mut().expect("expected Screen::Config")
    }

    #[test]
    fn adjust_config_mouse_updates_mouse_enabled_respecting_cli() {
        let mut state = initial_state();
        state.open_config();
        config_mut(&mut state).index = ConfigField::ALL
            .iter()
            .position(|f| *f == ConfigField::Mouse)
            .unwrap();
        state.no_mouse_cli = false;
        state.config_mouse = false;
        state.mouse_enabled = false;

        assert!(state.adjust_config_field(true));
        assert!(state.config_mouse);
        assert!(
            state.mouse_enabled,
            "toggling mouse on must enable session mouse when CLI does not force off"
        );

        // CLI --no-mouse still wins for the effective session flag.
        state.no_mouse_cli = true;
        state.config_mouse = false;
        state.mouse_enabled = false;
        assert!(state.adjust_config_field(true));
        assert!(state.config_mouse);
        assert!(
            !state.mouse_enabled,
            "--no-mouse must keep mouse_enabled false even when config prefers on"
        );
    }

    #[test]
    fn config_adjust_theme_returns_persist_settings() {
        let mut state = initial_state();
        state.open_config();
        // Theme is index 0
        config_mut(&mut state).index = 0;
        assert_eq!(state.theme_choice, crate::config::ThemeChoice::Dark);
        assert!(state.adjust_config_field(true));
        assert_eq!(state.theme_choice, crate::config::ThemeChoice::Light);
        // Space on config screen yields PersistSettings
        let outcome = state.handle_key(KeyCode::Char(' '));
        assert_eq!(outcome, KeyOutcome::PersistSettings);
    }

    #[test]
    fn config_adjust_scan_depth_clamps_and_reports_change() {
        let mut state = initial_state();
        state.open_config();
        config_mut(&mut state).index = ConfigField::ALL
            .iter()
            .position(|f| *f == ConfigField::ScanDepth)
            .unwrap();
        state.scan_depth = 0;
        assert!(!state.adjust_config_field(false)); // already min
        assert!(state.adjust_config_field(true));
        assert_eq!(state.scan_depth, 1);
    }

    #[test]
    fn config_toggles_show_full_diff() {
        let mut state = initial_state();
        state.open_config();
        config_mut(&mut state).index = ConfigField::ALL
            .iter()
            .position(|field| *field == ConfigField::DiffShowFull)
            .unwrap();
        assert!(!state.diff_show_full);
        assert!(state.adjust_config_field(true));
        assert!(state.diff_show_full);
    }
}
