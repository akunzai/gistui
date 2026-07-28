//! `Screen::Gists` — key handling, view-model, paint, and palette items colocated in one
//! file (issue #287, Phase 2).

use crate::tui::view_model::{ChromeVm, GistGroupRowVm, GistsEmptyKind, GistsVm};
use crate::tui::{AppState, HelpTopic, KeyOutcome, MouseLayout, PaneHit, Screen};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Padding},
    Frame,
};

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::GistManager;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    1
}

/// Shared "would this key actually do something" predicate for `Screen::Gists`, mirrored by
/// both [`AppState::handle_key_gists`]'s match-arm guards and `gists_palette_items` so the two
/// can never silently drift (issue #288).
pub(crate) fn gists_guard(state: &AppState, code: KeyCode) -> bool {
    let has_sel = state.gist_manager().map(|g| g.cursor.index).unwrap_or(0)
        < state.visible_gist_groups().len();
    match code {
        KeyCode::Enter | KeyCode::Char('o' | 'y' | 'H' | '*') => has_sel,
        _ => false,
    }
}

impl AppState {
    pub(crate) fn handle_key_gists(&mut self, code: KeyCode) -> KeyOutcome {
        self.status = None;
        // Inline text filter: live-navigate with arrows; Tab is a no-op (single pane).
        if self.gist_manager().is_some_and(|g| g.filtering) {
            let len = self.visible_gist_groups().len();
            if let Some(gm) = self.gist_manager_mut() {
                crate::tui::keys::handle_inline_filter_key(
                    code,
                    &mut gm.cursor,
                    &mut gm.filter_query,
                    &mut gm.filtering,
                    len,
                );
            }
            return KeyOutcome::None;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.screen = Screen::List,
            KeyCode::Char('/') => {
                if let Some(gm) = self.gist_manager_mut() {
                    gm.filtering = true;
                }
            }
            KeyCode::Char('s') => {
                if let Some(gm) = self.gist_manager_mut() {
                    gm.sort = gm.sort.next();
                    gm.cursor.reset();
                }
            }
            KeyCode::Char('v') => {
                if let Some(gm) = self.gist_manager_mut() {
                    gm.type_filter = gm.type_filter.next();
                    gm.cursor.reset();
                }
            }
            // Not gated through `gists_guard`: `star_toggle_intent` already has its own
            // complete "select a gist first" message for the no-selection case.
            KeyCode::Char('*') => return self.star_toggle_intent(),
            KeyCode::Enter if gists_guard(self, code) => {
                let Some(group) = self.selected_group() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::OpenGistDetail {
                    gist_id: group.id.clone(),
                };
            }
            KeyCode::Char('o') if gists_guard(self, code) => {
                let Some(gist_id) = self.context_gist_id() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::OpenBrowser { gist_id };
            }
            KeyCode::Char('y') if gists_guard(self, code) => {
                let Some(gist_id) = self.context_gist_id() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::CopyGistUrl { gist_id };
            }
            KeyCode::Char('H') if gists_guard(self, code) => {
                if self.open_revisions() {
                    if let Some(gist_id) = self.revision().and_then(|r| r.gist_id.clone()) {
                        return KeyOutcome::FetchRevisions { gist_id };
                    }
                }
            }
            KeyCode::Char('?') => self.open_help(),
            _ => {}
        }
        KeyOutcome::None
    }
}

