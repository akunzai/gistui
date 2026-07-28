//! `Screen::Pins` — key handling, view-model, paint, and palette items colocated in one
//! file (issue #287, Phase 2).

use crate::tui::view_model::{ChromeVm, PinRowVm, PinsEmptyKind, PinsVm};
use crate::tui::{AppState, HelpTopic, KeyOutcome, MouseLayout, PaneHit, Screen};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Padding},
    Frame,
};

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::Pins;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    1
}

/// Shared "would this key actually do something" predicate for `Screen::Pins`, mirrored by
/// both [`AppState::handle_key_pins`]'s match-arm guards and `pins_palette_items` so the two
/// can never silently drift (issue #288).
pub(crate) fn pins_guard(state: &AppState, code: KeyCode) -> bool {
    let has_pin = !state.pinned.is_empty() && state.selected_pin_index().is_some();
    match code {
        KeyCode::Enter => {
            has_pin
                && state.selected_pin_index().is_some_and(|idx| {
                    let pin = &state.pinned[idx];
                    crate::tui::keys::diff_pair_previewable(
                        state,
                        &pin.gist_id,
                        &pin.gist_filename,
                        Some(pin.local_path.as_path()),
                    )
                })
        }
        KeyCode::Char('x' | 's' | 'u' | 'd') => has_pin,
        _ => false,
    }
}

