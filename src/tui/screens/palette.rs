//! `Screen::Palette` — key handling, view-model, paint, and palette items colocated in one
//! file (issue #287, Phase 2).

use crate::tui::palette::{CrossAction, PaletteExec, PaletteMode};
use crate::tui::view_model::{ChromeVm, PaletteVm};
use crate::tui::EditResult;
use crate::tui::{AppState, HelpTopic, HitTarget, KeyOutcome, MouseFrame, RowTarget, Screen};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::List;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    1
}

impl AppState {
    pub(crate) fn handle_key_palette(
        &mut self,
        code: KeyCode,
        _modifiers: KeyModifiers,
    ) -> KeyOutcome {
        let mode = self.palette().map(|p| p.mode).unwrap_or_default();
        if mode == PaletteMode::Command {
            match code {
                KeyCode::Esc => {
                    self.close_palette();
                    return KeyOutcome::None;
                }
                KeyCode::Up => {
                    if let Some(p) = self.palette_mut() {
                        p.selected = p.selected.saturating_sub(1);
                    }
                    return KeyOutcome::None;
                }
                KeyCode::Down => {
                    let len = self.palette_visible_items().len();
                    if let Some(p) = self.palette_mut() {
                        if len > 0 && p.selected + 1 < len {
                            p.selected += 1;
                        }
                    }
                    return KeyOutcome::None;
                }
                KeyCode::Enter => return self.execute_palette_selection(),
                _ => {
                    if let Some(p) = self.palette_mut() {
                        if let EditResult::Changed = p.query.apply_edit(code) {
                            p.selected = 0;
                        }
                    }
                    self.palette_clamp_selection();
                    return KeyOutcome::None;
                }
            }
        }

        // Menu mode: no query box — arrows pick a row, Enter runs it, Esc closes.
        match code {
            KeyCode::Esc | KeyCode::Char(';') => {
                self.close_palette();
                KeyOutcome::None
            }
            KeyCode::Up => {
                if let Some(p) = self.palette_mut() {
                    p.selected = p.selected.saturating_sub(1);
                }
                KeyOutcome::None
            }
            KeyCode::Down => {
                let len = self.palette_visible_items().len();
                if let Some(p) = self.palette_mut() {
                    if len > 0 && p.selected + 1 < len {
                        p.selected += 1;
                    }
                }
                KeyOutcome::None
            }
            KeyCode::Enter => self.execute_palette_selection(),
            _ => KeyOutcome::None,
        }
    }

    pub(crate) fn execute_palette_selection(&mut self) -> KeyOutcome {
        let selected = self.palette().map(|p| p.selected).unwrap_or(0);
        let item = self
            .palette_visible_items()
            .get(selected)
            .map(|i| (*i).clone());
        let Some(item) = item else {
            return KeyOutcome::None;
        };
        if !item.enabled {
            return KeyOutcome::None;
        }
        let exec = item.exec;
        let origin = self
            .palette()
            .map(|p| p.origin_screen.clone())
            .unwrap_or(Screen::List);
        self.close_palette();
        self.screen = origin;
        match exec {
            PaletteExec::Key(code, modifiers) => self.handle_key_with(code, modifiers),
            PaletteExec::Cross(CrossAction::GoToGists) => {
                self.open_gist_manager();
                KeyOutcome::None
            }
            PaletteExec::Cross(CrossAction::GoToPins) => {
                self.open_pins();
                KeyOutcome::None
            }
            PaletteExec::Cross(CrossAction::OpenHelp) => {
                self.open_help();
                KeyOutcome::None
            }
            PaletteExec::Cross(CrossAction::OpenConfig) => {
                self.open_config();
                KeyOutcome::None
            }
            PaletteExec::Cross(CrossAction::ToggleTheme) => {
                self.theme_choice = match self.theme_choice {
                    crate::config::ThemeChoice::Dark => crate::config::ThemeChoice::Light,
                    crate::config::ThemeChoice::Light => crate::config::ThemeChoice::Dark,
                };
                self.theme = crate::tui::Theme::for_choice(self.theme_choice);
                KeyOutcome::ThemeToggle
            }
            PaletteExec::Cross(CrossAction::Quit) => KeyOutcome::Quit,
        }
    }
}

pub(crate) fn build_palette_vm(state: &AppState) -> PaletteVm {
    let p = state.palette().cloned().unwrap_or_default();
    let background =
        crate::tui::view_model::build_background_screen_vm(state, &p.origin_screen).map(Box::new);
    let has_query = p.mode == PaletteMode::Command;
    let title = match p.mode {
        PaletteMode::Menu => "Menu",
        PaletteMode::Command => "Command palette",
    };
    let items: Vec<crate::tui::view_model::PaletteRowVm> = state
        .palette_visible_items()
        .into_iter()
        .map(|item| crate::tui::view_model::PaletteRowVm {
            key_hint: item.key_hint.clone(),
            label: item.label.clone(),
            category: item.category,
            enabled: item.enabled,
        })
        .collect();
    let key_width = items
        .iter()
        .map(|item| item.key_hint.chars().count())
        .max()
        .unwrap_or(1)
        .max(1);
    PaletteVm {
        background,
        title,
        has_query,
        query: p.query,
        selected: p.selected,
        items,
        key_width,
        mode: p.mode,
        anchor: p.anchor,
    }
}