/// Gists manager body — usable under Palette-over-Gists as well.
pub(crate) fn build_gists_vm(state: &AppState) -> GistsVm {
    let gm = state.gist_manager().cloned().unwrap_or_default();
    let (footer_title, footer, footer_colored) = if gm.filtering {
        (
            "Filter (↑↓ move · Enter apply · Esc clear)".to_string(),
            format!("/{}_", gm.filter_query),
            false,
        )
    } else {
        let (footer, colored) =
            crate::tui::footer_with_status(state.status.as_deref(), crate::tui::MINIMAL_HINT);
        (String::new(), footer, colored)
    };

    let groups = state.visible_gist_groups();
    let total_groups = state.gist_groups().len();
    let now = crate::tui::render::unix_now();

    let (empty, empty_message, rows) = if groups.is_empty() {
        if total_groups == 0 {
            (
                GistsEmptyKind::NoGists,
                Some("  📭 No gists found".into()),
                Vec::new(),
            )
        } else {
            (
                GistsEmptyKind::NoFilterMatch,
                Some("  🔍 No gists match the filter".into()),
                Vec::new(),
            )
        }
    } else {
        let rows = groups
            .iter()
            .map(|g| GistGroupRowVm {
                gist_id: g.id.clone(),
                label: crate::tui::render::gist_group_row_label(
                    g,
                    now,
                    gm.sort,
                    (
                        state.gist_comment_counts.get(&g.id).copied().unwrap_or(0),
                        state.gist_star_counts.get(&g.id).copied().unwrap_or(0),
                        state.gist_fork_counts.get(&g.id).copied().unwrap_or(0),
                    ),
                    state.gist_is_starred(&g.id),
                    state.current_user_login.as_deref(),
                ),
            })
            .collect();
        (GistsEmptyKind::HasRows, None, rows)
    };

    let mut title = format!(
        "Gists {}  ·  sort:{}  ·  type:{}  ·  ★ {}  ·  ⑂ {}",
        crate::tui::render::count_label(groups.len(), total_groups),
        gm.sort.label(),
        gm.type_filter.label(),
        state.starred_gist_count(),
        state.owned_fork_gist_count()
    );
    if !gm.filter_query.is_empty() {
        title.push_str(&format!("  ·  /{}", gm.filter_query));
    }

    GistsVm {
        title,
        empty,
        empty_message,
        rows,
        selected: (!groups.is_empty()).then_some(gm.cursor.index),
        filtering: gm.filtering,
        filter_query: gm.filter_query.clone(),
        footer_title,
        footer,
        footer_colored,
        hscroll: gm.cursor.hscroll,
    }
}

pub(crate) fn render_gists_vm(
    frame: &mut Frame,
    state: &AppState,
    gists: &GistsVm,
    chrome: &ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    // Footer: filter input while filtering, else status or hints (see #72 / #250).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(crate::tui::render::footer_height(
                &gists.footer,
                area.width,
                &gists.footer_title,
            )),
        ])
        .split(area);

    let items: Vec<ListItem> = match gists.empty {
        GistsEmptyKind::HasRows => gists
            .rows
            .iter()
            .map(|row| ListItem::new(crate::tui::render::hscroll_str(&row.label, gists.hscroll)))
            .collect(),
        _ => {
            let msg = gists.empty_message.clone().unwrap_or_else(|| "  ".into());
            vec![ListItem::new(msg).style(Style::default().fg(state.theme.dim))]
        }
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(gists.title.clone())
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
    list_state.select(gists.selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if chrome.mouse_enabled {
        layout.list = Some(PaneHit {
            rect: chunks[0],
            offset: list_state.offset(),
        });
    }

    if gists.filtering {
        crate::tui::render::render_footer_line(
            frame,
            chunks[1],
            &gists.footer_title,
            crate::tui::render::input_line("/", &gists.filter_query, ""),
            &state.theme,
            layout,
        );
    } else {
        crate::tui::render_footer(
            frame,
            chunks[1],
            &gists.footer_title,
            &gists.footer,
            gists.footer_colored,
            &state.theme,
            layout,
        );
    }
    if chrome.mouse_enabled {
        layout.close_button = Some(crate::tui::render_close_button(frame, area, &state.theme));
    }
}

pub(crate) fn gists_palette_items(state: &AppState) -> Vec<crate::tui::palette::PaletteItem> {
    use crate::tui::palette::key_item;
    let g = |code| gists_guard(state, code);
    vec![
        key_item(
            "Enter",
            "Open gist detail",
            KeyCode::Enter,
            g(KeyCode::Enter),
        ),
        key_item(
            "o",
            "Open in browser",
            KeyCode::Char('o'),
            g(KeyCode::Char('o')),
        ),
        key_item(
            "y",
            "Copy gist URL",
            KeyCode::Char('y'),
            g(KeyCode::Char('y')),
        ),
        key_item(
            "H",
            "Revision history",
            KeyCode::Char('H'),
            g(KeyCode::Char('H')),
        ),
        key_item(
            "*",
            "Star / unstar gist",
            KeyCode::Char('*'),
            g(KeyCode::Char('*')),
        ),
        key_item("/", "Filter gists", KeyCode::Char('/'), true),
        key_item("s", "Cycle sort", KeyCode::Char('s'), true),
        key_item("v", "Cycle visibility", KeyCode::Char('v'), true),
        key_item("q", "Back to list", KeyCode::Char('q'), true),
        key_item("?", "Help", KeyCode::Char('?'), true),
    ]
}