impl AppState {
    pub(crate) fn handle_key_pins(&mut self, code: KeyCode) -> KeyOutcome {
        // One-shot: any key dismisses a lingering sync status; the run_loop IO helper for this
        // key may set a fresh one afterwards (e.g. "already in sync").
        self.status = None;
        // Inline text filter: live-navigate with arrows; Tab is a no-op (single pane).
        if self.pins().is_some_and(|p| p.filtering) {
            let len = self.visible_pin_indices().len();
            if let Some(pins) = self.pins_mut() {
                crate::tui::keys::handle_inline_filter_key(
                    code,
                    &mut pins.cursor,
                    &mut pins.filter_query,
                    &mut pins.filtering,
                    len,
                );
            }
            return KeyOutcome::None;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.screen = Screen::List,
            KeyCode::Char('/') => {
                if let Some(pins) = self.pins_mut() {
                    pins.filtering = true;
                }
            }
            KeyCode::Enter if pins_guard(self, code) => {
                let Some(index) = self.selected_pin_index() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::PreviewPinDiff { index };
            }
            KeyCode::Char('x') if pins_guard(self, code) => {
                let Some(index) = self.selected_pin_index() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::UnpinAtPin { index };
            }
            KeyCode::Char('s') if pins_guard(self, code) => {
                let Some(index) = self.selected_pin_index() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::SyncPinAuto { index };
            }
            KeyCode::Char('u') if pins_guard(self, code) => {
                let Some(index) = self.selected_pin_index() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::SyncPinPush { index };
            }
            KeyCode::Char('d') if pins_guard(self, code) => {
                let Some(index) = self.selected_pin_index() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::SyncPinPull { index };
            }
            KeyCode::Char('o') => {
                if let Some(pins) = self.pins_mut() {
                    pins.sort = pins.sort.next();
                    pins.cursor.reset();
                }
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PinLabelParams<'a> {
    pub icon: &'a str,
    pub local_path: &'a std::path::Path,
    pub gist_id: &'a str,
    pub gist_filename: &'a str,
    pub local_age: &'a str,
    pub gist_age: &'a str,
}

/// Builds a single Pins-screen row. The local path is rendered with `display_path`
/// (home → `~`) so it stays readable; the full row is horizontally scrollable. Pure so
/// the path-shortening is unit-testable without a frame.
pub(crate) fn pin_row_label(params: PinLabelParams<'_>) -> String {
    format!(
        "{}  {}  ↔  {} / {}   (local {} · gist {})",
        params.icon,
        crate::config::display_path(params.local_path),
        params.gist_id,
        params.gist_filename,
        params.local_age,
        params.gist_age,
    )
}

/// Pins body only — usable under Palette-over-Pins as well.
pub(crate) fn build_pins_vm(state: &AppState) -> PinsVm {
    let pins = state.pins().cloned().unwrap_or_default();
    let (footer_title, footer, footer_colored) = if pins.filtering {
        (
            "Filter (↑↓ move · Enter apply · Esc clear)".to_string(),
            format!("/{}_", pins.filter_query),
            false,
        )
    } else {
        let (footer, colored) =
            crate::tui::footer_with_status(state.status.as_deref(), crate::tui::MINIMAL_HINT);
        (String::new(), footer, colored)
    };

    let visible = state.visible_pin_indices();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let (empty, rows) = if state.pinned.is_empty() {
        (PinsEmptyKind::NoMappings, Vec::new())
    } else if visible.is_empty() {
        (PinsEmptyKind::NoFilterMatch, Vec::new())
    } else {
        let rows = visible
            .iter()
            .map(|&i| {
                let m = &state.pinned[i];
                let entry = state.cached_pin_sync_entry(i);
                let status = entry.status;
                let age = |ts: Option<u64>| {
                    ts.map(|t| crate::domain::humanize_age(now - t as i64))
                        .unwrap_or_else(|| "?".to_string())
                };
                let local_age = if status == crate::domain::SyncStatus::Missing {
                    "missing".to_string()
                } else {
                    age(entry.local_ts)
                };
                let label = pin_row_label(PinLabelParams {
                    icon: status.icon(),
                    local_path: &m.local_path,
                    gist_id: &m.gist_id,
                    gist_filename: &m.gist_filename,
                    local_age: &local_age,
                    gist_age: &age(entry.remote_ts),
                });
                PinRowVm {
                    pin_index: i,
                    status,
                    label,
                }
            })
            .collect();
        (PinsEmptyKind::HasRows, rows)
    };

    let mut title = format!(
        "Pinned Mappings {}",
        crate::tui::render::count_label(visible.len(), state.pinned.len())
    );
    if !pins.filter_query.is_empty() {
        title.push_str(&format!(" · /{}", pins.filter_query));
    }
    if pins.sort != crate::tui::PinSort::Default {
        title.push_str(&format!(" · sort:{}", pins.sort.label()));
    }

    PinsVm {
        title,
        empty,
        rows,
        selected: (!visible.is_empty()).then_some(pins.cursor.index),
        filtering: pins.filtering,
        filter_query: pins.filter_query.clone(),
        footer_title,
        footer,
        footer_colored,
        hscroll: pins.cursor.hscroll,
    }
}

pub(crate) fn render_pins_vm(
    frame: &mut Frame,
    state: &AppState,
    pins: &PinsVm,
    chrome: &ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    // Sync feedback (e.g. "already in sync") is carried in the Pins VM footer (see #72 / #241).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(crate::tui::render::footer_height(
                &pins.footer,
                area.width,
                &pins.footer_title,
            )),
        ])
        .split(area);

    let items: Vec<ListItem> = match pins.empty {
        PinsEmptyKind::NoMappings => {
            vec![
                ListItem::new("  📌 No pinned mappings found (use p to pin a pair)")
                    .style(Style::default().fg(state.theme.dim)),
            ]
        }
        PinsEmptyKind::NoFilterMatch => {
            vec![ListItem::new("  🔍 No pins match the filter")
                .style(Style::default().fg(state.theme.dim))]
        }
        PinsEmptyKind::HasRows => pins
            .rows
            .iter()
            .map(|row| {
                let item = ListItem::new(crate::tui::render::hscroll_str(&row.label, pins.hscroll));
                if row.status == crate::domain::SyncStatus::Missing {
                    item.style(Style::default().fg(state.theme.del_color))
                } else {
                    item
                }
            })
            .collect(),
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(pins.title.clone())
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.theme.accent))
                .style(state.theme.base_style())
                .padding(Padding::horizontal(1)),
        )
        .style(state.theme.base_style())
        .highlight_style(
            Style::default()
                .bg(state.theme.accent)
                .fg(state.theme.fg_on_accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(pins.selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if chrome.mouse_enabled {
        layout.list = Some(PaneHit {
            rect: chunks[0],
            offset: list_state.offset(),
        });
    }

    if pins.filtering {
        crate::tui::render::render_footer_line(
            frame,
            chunks[1],
            &pins.footer_title,
            crate::tui::render::input_line("/", &pins.filter_query, ""),
            &state.theme,
            layout,
        );
    } else {
        crate::tui::render_footer(
            frame,
            chunks[1],
            &pins.footer_title,
            &pins.footer,
            pins.footer_colored,
            &state.theme,
            layout,
        );
    }
    if chrome.mouse_enabled {
        layout.close_button = Some(crate::tui::render_close_button(frame, area, &state.theme));
    }
}

pub(crate) fn pins_palette_items(state: &AppState) -> Vec<crate::tui::palette::PaletteItem> {
    use crate::tui::palette::key_item;
    let g = |code| pins_guard(state, code);
    vec![
        key_item(
            "Enter",
            "Diff pinned pair",
            KeyCode::Enter,
            g(KeyCode::Enter),
        ),
        key_item("s", "Smart-sync", KeyCode::Char('s'), g(KeyCode::Char('s'))),
        key_item("u", "Force push", KeyCode::Char('u'), g(KeyCode::Char('u'))),
        key_item("d", "Force pull", KeyCode::Char('d'), g(KeyCode::Char('d'))),
        key_item("x", "Unpin pair", KeyCode::Char('x'), g(KeyCode::Char('x'))),
        key_item("/", "Filter pins", KeyCode::Char('/'), true),
        key_item("o", "Cycle sort", KeyCode::Char('o'), true),
        key_item("q", "Back to list", KeyCode::Char('q'), true),
        key_item("?", "Help", KeyCode::Char('?'), true),
    ]
}
