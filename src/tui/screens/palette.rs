//! `Screen::Palette` — key handling, view-model, paint, and palette items colocated in one
//! file (issue #287, Phase 2).

use crate::tui::palette::{CrossAction, PaletteExec, PaletteMode};
use crate::tui::screens::{lookup, ScreenVm};
use crate::tui::view_model::ChromeVm;
use crate::tui::{
    AppState, ConfigField, HelpTopic, HitTarget, KeyOutcome, MouseFrame, RowTarget, Screen,
};
use crate::tui::{EditResult, TextInput};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

/// Command palette / context menu overlay (#250). `background` is the already-built ViewModel
/// for the screen underneath (issue #272) — `None` for a Confirm-origin (still unpainted, #277)
/// or Palette-origin (unreachable: the palette can't be opened while itself active).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PaletteVm {
    pub background: Option<Box<ScreenVm>>,
    pub title: &'static str,
    pub has_query: bool,
    /// Live query text + cursor, painted as the input line in Command mode
    /// (`has_query`) — carried here so paint never reads `state.palette()` directly.
    pub query: TextInput,
    pub selected: usize,
    pub items: Vec<PaletteRowVm>,
    pub key_width: usize,
    pub mode: PaletteMode,
    pub anchor: Option<(u16, u16)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaletteRowVm {
    pub key_hint: String,
    pub label: String,
    /// What the action risks, resolved by the keymap table so paint only looks it up.
    pub category: crate::tui::keymap::Category,
    pub enabled: bool,
}

/// ViewModel for the screen a palette is covering, by its origin's tag. `state`'s accessors
/// (`config()`/`help()`/etc., #242) already resolve through a palette-parked payload, so these
/// build fns are called directly rather than on `p.origin_screen` itself.
///
/// `None` for Confirm (blank background preserved as-is, tracked separately in #277) and Palette
/// (unreachable — the palette can't be opened while itself active).
pub(crate) fn build_background_screen_vm(state: &AppState, origin: &Screen) -> Option<ScreenVm> {
    match origin {
        Screen::Confirm(_) | Screen::Palette(_) => None,
        other => Some((lookup(other).build_vm)(state)),
    }
}

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
                let change = self.settings.adjust(ConfigField::Theme, true).unwrap();
                KeyOutcome::PersistSettings {
                    effect: change.effect,
                    success_message: format!(
                        "Theme: {}",
                        self.settings.field_value(ConfigField::Theme)
                    ),
                }
            }
            PaletteExec::Cross(CrossAction::Quit) => KeyOutcome::Quit,
        }
    }
}

pub(crate) fn build_palette_vm(state: &AppState) -> PaletteVm {
    let p = state.palette().cloned().unwrap_or_default();
    let background = build_background_screen_vm(state, &p.origin_screen).map(Box::new);
    let has_query = p.mode == PaletteMode::Command;
    let title = match p.mode {
        PaletteMode::Menu => "Menu",
        PaletteMode::Command => "Command palette",
    };
    let items: Vec<PaletteRowVm> = state
        .palette_visible_items()
        .into_iter()
        .map(|item| PaletteRowVm {
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
    let dim = Style::default().fg(state.settings.theme().dim);
    let active = Style::default()
        .fg(state.settings.theme().fg_on_accent)
        .bg(state.settings.theme().accent)
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
                state.settings.theme().base_style()
            } else {
                Style::default().fg(state.settings.theme().dim)
            };
            lines.push(crate::tui::render::palette_row_spans(
                &item.key_hint,
                &item.label,
                item.category,
                palette.key_width,
                &state.settings.theme(),
                row_style,
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(state.settings.theme().base_style())
            .block(
                Block::default()
                    .title(crate::tui::render::fit_block_title(
                        palette.title,
                        rect.width,
                    ))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(state.settings.theme().accent))
                    .style(state.settings.theme().base_style()),
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
        let close = crate::tui::render::render_close_button(frame, rect, &state.settings.theme());
        layout.register(HitTarget::PaletteClose, close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::initial_state;
    use crate::tui::keymap::Category;
    use crate::tui::palette::{PaletteItem, PaletteState};
    use crate::tui::PendingAction;
    use ratatui::{backend::TestBackend, Terminal};
    use std::path::PathBuf;

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
    fn palette_vm_items_and_title() {
        use crate::tui::palette::{PaletteExec, PaletteItem, PaletteMode, PaletteState};
        use crossterm::event::KeyCode;

        let mut state = initial_state();
        state.screen = Screen::Palette(Box::new(PaletteState {
            mode: PaletteMode::Menu,
            items: vec![PaletteItem {
                key_hint: "d".into(),
                category: crate::tui::keymap::Category::Write,
                label: "download".into(),
                exec: PaletteExec::Key(KeyCode::Char('d'), crossterm::event::KeyModifiers::empty()),
                enabled: true,
                search: "d download".into(),
            }],
            selected: 0,
            origin_screen: Screen::List,
            anchor: Some((10, 5)),
            ..PaletteState::default()
        }));
        let p = build_palette_vm(&state);
        assert_eq!(p.title, "Menu");
        match p.background.as_deref() {
            Some(ScreenVm::List(_)) => {}
            other => panic!("expected List background, got {other:?}"),
        }
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.items[0].label, "download");
        assert!(p.items[0].enabled);
        assert_eq!(p.anchor, Some((10, 5)));
    }

    #[test]
    fn palette_vm_background_stays_blank_over_confirm() {
        let mut state = initial_state();
        state.enter_confirm(
            PendingAction::Upload {
                gist_id: "g1".into(),
                filename: "notes.txt".into(),
                local_path: PathBuf::from("notes.txt"),
            },
            String::new(),
        );
        // `;` (menu) has no items over Confirm and won't open (palette.rs:60-66); Ctrl+P
        // (command palette) always opens because it also carries the cross-screen items.
        state.open_palette_command();
        let p = build_palette_vm(&state);
        assert!(p.background.is_none());
    }
}