pub(crate) fn render_palette_vm(
    frame: &mut Frame,
    state: &AppState,
    palette: &PaletteVm,
    chrome: &ChromeVm,
    layout: &mut MouseFrame,
    feedback: &mut crate::tui::render::RenderFeedback,
) {
    let mut bg_layout = MouseFrame::default();
    if let Some(background) = &palette.background {
        crate::tui::render::render_screen_vm_with_feedback(
            frame,
            state,
            background,
            chrome,
            &mut bg_layout,
            feedback,
        );
    }

    let area = frame.area();
    // A query with no matches still paints one "(no matches)" placeholder row, so the height
    // budget must reserve a line for it — otherwise the panel sizes to fit only the query input
    // and the placeholder is clipped, leaving what looks like an empty frame.
    let body_lines = palette.items.len().max(1) + usize::from(palette.has_query);
    let longest_row = palette
        .items
        .iter()
        .map(|item| 2 + palette.key_width + 2 + item.label.chars().count());
    let content_width = longest_row.max().unwrap_or(20) as u16;
    let width = if palette.has_query {
        (area.width * 70 / 100).clamp(
            content_width.saturating_add(4),
            area.width.saturating_sub(2).max(1),
        )
    } else {
        (area.width * 45 / 100).clamp(
            content_width.saturating_add(4),
            area.width.saturating_sub(2).max(1),
        )
    };
    let max_h = area.height.saturating_sub(2).max(1) as usize;
    let height = (body_lines + 2).clamp(3, max_h) as u16;
    let (x, y) = match (palette.mode, palette.anchor) {
        (PaletteMode::Menu, Some((col, row))) => (
            col.saturating_sub(width / 2)
                .min(area.width.saturating_sub(width)),
            row.saturating_sub(1)
                .min(area.height.saturating_sub(height)),
        ),
        _ => (
            area.width.saturating_sub(width) / 2,
            area.height.saturating_sub(height).saturating_sub(1),
        ),
    };
    let rect = Rect::new(x, y, width, height);

    frame.render_widget(Clear, rect);

    layout.intercept_all();
    let dim = Style::default().fg(state.theme.dim);
    let active = Style::default()
        .fg(state.theme.fg_on_accent)
        .bg(state.theme.accent)
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line<'static>> = Vec::new();
    if palette.has_query {
        lines.push(crate::tui::render::input_line("> ", &palette.query, ""));
    }
    if palette.items.is_empty() {
        lines.push(Line::from(Span::styled("  (no matches)", dim)));
    } else {
        for (i, item) in palette.items.iter().enumerate() {
            let row_style = if i == palette.selected {
                active
            } else if item.enabled {
                state.theme.base_style()
            } else {
                Style::default().fg(state.theme.dim)
            };
            lines.push(crate::tui::render::palette_row_spans(
                &item.key_hint,
                &item.label,
                item.category,
                palette.key_width,
                &state.theme,
                row_style,
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(state.theme.base_style()).block(
            Block::default()
                .title(crate::tui::render::fit_block_title(
                    palette.title,
                    rect.width,
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.theme.accent))
                .style(state.theme.base_style()),
        ),
        rect,
    );

    let inner = rect.inner(Margin::new(1, 1));
    let mut y = inner.y + u16::from(palette.has_query);
    for (index, item) in palette.items.iter().enumerate() {
        if y >= inner.bottom() {
            break;
        }
        if chrome.mouse_enabled && item.enabled {
            layout.register(
                HitTarget::Row(RowTarget::Palette(index)),
                Rect::new(inner.x, y, inner.width, 1),
            );
        }
        y = y.saturating_add(1);
    }
    if chrome.mouse_enabled {
        let close = crate::tui::render::render_close_button(frame, rect, &state.theme);
        layout.register(HitTarget::PaletteClose, close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::initial_state;
    use crate::tui::keymap::Category;
    use crate::tui::palette::{PaletteItem, PaletteState};
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn enabled_row_after_disabled_row_keeps_its_palette_index() {
        let mut state = initial_state();
        state.screen = Screen::Palette(Box::new(PaletteState {
            mode: PaletteMode::Command,
            items: vec![
                PaletteItem {
                    key_hint: "x".into(),
                    label: "Disabled".into(),
                    exec: PaletteExec::Cross(CrossAction::Quit),
                    enabled: false,
                    category: Category::Nav,
                    search: "x disabled".into(),
                },
                PaletteItem {
                    key_hint: "t".into(),
                    label: "Toggle theme".into(),
                    exec: PaletteExec::Cross(CrossAction::ToggleTheme),
                    enabled: true,
                    category: Category::Nav,
                    search: "t toggle theme".into(),
                },
            ],
            origin_screen: Screen::List,
            ..PaletteState::default()
        }));
        let vm = build_palette_vm(&state);
        let chrome = ChromeVm {
            mouse_enabled: true,
            bg_task_msg: None,
            spinner_frame: 0,
        };
        let mut mouse = MouseFrame::default();
        let mut feedback = crate::tui::render::RenderFeedback::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal
            .draw(|frame| render_palette_vm(frame, &state, &vm, &chrome, &mut mouse, &mut feedback))
            .unwrap();
        let (col, row) = (0..20)
            .flat_map(|row| (0..80).map(move |col| (col, row)))
            .find(|&(col, row)| {
                mouse.resolve(col, row) == Some(HitTarget::Row(RowTarget::Palette(1)))
            })
            .expect("enabled row hit");

        let outcome = state.palette_click(col, row, &mouse);

        assert_eq!(outcome, KeyOutcome::ThemeToggle);
        assert_eq!(state.theme_choice, crate::config::ThemeChoice::Light);
    }
}
