//! `Screen::Pins` — key handling, view-model, paint, and palette items colocated in one
//! file (issue #287, Phase 2).

use crate::tui::keys::{apply_list_cursor_nav, point_in, NavAction};
use crate::tui::render::list_pane::render_list_pane;
use crate::tui::view_model::{
    ChromeVm, ListPaneEmpty, ListPaneVm, PaneTitleVm, PinsVm, RowEmphasis, RowVm,
};
use crate::tui::{AppState, HelpTopic, KeyOutcome, MouseLayout, Screen};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::Pins;
const PINS_STATUS_LEGEND: &str =
    "✓ synced · ↑ local newer · ↓ remote newer · ✕ missing · ? unknown";

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

    /// Arrow / hjkl / page-key navigation for `Screen::Pins`'s list cursor. Precomputes
    /// len/hmax with `&self`, then mutates the cursor (issue #274: cannot hold
    /// `&mut PinsState` from `match &mut self.screen` while calling helpers).
    pub(crate) fn apply_navigation_pins(&mut self, action: NavAction) -> bool {
        let len = self.visible_pin_indices().len();
        let hmax = self.pins_hscroll_max();
        let Some(pins) = self.pins_mut() else {
            return false;
        };
        apply_list_cursor_nav(&mut pins.cursor, action, len, hmax);
        true
    }

    /// Select the clicked row on `Screen::Pins`, moving the list cursor. Returns `true` when
    /// a row was hit.
    pub(crate) fn click_select_pins(&mut self, col: u16, row: u16, layout: &MouseLayout) -> bool {
        if let Some(hit) = layout.list {
            if point_in(hit.rect, col, row) {
                let count = self.visible_pin_indices().len();
                if let Some(idx) = hit.index_at(row, count) {
                    if let Some(pins) = self.pins_mut() {
                        pins.cursor.select(idx);
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PinLabelParams<'a> {
    pub icon: &'a str,
    pub local_path: &'a std::path::Path,
    pub gist_id: &'a str,
    pub gist_description: Option<&'a str>,
    pub gist_filename: &'a str,
    pub local_age: &'a str,
    pub gist_age: &'a str,
}

/// Builds a single Pins-screen row. The local path is rendered with `display_path`
/// (home → `~`) so it stays readable; the full row is horizontally scrollable. Pure so
/// the path-shortening is unit-testable without a frame.
///
/// The gist description leads the identity (falling back to the filename alone when the gist
/// has none), and the id trails as a fixed-width abbreviation — the full id no longer sits
/// between the local path and filename dominating the row (issue #347).
pub(crate) fn pin_row_label(params: PinLabelParams<'_>) -> String {
    let identity = match params.gist_description.filter(|d| !d.trim().is_empty()) {
        Some(desc) => format!("{desc} / {}", params.gist_filename),
        None => params.gist_filename.to_string(),
    };
    format!(
        "{}  {}  ↔  {}   (local {} · gist {} · #{})",
        params.icon,
        crate::config::display_path(params.local_path),
        identity,
        params.local_age,
        params.gist_age,
        crate::tui::render::short_gist_id(params.gist_id),
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
        let hints = crate::tui::keymap::footer_hints(&state.screen);
        let (footer, colored) = crate::tui::footer_with_status(state.status.as_deref(), &hints);
        (PINS_STATUS_LEGEND.to_string(), footer, colored)
    };

    let visible = state.visible_pin_indices();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let (empty, empty_message, rows) = if state.pinned.is_empty() {
        (
            ListPaneEmpty::NoItems,
            Some("  📌 No pinned mappings found (use p to pin a pair)".to_string()),
            Vec::new(),
        )
    } else if visible.is_empty() {
        (
            ListPaneEmpty::NoFilterMatch,
            Some("  🔍 No pins match the filter".to_string()),
            Vec::new(),
        )
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
                let gist_description = state.group_by_id(&m.gist_id).map(|g| g.description);
                let label = pin_row_label(PinLabelParams {
                    icon: status.icon(),
                    local_path: &m.local_path,
                    gist_id: &m.gist_id,
                    gist_description: gist_description.as_deref(),
                    gist_filename: &m.gist_filename,
                    local_age: &local_age,
                    gist_age: &age(entry.remote_ts),
                });
                RowVm {
                    label,
                    emphasis: if status == crate::domain::SyncStatus::Missing {
                        RowEmphasis::Danger
                    } else {
                        RowEmphasis::None
                    },
                }
            })
            .collect();
        (ListPaneEmpty::HasRows, None, rows)
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
        pane: ListPaneVm {
            title: PaneTitleVm::new(title),
            focused: true,
            selected: (!visible.is_empty()).then_some(pins.cursor.index),
            empty,
            empty_message,
            rows,
            hscroll: pins.cursor.hscroll,
            scrollbar: false,
        },
        filtering: pins.filtering,
        filter_query: pins.filter_query.clone(),
        footer_title,
        footer,
        footer_colored,
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
                pins.footer_colored,
            )),
        ])
        .split(area);

    render_list_pane(
        frame,
        chunks[0],
        &pins.pane,
        &state.theme,
        chrome.mouse_enabled,
        &mut layout.list,
    );

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
            crate::tui::keymap::for_screen(&state.screen),
            &state.theme,
        );
    }
    if chrome.mouse_enabled {
        layout.close_button = Some(crate::tui::render_close_button(frame, area, &state.theme));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::*;

    use crate::tui::tests::{pins_mut, pins_ref, pins_state_with_long_home_path, set_pending};

    #[test]
    fn pins_key_clears_lingering_status_for_one_shot_display() {
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
        state.status = Some("already in sync".into());
        state.handle_key(KeyCode::Up); // any key
        assert_eq!(state.status, None);
    }

    #[test]
    fn confirm_upload_n_cancels_to_diff_return_screen() {
        let mut state = initial_state();
        state.pending_return = Some(Screen::Pins(Box::default()));
        set_pending(
            &mut state,
            PendingAction::Upload {
                gist_id: "a".into(),
                filename: "settings.json".into(),
                local_path: PathBuf::from("/tmp/settings.json"),
            },
        );

        assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
        assert!(state.pending_action().is_none());
        assert!(
            state.screen.is_pins(),
            "cancelling an upload initiated from Pins must return to Pins, not always List"
        );
    }

    #[test]
    fn pins_screen_sync_keys_emit_outcomes() {
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("/tmp/a.txt"),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
        assert!(matches!(
            state.handle_key(KeyCode::Char('s')),
            KeyOutcome::SyncPinAuto { .. }
        ));
        assert!(matches!(
            state.handle_key(KeyCode::Char('u')),
            KeyOutcome::SyncPinPush { .. }
        ));
        assert!(matches!(
            state.handle_key(KeyCode::Char('d')),
            KeyOutcome::SyncPinPull { .. }
        ));
        assert!(matches!(
            state.handle_key(KeyCode::Char('x')),
            KeyOutcome::UnpinAtPin { .. }
        ));
    }

    #[test]
    fn pins_screen_enter_emits_preview_pin_diff() {
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("/tmp/a.txt"),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
        assert!(matches!(
            state.handle_key(KeyCode::Enter),
            KeyOutcome::PreviewPinDiff { .. }
        ));
    }

    #[test]
    fn pins_screen_enter_is_blocked_for_non_previewable_pair() {
        // Issue #288: `pins_palette_items`'s "Diff pinned pair" already checked previewability,
        // but `handle_key_pins`'s real Enter arm did not — a binary/image pinned pair showed
        // disabled in the palette yet still diffed on a direct keypress. Now shared via
        // `pins_guard`, so the key press is blocked the same way the palette already was.
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("/tmp/logo.png"),
            gist_id: "g1".into(),
            gist_filename: "logo.png".into(),
            direction: None,
            last_seen_hash: None,
        }];
        assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
    }

    #[test]
    fn pins_hscroll_starts_at_zero() {
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
        assert_eq!(pins_ref(&state).cursor.hscroll, 0);
    }

    #[test]
    fn pins_right_scrolls_then_clamps_at_a_bound() {
        let mut state = pins_state_with_long_home_path();
        state.handle_key(KeyCode::Right);
        assert_eq!(
            pins_ref(&state).cursor.hscroll,
            1,
            "Right should advance the scroll"
        );
        // Far past the end clamps to a stable maximum (does not run away).
        for _ in 0..500 {
            state.handle_key(KeyCode::Right);
        }
        let clamped = pins_ref(&state).cursor.hscroll;
        state.handle_key(KeyCode::Right);
        assert_eq!(
            pins_ref(&state).cursor.hscroll,
            clamped,
            "scroll must clamp at its max"
        );
        assert!(clamped > 0, "a long path must be scrollable");
    }

    #[test]
    fn pins_left_clamps_at_zero() {
        let mut state = pins_state_with_long_home_path();
        state.handle_key(KeyCode::Right);
        state.handle_key(KeyCode::Left);
        state.handle_key(KeyCode::Left);
        assert_eq!(pins_ref(&state).cursor.hscroll, 0);
    }

    #[test]
    fn pins_hscroll_resets_when_selection_moves() {
        let mut state = pins_state_with_long_home_path();
        state.pinned.push(PinnedMapping {
            local_path: PathBuf::from("/tmp/b.txt"),
            gist_id: "g2".into(),
            gist_filename: "b.txt".into(),
            direction: None,
            last_seen_hash: None,
        });
        state.handle_key(KeyCode::Right);
        assert!(pins_ref(&state).cursor.hscroll > 0);
        state.handle_key(KeyCode::Down);
        assert_eq!(
            pins_ref(&state).cursor.hscroll,
            0,
            "moving selection resets hscroll"
        );
    }

    fn state_with_pins(rows: &[(&str, &str, &str)]) -> AppState {
        let mut state = initial_state();
        state.cwd = PathBuf::from("/cwd");
        state.screen = Screen::Pins(Box::default());
        state.pinned = rows
            .iter()
            .map(|(lp, id, fname)| PinnedMapping {
                local_path: PathBuf::from(lp),
                gist_id: (*id).into(),
                gist_filename: (*fname).into(),
                direction: None,
                last_seen_hash: None,
            })
            .collect();
        state
    }

    #[test]
    fn visible_pin_indices_filters_by_path_and_filename() {
        let mut state = state_with_pins(&[
            ("/cwd/.zshrc", "g1", "zshrc"),
            ("/cwd/init.lua", "g2", "init.lua"),
            ("/cwd/notes.md", "g3", "notes.md"),
        ]);
        assert_eq!(state.visible_pin_indices(), vec![0, 1, 2]);

        pins_mut(&mut state).filter_query = "lua".into(); // matches filename of row 1
        assert_eq!(state.visible_pin_indices(), vec![1]);

        pins_mut(&mut state).filter_query = "ZSH".into(); // case-insensitive, matches path of row 0
        assert_eq!(state.visible_pin_indices(), vec![0]);
    }

    #[test]
    fn selected_pin_index_maps_through_filter() {
        let mut state = state_with_pins(&[
            ("/cwd/alpha", "g1", "alpha"),
            ("/cwd/beta", "g2", "beta"),
            ("/cwd/gamma", "g3", "gamma"),
        ]);
        pins_mut(&mut state).filter_query = "gamma".into(); // only row 2 visible
        pins_mut(&mut state).cursor.index = 0; // first (and only) visible row
        assert_eq!(state.selected_pin_index(), Some(2)); // TRUE index, not 0
    }

    #[test]
    fn pins_down_clamps_to_filtered_count() {
        let mut state = state_with_pins(&[
            ("/cwd/a", "g1", "a"),
            ("/cwd/blua", "g2", "blua"),
            ("/cwd/c", "g3", "c"),
        ]);
        pins_mut(&mut state).filter_query = "lua".into(); // 1 visible
        state.handle_key(KeyCode::Down);
        assert_eq!(pins_ref(&state).cursor.index, 0); // clamped to the single filtered row
    }

    #[test]
    fn pins_filter_input_behaviors() {
        let mut state = state_with_pins(&[("/cwd/a", "g1", "a"), ("/cwd/b", "g2", "b")]);
        pins_mut(&mut state).filtering = true;

        // live nav while typing
        state.handle_key(KeyCode::Down);
        assert_eq!(pins_ref(&state).cursor.index, 1);
        assert!(pins_ref(&state).filtering);

        // Tab is a no-op (single pane)
        state.handle_key(KeyCode::Char('a'));
        state.handle_key(KeyCode::Tab);
        assert!(pins_ref(&state).filtering);
        assert_eq!(pins_ref(&state).filter_query, "a");

        // Esc clears + exits
        state.handle_key(KeyCode::Esc);
        assert!(!pins_ref(&state).filtering);
        assert_eq!(pins_ref(&state).filter_query, "");

        // Backspace on empty exits
        pins_mut(&mut state).filtering = true;
        state.handle_key(KeyCode::Backspace);
        assert!(!pins_ref(&state).filtering);

        // Enter keeps query + exits
        pins_mut(&mut state).filtering = true;
        state.handle_key(KeyCode::Char('b'));
        state.handle_key(KeyCode::Enter);
        assert!(!pins_ref(&state).filtering);
        assert_eq!(pins_ref(&state).filter_query, "b");
    }

    #[test]
    fn pins_page_keys_jump_selection() {
        use crossterm::event::KeyModifiers;
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
        state.pinned = (0..12)
            .map(|i| PinnedMapping {
                local_path: PathBuf::from(format!("/cwd/p{i}.txt")),
                gist_id: format!("g{i}"),
                gist_filename: format!("f{i}.txt"),
                direction: None,
                last_seen_hash: None,
            })
            .collect();
        state.handle_key_with(KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert_eq!(pins_ref(&state).cursor.index, 10);
        state.handle_key(KeyCode::PageUp);
        assert_eq!(pins_ref(&state).cursor.index, 0);
    }

    #[test]
    fn pins_click_selects_and_double_click_matches_enter() {
        let mut state = state_with_pins(&[("a.txt", "g1", "a.txt"), ("b.txt", "g2", "b.txt")]);
        let hit = PaneHit {
            rect: Rect::new(0, 0, 40, 10),
            offset: 0,
        };
        let layout = MouseLayout {
            list: Some(hit),
            ..Default::default()
        };
        let out = state.handle_mouse(MouseInput::Click { col: 5, row: 2 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(pins_ref(&state).cursor.index, 1);
        let mut by_key = state.clone();
        let key_out = by_key.handle_key(KeyCode::Enter);
        let by_mouse = state.handle_mouse(MouseInput::DoubleClick { col: 5, row: 2 }, &layout);
        assert_eq!(by_mouse, key_out);
    }

    #[test]
    fn palette_esc_returns_to_origin() {
        let mut state = crate::tui::initial_state();
        state.screen = Screen::Pins(Box::default());
        state.open_palette_menu(None);
        assert!(state.screen.is_palette());
        state.handle_key(KeyCode::Esc);
        assert!(state.screen.is_pins());
    }
}
