//! `Screen::Gists` — key handling, view-model, paint, and palette items colocated in one
//! file (issue #287, Phase 2).

use crate::tui::keys::{apply_list_cursor_nav, point_in, NavAction};
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
const GISTS_HINTS: &str = "↑↓ move  ·  Enter detail  ·  * star  ·  o browser  ·  y copy URL  ·  H revisions  ·  s sort  ·  v visibility  ·  / filter  ·  Esc/q back";

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

    /// Arrow / hjkl / page-key navigation for `Screen::Gists`'s list cursor. Precomputes
    /// len/hmax with `&self`, then mutates the cursor (issue #274: cannot hold
    /// `&mut GistsManagerState` from `match &mut self.screen` while calling helpers).
    pub(crate) fn apply_navigation_gists(&mut self, action: NavAction) -> bool {
        let len = self.visible_gist_groups().len();
        let hmax = self.gists_hscroll_max();
        let Some(gm) = self.gist_manager_mut() else {
            return false;
        };
        apply_list_cursor_nav(&mut gm.cursor, action, len, hmax);
        true
    }

    /// Select the clicked row on `Screen::Gists`, moving the list cursor. Returns `true` when
    /// a row was hit.
    pub(crate) fn click_select_gists(&mut self, col: u16, row: u16, layout: &MouseLayout) -> bool {
        if let Some(hit) = layout.list {
            if point_in(hit.rect, col, row) {
                let count = self.visible_gist_groups().len();
                if let Some(idx) = hit.index_at(row, count) {
                    if let Some(gm) = self.gist_manager_mut() {
                        gm.cursor.select(idx);
                        return true;
                    }
                }
            }
        }
        false
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
            crate::tui::footer_with_status(state.status.as_deref(), GISTS_HINTS);
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
                gists.footer_colored,
            )),
        ])
        .split(area);

    let items: Vec<ListItem> = match gists.empty {
        GistsEmptyKind::HasRows => gists
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                ListItem::new(crate::tui::render::visible_list_row(
                    &row.label,
                    crate::tui::render::row_hscroll(gists.selected, i, gists.hscroll),
                    chunks[0].width,
                ))
            })
            .collect(),
        _ => {
            let msg = gists.empty_message.clone().unwrap_or_else(|| "  ".into());
            vec![ListItem::new(msg).style(Style::default().fg(state.theme.dim))]
        }
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(crate::tui::render::fit_block_title(
                    &gists.title,
                    chunks[0].width,
                ))
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
        .highlight_symbol(crate::tui::render::LIST_HIGHLIGHT_SYMBOL);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::*;

    use crate::tui::tests::{gists_mut, set_pending, state_with_gists, state_with_two_gists};

    fn gists_ref(state: &AppState) -> &GistsManagerState {
        state.gist_manager().expect("expected Screen::Gists")
    }

    #[test]
    fn enter_on_gist_opens_detail() {
        let mut state = state_with_gists();
        state.screen = Screen::Gists(Box::default());
        let outcome = state.handle_key(KeyCode::Enter);
        assert!(matches!(outcome, KeyOutcome::OpenGistDetail { .. }));
    }

    #[test]
    fn context_gist_id_uses_group_cursor_on_gists_screen() {
        let mut state = state_with_gists();
        state.screen = Screen::Gists(Box::default());
        gists_mut(&mut state).cursor.index = 0;
        assert_eq!(
            state.context_gist_id(),
            state.selected_group().map(|g| g.id)
        );
    }

    #[test]
    fn o_in_gist_view_opens_browser() {
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists(Box::default());
        assert!(matches!(
            state.handle_key(KeyCode::Char('o')),
            KeyOutcome::OpenBrowser { .. }
        ));
    }

    #[test]
    fn compact_confirm_y_executes_and_n_returns_to_gist_manager() {
        let mut state = state_with_two_gists();
        // CompactAnalyze parks the restore target before Confirm opens (staged pending_return).
        state.pending_return = Some(Screen::Gists(Box::default()));
        set_pending(
            &mut state,
            PendingAction::CompactGist {
                gist_id: "a".into(),
                label: "My Ghostty config".into(),
                count: 3,
            },
        );
        assert_eq!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::ExecuteCompactGist
        );

        // Re-open confirm for the cancel path (y does not leave Confirm until IO runs).
        state.pending_return = Some(Screen::Gists(Box::default()));
        set_pending(
            &mut state,
            PendingAction::CompactGist {
                gist_id: "a".into(),
                label: "My Ghostty config".into(),
                count: 3,
            },
        );
        // Cancelling drops the pending action and lands back on the parked restore target.
        state.handle_key(KeyCode::Char('n'));
        assert!(state.screen.is_gists());
        assert!(state.pending_action().is_none());
    }

    #[test]
    fn g_opens_gist_view_landing_on_the_selected_files_gist() {
        let mut state = state_with_two_gists();
        // Select the second gist's row in the main (file) list, then jump to the
        // gist-level view; it should land on that same gist.
        state.gist_index = 1;
        assert_eq!(state.handle_key(KeyCode::Char('g')), KeyOutcome::None);
        assert!(state.screen.is_gists());
        assert_eq!(gists_ref(&state).cursor.index, 1);
        assert_eq!(state.selected_group().unwrap().id, "b");
    }

    #[test]
    fn gist_view_v_cycles_visibility_filter() {
        // state_with_two_gists: gist "a" is public, gist "b" is secret.
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists(Box::default());
        assert_eq!(state.visible_gist_groups().len(), 2);

        state.handle_key(KeyCode::Char('v')); // -> public
        let vis = state.visible_gist_groups();
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].id, "a");

        state.handle_key(KeyCode::Char('v')); // -> secret
        let vis = state.visible_gist_groups();
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].id, "b");

        state.handle_key(KeyCode::Char('v')); // -> starred (empty source)
        assert_eq!(state.visible_gist_groups().len(), 0);

        state.handle_key(KeyCode::Char('v')); // -> forked (none here)
        assert_eq!(state.visible_gist_groups().len(), 0);

        state.handle_key(KeyCode::Char('v')); // -> all
        assert_eq!(state.visible_gist_groups().len(), 2);
    }

    #[test]
    fn gist_view_filter_narrows_then_esc_clears() {
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists(Box::default());
        state.handle_key(KeyCode::Char('/'));
        assert!(gists_ref(&state).filtering);
        for c in "ssh".chars() {
            state.handle_key(KeyCode::Char(c));
        }
        let vis = state.visible_gist_groups();
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].id, "b"); // "SSH config"

        state.handle_key(KeyCode::Esc);
        assert!(!gists_ref(&state).filtering);
        assert!(gists_ref(&state).filter_query.is_empty());
        assert_eq!(state.visible_gist_groups().len(), 2);
    }

    #[test]
    fn gist_view_s_cycles_sort_updated_then_created() {
        let mut state = initial_state();
        state.screen = Screen::Gists(Box::default());
        state.gists = vec![
            GistFile {
                gist_id: "old-upd".into(),
                description: "x".into(),
                filename: "f".into(),
                public: false,
                updated_at: "2026-01-01T00:00:00Z".into(),
                created_at: "2026-12-01T00:00:00Z".into(),
                owner_login: String::new(),
                fork_of_id: None,

                raw_url: None,

                content_type: None,

                node_id: None,
            },
            GistFile {
                gist_id: "new-upd".into(),
                description: "y".into(),
                filename: "g".into(),
                public: false,
                updated_at: "2026-06-01T00:00:00Z".into(),
                created_at: "2026-02-01T00:00:00Z".into(),
                owner_login: String::new(),
                fork_of_id: None,

                raw_url: None,

                content_type: None,

                node_id: None,
            },
        ];
        // Default: sort by updated (newest first).
        assert_eq!(gists_ref(&state).sort, GistGroupSort::Updated);
        assert_eq!(state.visible_gist_groups()[0].id, "new-upd");
        // s -> sort by created (newest created first).
        state.handle_key(KeyCode::Char('s'));
        assert_eq!(gists_ref(&state).sort, GistGroupSort::Created);
        assert_eq!(state.visible_gist_groups()[0].id, "old-upd");
    }

    #[test]
    fn gist_view_left_right_scrolls_horizontally() {
        let mut state = state_with_two_gists();
        state.screen = Screen::Gists(Box::default());
        assert_eq!(gists_ref(&state).cursor.hscroll, 0);
        state.handle_key(KeyCode::Right);
        assert_eq!(gists_ref(&state).cursor.hscroll, 1);
        state.handle_key(KeyCode::Left);
        assert_eq!(gists_ref(&state).cursor.hscroll, 0);
        // Left at the origin saturates at 0.
        state.handle_key(KeyCode::Left);
        assert_eq!(gists_ref(&state).cursor.hscroll, 0);
    }

    fn gists_screen_state() -> AppState {
        let mut state = initial_state();
        state.gists = vec![
            GistFile {
                gist_id: "g1".into(),
                description: "alpha".into(),
                filename: "a.txt".into(),
                public: false,
                updated_at: "2026-06-10T00:00:00Z".into(),
                created_at: "2026-06-01T00:00:00Z".into(),
                owner_login: String::new(),
                fork_of_id: None,

                raw_url: None,

                content_type: None,

                node_id: None,
            },
            GistFile {
                gist_id: "g2".into(),
                description: "beta".into(),
                filename: "b.txt".into(),
                public: false,
                updated_at: "2026-06-10T00:00:00Z".into(),
                created_at: "2026-06-01T00:00:00Z".into(),
                owner_login: String::new(),
                fork_of_id: None,

                raw_url: None,

                content_type: None,

                node_id: None,
            },
        ];
        state.screen = Screen::Gists(Box::default());
        state
    }

    #[test]
    fn gists_filter_navigates_while_typing() {
        let mut state = gists_screen_state();
        gists_mut(&mut state).filtering = true;

        state.handle_key(KeyCode::Down);
        assert_eq!(gists_ref(&state).cursor.index, 1);
        assert!(gists_ref(&state).filtering);
        state.handle_key(KeyCode::Up);
        assert_eq!(gists_ref(&state).cursor.index, 0);
    }

    #[test]
    fn gists_filter_empty_backspace_exits() {
        let mut state = gists_screen_state();
        gists_mut(&mut state).filtering = true;

        state.handle_key(KeyCode::Char('a'));
        state.handle_key(KeyCode::Backspace); // empty again, still filtering
        assert!(gists_ref(&state).filtering);
        state.handle_key(KeyCode::Backspace); // empty -> exit
        assert!(!gists_ref(&state).filtering);
    }

    #[test]
    fn gists_filter_tab_is_noop() {
        let mut state = gists_screen_state();
        gists_mut(&mut state).filtering = true;
        state.handle_key(KeyCode::Char('a'));

        state.handle_key(KeyCode::Tab);
        assert!(gists_ref(&state).filtering); // still typing
        assert_eq!(gists_ref(&state).filter_query, "a"); // unchanged
    }

    #[test]
    fn gists_page_keys_jump_selection() {
        let mut state = initial_state();
        state.screen = Screen::Gists(Box::default());
        state.gists = (0..12)
            .map(|i| GistFile {
                gist_id: format!("g{i}"),
                description: format!("gist {i}"),
                filename: "a.txt".into(),
                public: true,
                updated_at: "x".into(),
                created_at: "x".into(),
                owner_login: String::new(),
                fork_of_id: None,
                raw_url: None,
                content_type: None,
                node_id: None,
            })
            .collect();
        state.handle_key(KeyCode::PageDown);
        assert_eq!(gists_ref(&state).cursor.index, 10);
        state.handle_key(KeyCode::PageDown);
        assert_eq!(gists_ref(&state).cursor.index, 11);
    }

    #[test]
    fn fork_key_ignored_on_list_and_gist_manager() {
        let mut state = initial_state();
        state.current_user_login = Some("me".into());
        state.starred_gists = vec![GistFile {
            gist_id: "foreign".into(),
            description: "x".into(),
            filename: "a.txt".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: "other".into(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        }];
        state.gist_type_filter = GistTypeFilter::Starred;
        state.gist_index = 0;
        assert_eq!(state.handle_key(KeyCode::Char('F')), KeyOutcome::None);

        state.screen = Screen::Gists(Box::default());
        gists_mut(&mut state).type_filter = GistTypeFilter::Starred;
        gists_mut(&mut state).cursor.index = 0;
        assert_eq!(state.handle_key(KeyCode::Char('F')), KeyOutcome::None);
    }

    #[test]
    fn gists_click_selects_and_double_click_matches_enter() {
        let mut state = gists_screen_state(); // 2 groups, Screen::Gists
        gists_mut(&mut state).cursor.index = 0;
        let hit = PaneHit {
            rect: Rect::new(0, 0, 40, 10),
            offset: 0,
        };
        let layout = MouseLayout {
            list: Some(hit),
            ..Default::default()
        };
        // Row 2 is the 2nd content row (border at row 0) -> idx 1.
        let out = state.handle_mouse(MouseInput::Click { col: 5, row: 2 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(gists_ref(&state).cursor.index, 1);
        // Double-click activates the same row, exactly as Enter would.
        let mut by_key = state.clone();
        let key_out = by_key.handle_key(KeyCode::Enter);
        let by_mouse = state.handle_mouse(MouseInput::DoubleClick { col: 5, row: 2 }, &layout);
        assert_eq!(by_mouse, key_out);
        assert!(matches!(by_mouse, KeyOutcome::OpenGistDetail { .. }));
    }
}
