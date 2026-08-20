use super::*;

/// Test helper: mutable HelpState when on Help (creates empty Help if needed).
pub(super) fn help_mut(state: &mut AppState) -> &mut HelpState {
    if !state.screen.is_help() {
        state.screen = Screen::Help(Box::default());
    }
    match &mut state.screen {
        Screen::Help(h) => h.as_mut(),
        _ => unreachable!(),
    }
}
pub(super) fn help_ref(state: &AppState) -> &HelpState {
    state.help().expect("expected Screen::Help")
}
pub(super) fn revision_ref(state: &AppState) -> &RevisionState {
    state.revision().expect("expected Screen::Revisions")
}
pub(super) fn pins_mut(state: &mut AppState) -> &mut PinsState {
    if !state.screen.is_pins() {
        state.screen = Screen::Pins(Box::default());
    }
    state.pins_mut().expect("expected Screen::Pins")
}
pub(super) fn pins_ref(state: &AppState) -> &PinsState {
    state.pins().expect("expected Screen::Pins")
}
pub(super) fn gists_mut(state: &mut AppState) -> &mut GistsManagerState {
    if !state.screen.is_gists() {
        state.screen = Screen::Gists(Box::default());
    }
    state.gist_manager_mut().expect("expected Screen::Gists")
}
pub(super) fn detail_mut(state: &mut AppState) -> &mut DetailState {
    if !state.screen.is_gist_detail() {
        state.screen = Screen::GistDetail(Box::default());
    }
    state.detail_mut().expect("expected Screen::GistDetail")
}

/// Open Confirm with the given action (keeps existing body text when already on Diff/Confirm).
/// Mirrors production `enter_confirm`/`enter_confirm_from_diff`: a staged `pending_return`
/// becomes the cancel path (via `AppState::enter`) when present, otherwise the live screen does.
pub(super) fn set_pending(state: &mut AppState, action: PendingAction) {
    if state.screen.is_confirm() {
        if let Some(c) = state.confirm_mut() {
            c.action = action;
        }
        return;
    }
    if state.screen.is_diff() {
        state.enter_confirm_from_diff(action);
        return;
    }
    state.enter_confirm(action, String::new());
}

pub(super) fn set_diff_body(state: &mut AppState, text: impl Into<String>) {
    let text = text.into();
    if let Some(t) = state.diff_body_text_mut() {
        *t = text;
        return;
    }
    // Ensure a Diff payload exists for tests that set body before navigating.
    state.screen = Screen::Diff(Box::new(DiffState {
        text,
        ..DiffState::default()
    }));
}

pub(super) fn set_diff_scroll(state: &mut AppState, scroll: u16) {
    match &mut state.screen {
        Screen::Diff(d) => d.scroll = scroll,
        Screen::Confirm(c) => c.scroll = scroll,
        _ => {
            state.screen = Screen::Diff(Box::new(DiffState {
                scroll,
                ..DiffState::default()
            }));
        }
    }
}

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::path::PathBuf;

pub(super) fn state_with_gists() -> AppState {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            description: "demo".into(),
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            ..GistFile::fixture("g1", "a.txt")
        },
        GistFile {
            description: "demo".into(),
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            ..GistFile::fixture("g1", "b.txt")
        },
    ];
    gists_mut(&mut state).cursor.index = 0;
    state
}

pub(super) fn state_with_local_paths(paths: &[&str]) -> AppState {
    let mut state = initial_state();
    state.cwd = PathBuf::from("/cwd");
    state.locals = paths
        .iter()
        .map(|p| LocalCandidate {
            path: PathBuf::from(p),
            pinned: false,
            modified: None,
        })
        .collect();
    state
}

/// Issue #348: the diff header's gist side must show the real update time when the gist is
/// already loaded in memory (e.g. it's listed in Gist manager or Pinned Mappings), not
/// `(unknown)` — the throwaway `GistFileRef` used to fetch/sync carries no `updated_at` of
/// its own, so `gist_file_for_diff` must fill it in from the owned/starred lists.
#[test]
fn gist_file_for_diff_fills_updated_at_from_loaded_gists() {
    let state = state_with_gists();
    let file = GistFileRef::id_name("g1", "a.txt");
    let resolved = state.gist_file_for_diff(&file);
    assert_eq!(resolved.updated_at, "2026-06-10T00:00:00Z");

    let (_, gist_label) = diff_labels(None, &resolved);
    assert!(
        gist_label.contains("2026-06-10 00:00 UTC"),
        "expected the real timestamp in the header, got {gist_label}"
    );
    assert!(!gist_label.contains("unknown"), "got {gist_label}");
}

/// Issue #348: `(unknown)` is still shown, but only when the gist genuinely isn't loaded
/// anywhere in memory — not as the default for every diff.
#[test]
fn gist_file_for_diff_falls_back_to_unknown_for_an_unloaded_gist() {
    let state = initial_state();
    let file = GistFileRef::id_name("never-loaded", "a.txt");
    let resolved = state.gist_file_for_diff(&file);
    let (_, gist_label) = diff_labels(None, &resolved);
    assert!(gist_label.contains("(unknown)"), "got {gist_label}");
}

/// Issue #348: the lookup must also cover starred (not just owned) gists — one of the call
/// sites this fix replaced only checked `state.gists`, so a starred gist's diff header still
/// showed `(unknown)` even though its age was visible in Pinned Mappings.
#[test]
fn gist_file_for_diff_finds_a_starred_gist_too() {
    let mut state = initial_state();
    state.starred_gists = vec![GistFile {
        description: "starred demo".into(),
        public: true,
        updated_at: "2026-06-12T08:00:00Z".into(),
        created_at: "2026-06-01T00:00:00Z".into(),
        owner_login: "someone-else".into(),
        ..GistFile::fixture("s1", "notes.md")
    }];
    let file = GistFileRef::id_name("s1", "notes.md");
    let resolved = state.gist_file_for_diff(&file);
    assert_eq!(resolved.updated_at, "2026-06-12T08:00:00Z");
}

#[test]
fn local_filter_matches_filename_and_relative_path() {
    let mut state =
        state_with_local_paths(&["/cwd/settings.json", "/cwd/src/main.rs", "/cwd/notes.txt"]);

    assert_eq!(state.visible_locals().len(), 3);

    state.local_filter_query = "json".into();
    let visible: Vec<_> = state
        .visible_locals()
        .iter()
        .map(|r| r.candidate.path.clone())
        .collect();
    assert_eq!(visible, vec![PathBuf::from("/cwd/settings.json")]);

    state.local_filter_query = "src/".into();
    let visible: Vec<_> = state
        .visible_locals()
        .iter()
        .map(|r| r.candidate.path.clone())
        .collect();
    assert_eq!(visible, vec![PathBuf::from("/cwd/src/main.rs")]);

    state.local_filter_query = "NOTES".into();
    assert_eq!(state.visible_locals().len(), 1);
}

#[test]
fn local_down_clamps_to_filtered_count() {
    let mut state = state_with_local_paths(&["/cwd/a.json", "/cwd/b.txt", "/cwd/c.txt"]);
    state.focus = FocusPane::Local;
    state.local_filter_query = "json".into(); // only 1 match

    state.handle_key(KeyCode::Down); // would move to index 1 if clamped on raw len
    assert_eq!(state.local_index, 0); // clamped: only one visible row
}

pub(super) fn list_state_with_matches() -> AppState {
    let mut state = initial_state();
    state.locals = vec![
        LocalCandidate {
            path: std::path::PathBuf::from("/cwd/settings.json"),
            pinned: false,
            modified: None,
        },
        LocalCandidate {
            path: std::path::PathBuf::from("/cwd/other.txt"),
            pinned: false,
            modified: None,
        },
    ];
    state.gists = vec![
        GistFile {
            description: "Zed".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "settings.json")
        },
        GistFile {
            description: "misc".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("b", "zzz.txt")
        },
    ];
    state.local_index = 0;
    state.gist_index = 0;
    state
}

#[test]
fn anchor_defaults_to_local() {
    assert_eq!(initial_state().anchor, FocusPane::Local);
}

#[test]
fn gist_ranking_follows_anchor_not_focus() {
    let mut state = list_state_with_matches();
    state.anchor = FocusPane::Local;
    state.local_index = 0; // settings.json
    state.focus = FocusPane::Gist; // focus moved away, but anchor still Local
    let ranked = state.ranked_gists();
    assert_eq!(ranked[0].file.filename, "settings.json");
}

#[test]
fn a_key_toggles_anchor_and_resets_ranked_pane() {
    let mut state = list_state_with_matches();
    assert_eq!(state.anchor, FocusPane::Local);
    state.local_index = 1;
    state.local_hscroll = 3;
    state.handle_key(KeyCode::Char('a'));
    assert_eq!(state.anchor, FocusPane::Gist);
    // anchor now Gist → local is the newly-ranked (non-driver) pane → reset to top.
    assert_eq!(state.local_index, 0);
    assert_eq!(state.local_hscroll, 0);
}

#[test]
fn a_key_toggle_reverse_direction_resets_gist() {
    let mut state = list_state_with_matches();
    state.anchor = FocusPane::Gist;
    state.gist_index = 1;
    state.gist_hscroll = 4;
    state.handle_key(KeyCode::Char('a'));
    assert_eq!(state.anchor, FocusPane::Local);
    assert_eq!(state.gist_index, 0);
    assert_eq!(state.gist_hscroll, 0);
}

#[test]
fn moving_driver_pane_up_resets_ranked_pane() {
    let mut state = list_state_with_matches();
    state.anchor = FocusPane::Local;
    state.focus = FocusPane::Local;
    state.local_index = 1; // >0 so Up fires
    state.gist_index = 1;
    state.handle_key(KeyCode::Up);
    assert_eq!(state.local_index, 0);
    assert_eq!(state.gist_index, 0);
}

#[test]
fn moving_ranked_pane_does_not_reset_driver() {
    let mut state = list_state_with_matches();
    state.anchor = FocusPane::Local; // Local drives
    state.local_index = 0;
    state.focus = FocusPane::Gist; // picking in the ranked gist pane
    state.handle_key(KeyCode::Down);
    assert_eq!(state.gist_index, 1);
    assert_eq!(state.local_index, 0); // driver NOT reset
}

#[test]
fn moving_driver_pane_resets_ranked_pane() {
    let mut state = list_state_with_matches();
    state.anchor = FocusPane::Local;
    state.focus = FocusPane::Local; // moving the driver
    state.gist_index = 1;
    state.handle_key(KeyCode::Down);
    assert_eq!(state.local_index, 1);
    assert_eq!(state.gist_index, 0); // ranked pane reset to top
}

#[test]
fn detail_q_returns_to_gists() {
    let mut state = state_with_gists();
    // Mirrors what `enter()` does when GistDetail is opened from Gists (OpenGistDetail).
    state.nav_stack.push(Screen::Gists(Box::default()));
    state.screen = Screen::GistDetail(Box::default());
    state.handle_key(KeyCode::Char('q'));
    assert!(state.screen.is_gists());
}

#[test]
fn preview_q_returns_to_launch_screen() {
    let mut state = state_with_gists();
    state.nav_stack.push(Screen::GistDetail(Box::default()));
    state.screen = Screen::Preview(Box::default());
    state.handle_key(KeyCode::Char('q'));
    assert!(state.screen.is_gist_detail());
    // nav_stack is now drained, so a later list-launched preview isn't left pointing here.
    assert!(state.nav_stack.is_empty());
}

#[test]
fn file_list_scroll_keeps_cursor_visible() {
    // count <= visible: no scroll.
    assert_eq!(file_list_scroll(0, 5, 3), 0);
    assert_eq!(file_list_scroll(2, 5, 3), 0);
    // cursor within the first window: no scroll.
    assert_eq!(file_list_scroll(2, 5, 20), 0);
    assert_eq!(file_list_scroll(4, 5, 20), 0);
    // cursor past the window: scroll so cursor is the last visible row.
    assert_eq!(file_list_scroll(5, 5, 20), 1);
    assert_eq!(file_list_scroll(19, 5, 20), 15);
    // visible_rows == 0: never panic, offset 0.
    assert_eq!(file_list_scroll(19, 0, 20), 0);
}

#[test]
fn footer_with_status_prefers_status_else_colourised_hints() {
    // A one-shot status message wins and is shown plain (not key-colourised).
    let (msg, colored) = footer_with_status(Some("already in sync"), "↑↓ move · q back");
    assert_eq!(msg, "already in sync");
    assert!(!colored);
    // Otherwise the key hints render, colourised.
    let (hint, colored) = footer_with_status(None, "↑↓ move · q back");
    assert_eq!(hint, "↑↓ move · q back");
    assert!(colored);
}

#[test]
fn detail_focus_tab_tracks_focus() {
    assert_eq!(detail_focus_tab(DetailFocus::Files), 0);
    assert_eq!(detail_focus_tab(DetailFocus::Comments), 1);
}

#[test]
fn count_label_plain_unless_filtered() {
    assert_eq!(count_label(12, 12), "(12)");
    assert_eq!(count_label(0, 0), "(0)");
    // Filtered: fewer shown than total.
    assert_eq!(count_label(3, 12), "(3/12)");
}

#[test]
fn spinner_glyph_cycles_through_frames_and_wraps() {
    // Adjacent ticks advance the frame; the cycle wraps after a full revolution.
    assert_ne!(spinner_glyph(0), spinner_glyph(1));
    assert_eq!(spinner_glyph(0), spinner_glyph(10));
    assert_eq!(spinner_glyph(3), spinner_glyph(13));
    // Every position in one revolution yields a distinct glyph.
    let frames: std::collections::HashSet<_> = (0..10).map(spinner_glyph).collect();
    assert_eq!(frames.len(), 10);
}

#[test]
fn about_metadata_is_available_for_help() {
    // The footer renders the repo URL; guard against dropping it from Cargo.toml.
    assert!(env!("CARGO_PKG_REPOSITORY").contains("github.com/akunzai/gistui"));
}

#[test]
fn hint_line_colours_keys_by_action_category() {
    // Detail is the screen that binds `X` to a destructive delete.
    let bindings = keymap::for_screen(&Screen::List);
    let line = hint_line(
        "Tab panes  ·  d download  ·  P pins  ·  Esc/q back",
        bindings,
        &Theme::DARK,
    );
    let key_fg = |k: &str| {
        line.spans
            .iter()
            .find(|s| s.content == k)
            .unwrap_or_else(|| panic!("key span {k}"))
            .style
            .fg
    };
    assert_eq!(key_fg("Tab"), Some(Color::Cyan)); // navigation
    assert_eq!(key_fg("d"), Some(Color::Green)); // write
    assert_eq!(key_fg("P"), Some(Color::Cyan)); // opens a view, not the `pin` write action
    assert_eq!(key_fg("Esc/q"), Some(Color::Cyan)); // navigation
                                                    // Labels keep default brightness (no fg override) regardless of the key's category.
    let label = line
        .spans
        .iter()
        .find(|s| s.content.contains("download"))
        .expect("label span");
    assert_eq!(label.style.fg, None);
}

#[test]
fn category_color_maps_every_category() {
    use crate::tui::keymap::Category;
    assert_eq!(category_color(Category::Nav, &Theme::DARK), Color::Cyan);
    assert_eq!(category_color(Category::Read, &Theme::DARK), Color::Cyan);
    assert_eq!(category_color(Category::Write, &Theme::DARK), Color::Green);
    assert_eq!(
        category_color(Category::Destructive, &Theme::DARK),
        Color::Red
    );
}

/// A key the screen's table does not claim — a status line, or `MINIMAL_HINT`'s `;` and
/// `Ctrl+p` — reads as navigation rather than panicking or borrowing a neighbour's colour.
#[test]
fn hint_line_leaves_an_unclaimed_key_on_the_navigation_accent() {
    let line = hint_line(
        crate::tui::MINIMAL_HINT,
        keymap::for_screen(&Screen::GistDetail(Box::default())),
        &Theme::DARK,
    );
    let key_fg = |k: &str| {
        line.spans
            .iter()
            .find(|s| s.content == k)
            .unwrap_or_else(|| panic!("key span {k}"))
            .style
            .fg
    };
    assert_eq!(key_fg(";"), Some(Color::Cyan));
    assert_eq!(key_fg("Ctrl+p"), Some(Color::Cyan));
}

#[test]
fn hint_line_preserves_every_character() {
    // Sizing relies on wrap_line_count over the raw text, so styling must not add/drop chars.
    let text = "↑↓ move  ·  Enter diff · q back";
    let joined: String = hint_line(text, keymap::for_screen(&Screen::List), &Theme::DARK)
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(joined, text);
}

#[test]
fn tab_switches_focus() {
    let mut state = initial_state();
    assert_eq!(state.focus, FocusPane::Local);
    state.handle_key(KeyCode::Tab);
    assert_eq!(state.focus, FocusPane::Gist);
}

#[test]
fn digit_keys_jump_to_a_pane() {
    let mut state = initial_state();
    state.handle_key(KeyCode::Char('2'));
    assert_eq!(state.focus, FocusPane::Gist);
    state.handle_key(KeyCode::Char('1'));
    assert_eq!(state.focus, FocusPane::Local);
}

#[test]
fn t_toggles_gist_view() {
    let mut state = initial_state();
    assert_eq!(state.gist_view, GistView::Description);
    state.handle_key(KeyCode::Char('t'));
    assert_eq!(state.gist_view, GistView::Id);
    state.handle_key(KeyCode::Char('t'));
    assert_eq!(state.gist_view, GistView::Description);
}

#[test]
fn gist_row_label_switches_with_view() {
    let g = RankedGistFile {
        file: GistFile {
            description: "My Ghostty config".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("abc", "config")
        },
        mark: crate::ranking::MatchMark::None,
    };
    assert_eq!(
        gist_row_label(&g, GistView::Description),
        "config — My Ghostty config"
    );
    assert_eq!(gist_row_label(&g, GistView::Id), "abc / config");
}

#[test]
fn v_cycles_gist_type_filter() {
    let mut state = initial_state();
    assert_eq!(state.gist_type_filter, GistTypeFilter::All);
    state.handle_key(KeyCode::Char('v'));
    assert_eq!(state.gist_type_filter, GistTypeFilter::Public);
    state.handle_key(KeyCode::Char('v'));
    assert_eq!(state.gist_type_filter, GistTypeFilter::Secret);
    state.handle_key(KeyCode::Char('v'));
    assert_eq!(state.gist_type_filter, GistTypeFilter::Starred);
    state.handle_key(KeyCode::Char('v'));
    assert_eq!(state.gist_type_filter, GistTypeFilter::Forked);
    state.handle_key(KeyCode::Char('v'));
    assert_eq!(state.gist_type_filter, GistTypeFilter::All);
}

#[test]
fn s_cycles_gist_sort_when_gist_pane_focused() {
    let mut state = initial_state();
    state.focus = FocusPane::Gist;
    assert_eq!(state.gist_sort, GistSort::Match);
    state.handle_key(KeyCode::Char('s'));
    assert_eq!(state.gist_sort, GistSort::Name);
    state.handle_key(KeyCode::Char('s'));
    assert_eq!(state.gist_sort, GistSort::Recent);
    state.handle_key(KeyCode::Char('s'));
    assert_eq!(state.gist_sort, GistSort::Match);
    // The local sort is untouched while the gist pane is focused.
    assert_eq!(state.local_sort, LocalSort::Match);
}

#[test]
fn s_cycles_local_sort_when_local_pane_focused() {
    let mut state = initial_state();
    assert_eq!(state.focus, FocusPane::Local);
    assert_eq!(state.local_sort, LocalSort::Match);
    state.handle_key(KeyCode::Char('s'));
    assert_eq!(state.local_sort, LocalSort::Name);
    state.handle_key(KeyCode::Char('s'));
    assert_eq!(state.local_sort, LocalSort::Recent);
    state.handle_key(KeyCode::Char('s'));
    assert_eq!(state.local_sort, LocalSort::Match);
    // The gist sort is untouched while the local pane is focused.
    assert_eq!(state.gist_sort, GistSort::Match);
}

#[test]
fn reverse_ranking_orders_locals_by_selected_gist() {
    let mut state = initial_state();
    state.anchor = FocusPane::Gist;
    state.gists = vec![GistFile {
        updated_at: "x".into(),
        created_at: "x".into(),
        ..GistFile::fixture("a", "settings.json")
    }];
    state.locals = vec![
        LocalCandidate {
            path: PathBuf::from("other.txt"),
            pinned: false,
            modified: None,
        },
        LocalCandidate {
            path: PathBuf::from("settings.json"),
            pinned: false,
            modified: None,
        },
    ];
    // The local pane reverse-ranks against the selected gist (gist_index 0).
    let visible = state.visible_locals();
    assert_eq!(visible[0].candidate.path, PathBuf::from("settings.json"));
    assert_ne!(visible[0].mark, crate::ranking::MatchMark::None);
}

#[test]
fn local_sort_name_orders_by_filename() {
    let mut state = initial_state(); // focus Local -> no reverse ranking
    state.local_sort = LocalSort::Name;
    state.locals = vec![
        LocalCandidate {
            path: PathBuf::from("zeta.txt"),
            pinned: false,
            modified: None,
        },
        LocalCandidate {
            path: PathBuf::from("alpha.txt"),
            pinned: false,
            modified: None,
        },
    ];
    assert_eq!(
        state.visible_locals()[0].candidate.path,
        PathBuf::from("alpha.txt")
    );
}

#[test]
fn local_sort_recent_orders_by_mtime_desc_none_last() {
    let mut state = initial_state();
    state.local_sort = LocalSort::Recent;
    state.locals = vec![
        LocalCandidate {
            path: PathBuf::from("old"),
            pinned: false,
            modified: Some(100),
        },
        LocalCandidate {
            path: PathBuf::from("none"),
            pinned: false,
            modified: None,
        },
        LocalCandidate {
            path: PathBuf::from("new"),
            pinned: false,
            modified: Some(500),
        },
    ];
    let paths: Vec<_> = state
        .visible_locals()
        .into_iter()
        .map(|r| r.candidate.path)
        .collect();
    assert_eq!(
        paths,
        vec![
            PathBuf::from("new"),
            PathBuf::from("old"),
            PathBuf::from("none")
        ]
    );
}

#[test]
fn ranking_helpers_terminate_in_either_anchor() {
    // Regression: eagerly evaluating the cross-pane selection caused the two
    // anchor-driven rankings to recurse into each other.
    let mut state = initial_state();
    state.gists = vec![GistFile {
        updated_at: "x".into(),
        created_at: "x".into(),
        ..GistFile::fixture("a", "f")
    }];
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("f"),
        pinned: false,
        modified: None,
    }];
    for anchor in [FocusPane::Local, FocusPane::Gist] {
        state.anchor = anchor;
        let _ = state.ranked_gists();
        let _ = state.visible_locals();
        let _ = state.selected_local();
        let _ = state.selected_gist();
    }
}

#[test]
fn sort_by_name_and_recent_reorders_gists() {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            public: true,
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            ..GistFile::fixture("z", "zeta.json")
        },
        GistFile {
            public: true,
            updated_at: "2026-09-09T00:00:00Z".into(),
            created_at: "2026-09-09T00:00:00Z".into(),
            ..GistFile::fixture("a", "alpha.json")
        },
    ];
    // No local selected -> Match keeps gh list order (zeta, alpha).
    assert_eq!(state.ranked_gists()[0].file.filename, "zeta.json");

    state.gist_sort = GistSort::Name;
    assert_eq!(state.ranked_gists()[0].file.filename, "alpha.json");

    state.gist_sort = GistSort::Recent;
    assert_eq!(state.ranked_gists()[0].file.filename, "alpha.json");
    assert_eq!(state.ranked_gists()[1].file.filename, "zeta.json");
}

#[test]
fn gist_type_filter_limits_ranked_gists() {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            description: "p".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("pub", "a.json")
        },
        GistFile {
            description: "s".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("sec", "b.json")
        },
    ];
    assert_eq!(state.ranked_gists().len(), 2);

    state.gist_type_filter = GistTypeFilter::Public;
    let only_public = state.ranked_gists();
    assert_eq!(only_public.len(), 1);
    assert_eq!(only_public[0].file.gist_id, "pub");

    state.gist_type_filter = GistTypeFilter::Secret;
    let only_secret = state.ranked_gists();
    assert_eq!(only_secret.len(), 1);
    assert_eq!(only_secret[0].file.gist_id, "sec");
}

pub(super) fn state_with_two_gists() -> AppState {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            description: "My Ghostty config".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "config.ghostty")
        },
        GistFile {
            description: "SSH config".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("b", "ssh_config")
        },
    ];
    state.focus = FocusPane::Gist;
    state
}

#[test]
fn slash_enters_filter_mode_and_typing_filters() {
    let mut state = state_with_two_gists();
    assert!(!state.filtering);
    state.handle_key(KeyCode::Char('/'));
    assert!(state.filtering);
    // Type "ghostty" -> matches only the first gist (by filename + description).
    for c in "ghostty".chars() {
        state.handle_key(KeyCode::Char(c));
    }
    let ranked = state.ranked_gists();
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].file.gist_id, "a");
}

#[test]
fn filter_matches_description_case_insensitively() {
    let mut state = state_with_two_gists();
    state.filter_query = "SSH".into();
    let ranked = state.ranked_gists();
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].file.gist_id, "b");
}

#[test]
fn filter_enter_keeps_query_esc_clears() {
    let mut state = state_with_two_gists();
    state.handle_key(KeyCode::Char('/'));
    state.handle_key(KeyCode::Char('s'));
    state.handle_key(KeyCode::Char('s'));
    state.handle_key(KeyCode::Char('h'));
    state.handle_key(KeyCode::Enter);
    assert!(!state.filtering);
    assert_eq!(state.filter_query, "ssh");
    // Re-enter and Esc clears.
    state.handle_key(KeyCode::Char('/'));
    state.handle_key(KeyCode::Esc);
    assert!(!state.filtering);
    assert!(state.filter_query.is_empty());
}

#[test]
fn filter_backspace_deletes_last_char() {
    let mut state = state_with_two_gists();
    state.handle_key(KeyCode::Char('/'));
    state.handle_key(KeyCode::Char('x'));
    state.handle_key(KeyCode::Char('y'));
    state.handle_key(KeyCode::Backspace);
    assert_eq!(state.filter_query, "x");
}

#[test]
fn confirm_screen_scrolls_diff() {
    let mut state = initial_state();
    set_pending(&mut state, PendingAction::Download);
    set_diff_body(&mut state, "l1\nl2\nl3");
    assert_eq!(state.handle_key(KeyCode::Down), KeyOutcome::None);
    assert_eq!(state.diff_scroll(), 1);
    state.handle_key(KeyCode::Up);
    assert_eq!(state.diff_scroll(), 0);
}

#[test]
fn space_on_selected_gist_returns_preview_content() {
    let mut state = state_with_two_gists();
    assert!(matches!(
        state.handle_key(KeyCode::Char(' ')),
        KeyOutcome::PreviewContent { .. }
    ));
}

/// Issue #347: the Preview screen titles itself with the gist's description, consistent
/// with the Gist detail screen, rather than the raw id.
#[test]
fn preview_title_uses_gist_description() {
    let state = state_with_gists();
    assert_eq!(state.preview_title("g1", "a.txt"), "Preview: demo / a.txt");
}

/// Issue #347: without a known description, the preview title still identifies the gist
/// (falling back to its id) instead of silently showing just the filename.
#[test]
fn preview_title_falls_back_to_id_without_description() {
    let state = initial_state();
    assert_eq!(
        state.preview_title("unknown-id", "a.txt"),
        "Preview: Gist unknown-id / a.txt"
    );
}

#[test]
fn space_blocks_preview_for_image_gist_file() {
    let mut state = state_with_two_gists();
    state.gists[0].filename = "logo.png".into();
    state.gists[0].content_type = Some("image/png".into());
    assert_eq!(state.handle_key(KeyCode::Char(' ')), KeyOutcome::None);
    assert!(state
        .status
        .as_deref()
        .is_some_and(|s| s.contains("image file")));
}

#[test]
fn enter_blocks_diff_for_image_gist_file() {
    let mut state = state_with_two_gists();
    state.gists[0].filename = "photo.jpg".into();
    state.gists[0].content_type = Some("image/jpeg".into());
    assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
    assert!(state
        .status
        .as_deref()
        .is_some_and(|s| s.contains("image file")));
}

#[test]
fn space_without_gist_is_noop() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Char(' ')), KeyOutcome::None);
}

#[test]
fn question_opens_contextual_help_from_list() {
    let mut state = initial_state();
    state.handle_key(KeyCode::Char('?'));
    assert!(state.screen.is_help());
    assert_eq!(help_ref(&state).topic, HelpTopic::List);
    assert_eq!(state.nav_stack.last(), Some(&Screen::List));
    assert!(!help_ref(&state).index_open);
    // Arrow keys scroll help
    state.handle_key(KeyCode::Down);
    assert!(state.screen.is_help());
    assert_eq!(help_ref(&state).scroll, 1);
    state.handle_key(KeyCode::Up);
    assert!(state.screen.is_help());
    assert_eq!(help_ref(&state).scroll, 0);
    // Esc closes help (payload is dropped with the screen — no stale help on AppState).
    help_mut(&mut state).scroll = 5;
    state.handle_key(KeyCode::Esc);
    assert_eq!(state.screen, Screen::List);
    assert!(state.help().is_none());
}

#[test]
fn q_in_help_closes_to_list() {
    let mut state = initial_state();
    state.screen = Screen::Help(Box::default());
    assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn diff_view_applies_vertical_and_horizontal_scroll() {
    let text = "--- a\n+++ b\nabcdef\n more";
    let v = diff_view_highlighted(text, 2, 2, None, false, &Theme::DARK); // skip 2 lines, drop 2 leading chars
    assert_eq!(v.lines.len(), 2);
    assert_eq!(v.lines[0].spans[0].content, "cdef");
}

#[test]
fn diff_view_inline_highlights_changed_words() {
    // A single-line modification: "hello world" → "hello planet"
    let text = "--- a\n+++ b\n-hello world\n+hello planet\n";
    let v = diff_view_highlighted(text, 2, 0, None, false, &Theme::DARK); // skip header lines
                                                                          // del line: span 0 is "-", unchanged word "hello " is plain red,
                                                                          //           changed word "world" is bold red
    assert_eq!(v.lines.len(), 2);
    let del = &v.lines[0];
    let sign = del.spans.iter().find(|s| s.content == "-").unwrap();
    assert_eq!(sign.style.fg, Some(Color::Red));
    // "world" is the changed word — should be bold
    let world = del
        .spans
        .iter()
        .find(|s| s.content.trim() == "world")
        .unwrap();
    assert!(world.style.add_modifier.contains(Modifier::BOLD));
    // "hello " is unchanged — should NOT be bold
    let hello = del
        .spans
        .iter()
        .find(|s| s.content.starts_with("hello"))
        .unwrap();
    assert!(!hello.style.add_modifier.contains(Modifier::BOLD));
    // ins line: "planet" should be bold green
    let ins = &v.lines[1];
    let planet = ins
        .spans
        .iter()
        .find(|s| s.content.trim() == "planet")
        .unwrap();
    assert!(planet.style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn diff_view_highlights_context_lines_for_known_language() {
    // Context line " let x = 1;" gets syntax colour; the -/+ pair keeps red/green.
    let text = "--- a\n+++ b\n let x = 1;\n-old\n+new\n";
    let v = diff_view_highlighted(text, 0, 0, Some("rs"), true, &Theme::DARK);
    let ctx = v
        .lines
        .iter()
        .find(|l| l.spans.first().map(|s| s.content.as_ref()) == Some(" "))
        .expect("a context line marked by a leading space span");
    // `let` is a Rust keyword → magenta somewhere on the context line.
    assert!(ctx.spans.iter().any(|s| s.style.fg == Some(Color::Magenta)));
    // The del line stays red, never picks up a syntax colour.
    let del = v
        .lines
        .iter()
        .find(|l| l.spans.iter().any(|s| s.content == "-"))
        .unwrap();
    assert!(del.spans.iter().all(|s| s.style.fg != Some(Color::Magenta)));
}

#[test]
fn diff_view_leaves_context_plain_when_highlight_disabled() {
    let text = "--- a\n+++ b\n let x = 1;\n";
    let v = diff_view_highlighted(text, 0, 0, Some("rs"), false, &Theme::DARK);
    assert!(v.lines[2].spans.iter().all(|s| s.style.fg.is_none()));
}

#[test]
fn diff_view_skips_tabbed_context_lines() {
    // A tab in the context line keeps it plain so indentation stays aligned with -/+ lines.
    let text = "--- a\n+++ b\n \tlet x = 1;\n";
    let v = diff_view_highlighted(text, 0, 0, Some("rs"), true, &Theme::DARK);
    assert!(v.lines[2].spans.iter().all(|s| s.style.fg.is_none()));
}

#[test]
fn header_line_tints_local_yellow_and_gist_blue() {
    let local = header_line(
        "--- local: notes.txt (2026-06-10 14:25 UTC)",
        0,
        &Theme::DARK,
    );
    let kw = local.spans.iter().find(|s| s.content == "local").unwrap();
    assert_eq!(kw.style.fg, Some(Color::Yellow));

    let gist = header_line(
        "+++ gist abc123 / notes.txt (2026-06-10 13:10 UTC)",
        0,
        &Theme::DARK,
    );
    let kw = gist.spans.iter().find(|s| s.content == "gist").unwrap();
    assert_eq!(kw.style.fg, Some(Color::Blue));
}

#[test]
fn light_theme_keeps_focus_and_diff_header_colours_distinct() {
    let theme = Theme::LIGHT;

    assert_ne!(theme.accent, theme.dim);
    assert_ne!(theme.accent, theme.gist_label_color);
    assert_ne!(theme.dim, theme.gist_label_color);
}

#[test]
fn preview_diff_text_flips_with_focus() {
    // Download orientation (gist pane focused): old = local, new = gist.
    let dl = preview_diff_text(false, "local: a", "old\n", "gist b", "new\n", false);
    assert!(dl.starts_with("--- local: a\n+++ gist b\n"));

    // Upload orientation (local pane focused): old = gist, new = local.
    let ul = preview_diff_text(true, "local: a", "old\n", "gist b", "new\n", false);
    assert!(ul.starts_with("--- gist b\n+++ local: a\n"));
}

#[test]
fn format_unix_utc_known_instants() {
    assert_eq!(format_unix_utc(0), "1970-01-01 00:00 UTC");
    assert_eq!(format_unix_utc(1_780_656_360), "2026-06-05 10:46 UTC");
}

#[test]
fn gist_time_label_normalises_rfc3339() {
    assert_eq!(
        gist_time_label("2026-06-08T11:06:18Z"),
        "2026-06-08 11:06 UTC"
    );
    assert_eq!(gist_time_label(""), "unknown");
    assert_eq!(gist_time_label("short"), "short");
}

#[test]
fn wrap_line_count_is_responsive_to_width() {
    let text = "aaa bbb ccc";
    assert_eq!(wrap_line_count(text, 100), 1);
    assert_eq!(wrap_line_count(text, 7), 2);
    assert_eq!(wrap_line_count(text, 3), 3);
    assert_eq!(wrap_line_count(text, 0), 1);
}

#[test]
fn footer_height_collapses_to_zero_when_empty_else_wraps() {
    assert_eq!(footer_height("", 100, "", false), 0);
    assert_eq!(footer_height("? Help", 100, "", false), 1);
    assert_eq!(footer_height("aaa bbb ccc", 9, "", false), 2); // 2 wrapped lines at inner width 7
    assert_eq!(footer_height("/x_", 100, "Filter", false), 2); // title row + 1 content line
    assert_eq!(footer_height("aaa bbb ccc", 9, "", true), 1); // hints stay one row (#342)
}

#[test]
fn minimal_hint_shows_menu_and_palette_shortcuts() {
    assert_eq!(MINIMAL_HINT, "; Menu · Ctrl+p Palette");
    let (hint, colored) = footer_with_status(None, MINIMAL_HINT);
    assert_eq!(hint, "; Menu · Ctrl+p Palette");
    assert!(colored);
    let (status, colored) = footer_with_status(Some("Downloaded file.txt"), MINIMAL_HINT);
    assert_eq!(status, "Downloaded file.txt");
    assert!(!colored);
}

#[test]
fn confirm_prompt_covers_each_pending_action() {
    let mut state = initial_state();

    state.enter_diff(
        String::new(),
        String::new(),
        PathBuf::new(),
        PathBuf::from("notes.txt"),
    );
    state.enter_confirm_from_diff(PendingAction::Download);
    assert_eq!(confirm_prompt(&state), "Overwrite notes.txt? (y/n)");
    assert_eq!(confirm_modal_style(&state), ("Overwrite", Color::Red));

    set_pending(
        &mut state,
        PendingAction::Delete {
            gist_id: "abc".into(),
            label: "my config".into(),
        },
    );
    assert_eq!(
        confirm_prompt(&state),
        "Permanently delete \"my config\" (abc)? (y/n)"
    );
    assert_eq!(confirm_modal_style(&state), ("Delete", Color::Red));

    set_pending(
        &mut state,
        PendingAction::Upload {
            gist_id: "g1".into(),
            filename: "main.rs".into(),
            local_path: PathBuf::from("main.rs"),
        },
    );
    assert!(confirm_prompt(&state).starts_with("Upload main.rs to gist g1?"));
    assert_eq!(confirm_modal_style(&state), ("Upload", Color::Yellow));

    set_pending(
        &mut state,
        PendingAction::CompactGist {
            gist_id: "abc".into(),
            label: "my config".into(),
            count: 4,
        },
    );
    assert_eq!(
            confirm_prompt(&state),
            "Compact 4 revisions of \"my config\" into one? This force-pushes and cannot be undone. (y/n)"
        );
    assert_eq!(
        confirm_modal_style(&state),
        ("Compact revisions", Color::Red)
    );
}

#[test]
fn confirm_prompt_shows_description_editor_for_create() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Create {
            local_path: PathBuf::from("notes.txt"),
        },
    );
    state.editing_description = true;
    state.description_input = "hello".into();
    assert_eq!(
        confirm_prompt(&state),
        "Description (optional): hello_   ·  Enter next  ·  Esc cancel"
    );
    assert_eq!(confirm_modal_style(&state), ("Description", Color::Cyan));
}

#[test]
fn confirm_prompt_shows_watching_indicator_for_upload() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Upload {
            gist_id: "a".into(),
            filename: "notes.txt".into(),
            local_path: PathBuf::from("/tmp/notes.txt"),
        },
    );
    state.upload.watching = true;

    let prompt = confirm_prompt(&state);
    assert!(prompt.contains("watching for edits"));
    assert!(
        !prompt.contains("y yes"),
        "y/e hints should be hidden while watching"
    );
}

#[test]
fn marked_row_text_uses_match_mark_pin_prefix() {
    use crate::ranking::MatchMark;
    assert_eq!(marked_row_text("x".into(), MatchMark::Pinned), "📌 x");
    assert_eq!(marked_row_text("x".into(), MatchMark::ExactFilename), "x");
    assert_eq!(marked_row_text("x".into(), MatchMark::None), "x");
}

#[test]
fn gist_row_label_falls_back_to_filename_when_description_empty() {
    let g = RankedGistFile {
        file: GistFile {
            description: "  ".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("abc", "config")
        },
        mark: crate::ranking::MatchMark::None,
    };
    assert_eq!(gist_row_label(&g, GistView::Description), "config");
}

#[test]
fn left_right_scrolls_focused_gist_pane() {
    let mut state = initial_state();
    state.gists = vec![GistFile {
        description: "a fairly long description for scrolling".into(),
        updated_at: "x".into(),
        created_at: "x".into(),
        ..GistFile::fixture("a", "f.json")
    }];
    state.focus = FocusPane::Gist;
    assert_eq!(state.gist_hscroll, 0);
    state.handle_key(KeyCode::Left); // saturates at 0
    assert_eq!(state.gist_hscroll, 0);
    state.handle_key(KeyCode::Right);
    state.handle_key(KeyCode::Right);
    assert_eq!(state.gist_hscroll, 2);
    state.handle_key(KeyCode::Left);
    assert_eq!(state.gist_hscroll, 1);
}

#[test]
fn gist_hscroll_caps_at_painted_row() {
    let mut state = initial_state();
    state.gists = vec![GistFile {
        description: "tiny".into(),
        updated_at: "x".into(),
        created_at: "x".into(),
        ..GistFile::fixture("a", "f")
    }];
    state.focus = FocusPane::Gist;
    // Cap must use the painted display string (star / pin prefixes included), not the
    // star-less label helper — see issue #247.
    let ranked = state.ranked_gists();
    let row = marked_row_text(
        gist_row_display(&ranked[0], state.gist_view, &state),
        ranked[0].mark,
    );
    let max = super::text::hscroll_max_for_text(&row);
    for _ in 0..200 {
        state.handle_key(KeyCode::Right);
    }
    assert_eq!(state.gist_hscroll, max);
}

#[test]
fn gist_hscroll_follows_the_selected_row() {
    let mut state = initial_state();
    state.gist_sort = GistSort::Name;
    state.gists = vec![
        GistFile {
            description: "ab".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("short", "a.txt")
        },
        GistFile {
            description: "a fairly long description for scrolling".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("long", "b.txt")
        },
    ];
    state.focus = FocusPane::Gist;
    let ranked = state.ranked_gists();
    let short_max = super::text::hscroll_max_for_text(&marked_row_text(
        gist_row_display(&ranked[0], state.gist_view, &state),
        ranked[0].mark,
    ));
    let long_max = super::text::hscroll_max_for_text(&marked_row_text(
        gist_row_display(&ranked[1], state.gist_view, &state),
        ranked[1].mark,
    ));
    assert!(
        short_max < long_max,
        "fixture must make a.txt shorter than b.txt"
    );
    for _ in 0..200 {
        state.handle_key(KeyCode::Right);
    }
    assert_eq!(
        state.gist_hscroll, short_max,
        "Right must stop at the selected row, not the longest row in the pane"
    );
    state.handle_key(KeyCode::Down);
    for _ in 0..200 {
        state.handle_key(KeyCode::Right);
    }
    assert_eq!(state.gist_hscroll, long_max);
    state.handle_key(KeyCode::Up);
    assert_eq!(state.gist_index, 0);
    assert!(
        state.gist_hscroll <= short_max,
        "selected row must not stay scrolled past its own content (hscroll {}, max {})",
        state.gist_hscroll,
        short_max
    );
}

#[test]
fn local_hscroll_caps_at_selected_row_not_the_longest() {
    let mut state = state_with_local_paths(&[
        "/cwd/ab.txt",
        "/cwd/a-fairly-long-filename-for-scrolling.md",
    ]);
    state.focus = FocusPane::Local;
    state.local_index = 0;
    let locals = state.visible_locals();
    let short_row = marked_row_text(
        super::text::local_row_label(&locals[0].candidate.path, &state.cwd),
        locals[0].mark,
    );
    let long_row = marked_row_text(
        super::text::local_row_label(&locals[1].candidate.path, &state.cwd),
        locals[1].mark,
    );
    let short_max = super::text::hscroll_max_for_text(&short_row);
    let long_max = super::text::hscroll_max_for_text(&long_row);
    assert!(
        short_max < long_max,
        "fixture must make the selected row shorter than its sibling"
    );
    for _ in 0..200 {
        state.handle_key(KeyCode::Right);
    }
    assert_eq!(
        state.local_hscroll, short_max,
        "Right must stop at the selected local row, not the longest row in the pane"
    );
}

#[test]
fn gist_hscroll_caps_include_star_prefix() {
    let mut state = initial_state();
    state.gists = vec![GistFile {
        description: "tiny".into(),
        updated_at: "x".into(),
        created_at: "x".into(),
        ..GistFile::fixture("starred-id", "f")
    }];
    state.starred_gist_ids.insert("starred-id".into());
    state.focus = FocusPane::Gist;

    let ranked = state.ranked_gists();
    let display = gist_row_display(&ranked[0], state.gist_view, &state);
    assert!(
        display.starts_with("★ "),
        "display must include star prefix, got {display:?}"
    );
    let label = gist_row_label(&ranked[0], state.gist_view);
    assert!(
        !label.starts_with('★'),
        "label helper stays star-less for pure text tests"
    );
    // Regression: measuring the label (no star) under-scrolled by 2 chars.
    assert_eq!(
        super::text::text_len(&display),
        super::text::text_len(&label) + 2
    );

    let row = marked_row_text(display, ranked[0].mark);
    let max = super::text::hscroll_max_for_text(&row);
    let label_only_max = super::text::hscroll_max_for_text(&label);
    assert!(max > label_only_max, "star must raise the hscroll cap");

    for _ in 0..200 {
        state.handle_key(KeyCode::Right);
    }
    assert_eq!(
        state.gist_hscroll, max,
        "Right must reach the display-string max, not the star-less label max"
    );
}

#[test]
fn moving_gist_selection_resets_hscroll() {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            description: "first long description here".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "a.json")
        },
        GistFile {
            description: "second long description here".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("b", "b.json")
        },
    ];
    state.focus = FocusPane::Gist;
    state.handle_key(KeyCode::Right);
    assert_eq!(state.gist_hscroll, 1);
    state.handle_key(KeyCode::Down);
    assert_eq!(state.gist_hscroll, 0);
}

#[test]
fn empty_state_has_no_ranked_gists() {
    let state = initial_state();
    assert!(state.ranked_gists().is_empty());
}

#[test]
fn no_local_selected_lists_all_gists_unranked() {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            description: "first".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "alpha.json")
        },
        GistFile {
            description: "second".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("b", "beta.json")
        },
    ];
    let ranked = state.ranked_gists();
    assert_eq!(ranked.len(), 2);
    // Order preserved (unranked) and no scoring applied.
    assert_eq!(ranked[0].file.filename, "alpha.json");
    assert_eq!(ranked[0].mark, crate::ranking::MatchMark::None);
}

#[test]
fn enter_with_no_local_but_gist_selected_returns_preview() {
    let mut state = initial_state();
    state.gists = vec![GistFile {
        description: "first".into(),
        updated_at: "x".into(),
        created_at: "x".into(),
        ..GistFile::fixture("a", "alpha.json")
    }];
    state.focus = FocusPane::Gist;
    assert!(state.locals.is_empty());
    assert!(matches!(
        state.handle_key(KeyCode::Enter),
        KeyOutcome::PreviewDiff { .. }
    ));
}

#[test]
fn local_selection_changes_ranked_gists() {
    let mut state = initial_state();
    state.locals = vec![
        LocalCandidate {
            path: PathBuf::from("/tmp/settings.json"),
            pinned: false,
            modified: None,
        },
        LocalCandidate {
            path: PathBuf::from("/tmp/statusline.sh"),
            pinned: false,
            modified: None,
        },
    ];
    state.gists = vec![
        GistFile {
            description: "settings".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "settings.json")
        },
        GistFile {
            description: "status".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("b", "statusline.sh")
        },
    ];

    assert_eq!(state.ranked_gists()[0].file.filename, "settings.json");
    state.handle_key(KeyCode::Down);
    assert_eq!(state.ranked_gists()[0].file.filename, "statusline.sh");
}

/// Public `ranked_gists` / `visible_locals` / `selected_*` stay pure recomputes (no
/// content-hash / epoch memo — #154 closed that approach). Hot paths use
/// `list_pane_snapshots()` (#224 shape #1) which builds each list once without caching
/// across mutations. `selected_gist` / `selected_local` must still equal `list[index]`
/// after an earlier read and an input mutation — a future silent cache would break here.
#[test]
fn selected_accessors_track_recomputed_lists_with_no_cache() {
    let mut state = initial_state();
    state.locals = vec![
        LocalCandidate {
            path: PathBuf::from("/tmp/settings.json"),
            pinned: false,
            modified: None,
        },
        LocalCandidate {
            path: PathBuf::from("/tmp/statusline.sh"),
            pinned: false,
            modified: None,
        },
    ];
    state.gists = vec![
        GistFile {
            description: "settings".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "settings.json")
        },
        GistFile {
            description: "status".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("b", "statusline.sh")
        },
    ];

    // Read both lists first — this would warm any hypothetical cache.
    let _ = state.ranked_gists();
    let _ = state.visible_locals();
    // Accessors equal a fresh recompute at the current indices.
    assert_eq!(
        state.selected_gist().map(|g| g.file.filename),
        state
            .ranked_gists()
            .into_iter()
            .nth(state.gist_index)
            .map(|g| g.file.filename),
    );
    assert_eq!(
        state.selected_local().map(|l| l.path),
        state
            .visible_locals()
            .into_iter()
            .nth(state.local_index)
            .map(|r| r.candidate.path),
    );
    assert_eq!(state.ranked_gists()[0].file.filename, "settings.json");

    // Move the local selection: ranking must reflect the *new* state, not the earlier read.
    state.handle_key(KeyCode::Down);
    assert_eq!(state.ranked_gists()[0].file.filename, "statusline.sh");
    // The accessors still match a fresh recompute after the mutation.
    assert_eq!(
        state.selected_gist().map(|g| g.file.filename),
        state
            .ranked_gists()
            .into_iter()
            .nth(state.gist_index)
            .map(|g| g.file.filename),
    );
    assert_eq!(
        state.selected_local().map(|l| l.path),
        state
            .visible_locals()
            .into_iter()
            .nth(state.local_index)
            .map(|r| r.candidate.path),
    );
}

#[test]
fn list_pane_snapshots_match_public_accessors() {
    let mut state = initial_state();
    state.locals = vec![
        LocalCandidate {
            path: PathBuf::from("/tmp/settings.json"),
            pinned: false,
            modified: None,
        },
        LocalCandidate {
            path: PathBuf::from("/tmp/statusline.sh"),
            pinned: false,
            modified: None,
        },
    ];
    state.gists = vec![
        GistFile {
            description: "settings".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "settings.json")
        },
        GistFile {
            description: "status".into(),
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("b", "statusline.sh")
        },
    ];

    for anchor in [FocusPane::Local, FocusPane::Gist] {
        state.anchor = anchor;
        let (locals, gists) = state.list_pane_snapshots();
        assert_eq!(
            locals
                .iter()
                .map(|r| r.candidate.path.clone())
                .collect::<Vec<_>>(),
            state
                .visible_locals()
                .into_iter()
                .map(|r| r.candidate.path)
                .collect::<Vec<_>>(),
            "locals mismatch for {anchor:?}"
        );
        assert_eq!(
            gists
                .iter()
                .map(|g| g.file.filename.clone())
                .collect::<Vec<_>>(),
            state
                .ranked_gists()
                .into_iter()
                .map(|g| g.file.filename)
                .collect::<Vec<_>>(),
            "gists mismatch for {anchor:?}"
        );
    }
}

#[test]
fn changing_local_selection_resets_gist_index() {
    let mut state = initial_state();
    state.locals = vec![
        LocalCandidate {
            path: PathBuf::from("/tmp/a.json"),
            pinned: false,
            modified: None,
        },
        LocalCandidate {
            path: PathBuf::from("/tmp/b.json"),
            pinned: false,
            modified: None,
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
        modified: None,
    }];
    state.gists = vec![GistFile {
        description: "settings".into(),
        updated_at: "x".into(),
        created_at: "x".into(),
        ..GistFile::fixture("a", "settings.json")
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
        PathBuf::from("/tmp/cwd/x"),
    );
    assert!(state.screen.is_diff());
    assert!(state.diff_previewed());
    assert_eq!(state.preview_remote(), "remote body");
    assert_eq!(state.preview_local(), PathBuf::from("/tmp/x"));
    assert_eq!(state.download_target(), PathBuf::from("/tmp/cwd/x"));
    assert_eq!(state.diff_scroll(), 0);
}

#[test]
fn enter_in_gist_focus_with_selection_returns_preview() {
    let mut state = state_with_selection();
    assert!(matches!(
        state.handle_key(KeyCode::Enter),
        KeyOutcome::PreviewDiff { .. }
    ));
}

#[test]
fn enter_with_nested_local_targets_its_directory() {
    let mut state = state_with_selection();
    state.cwd = PathBuf::from("/tmp");
    state.locals[0].path = PathBuf::from("/tmp/nested/settings.json");

    let KeyOutcome::PreviewDiff {
        local_path, target, ..
    } = state.handle_key(KeyCode::Enter)
    else {
        panic!("expected PreviewDiff");
    };

    assert_eq!(local_path, Some(PathBuf::from("/tmp/nested/settings.json")));
    assert_eq!(target, PathBuf::from("/tmp/nested/settings.json"));
}

#[test]
fn enter_in_local_focus_previews_top_gist() {
    let mut state = state_with_selection();
    state.focus = FocusPane::Local;
    assert!(matches!(
        state.handle_key(KeyCode::Enter),
        KeyOutcome::PreviewDiff { .. }
    ));
}

#[test]
fn enter_with_no_gists_is_noop_in_local_focus() {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/x"),
        pinned: false,
        modified: None,
    }];
    state.focus = FocusPane::Local;
    assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
}

#[test]
fn d_in_gist_focus_returns_download_gist() {
    let mut state = state_with_selection();
    assert!(matches!(
        state.handle_key(KeyCode::Char('d')),
        KeyOutcome::DownloadGist { .. }
    ));
}

#[test]
fn d_in_local_focus_is_noop() {
    let mut state = state_with_selection();
    state.focus = FocusPane::Local;
    assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
}

#[test]
fn d_without_gists_is_noop() {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/x"),
        pinned: false,
        modified: None,
    }];
    state.focus = FocusPane::Gist;
    assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
}

#[test]
fn enter_without_gists_is_noop() {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/x"),
        pinned: false,
        modified: None,
    }];
    state.focus = FocusPane::Gist;
    assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
}

#[test]
fn diff_scroll_respects_bounds() {
    let mut state = initial_state();
    state.enter_diff(
        "l1\nl2\nl3".into(),
        "r".into(),
        PathBuf::from("/tmp/x"),
        PathBuf::from("/tmp/x"),
    );
    assert_eq!(state.diff_scroll(), 0);
    state.handle_key(KeyCode::Up); // stays at 0
    assert_eq!(state.diff_scroll(), 0);
    state.handle_key(KeyCode::Down);
    assert_eq!(state.diff_scroll(), 1);
    state.handle_key(KeyCode::Down);
    assert_eq!(state.diff_scroll(), 2);
    state.handle_key(KeyCode::Down); // capped at lines-1 = 2
    assert_eq!(state.diff_scroll(), 2);
    state.handle_key(KeyCode::Up);
    assert_eq!(state.diff_scroll(), 1);
}

#[test]
fn diff_hscroll_respects_bounds() {
    let mut state = initial_state();
    // Longest line is "abcd" (4 chars) -> max offset 3.
    state.enter_diff(
        "abcd\nab".into(),
        "r".into(),
        PathBuf::from("/tmp/x"),
        PathBuf::from("/tmp/x"),
    );
    assert_eq!(state.diff_hscroll(), 0);
    state.handle_key(KeyCode::Left); // stays at 0
    assert_eq!(state.diff_hscroll(), 0);
    for _ in 0..10 {
        state.handle_key(KeyCode::Right);
    }
    assert_eq!(state.diff_hscroll(), 3);
    state.handle_key(KeyCode::Left);
    assert_eq!(state.diff_hscroll(), 2);
}

#[test]
fn d_in_diff_requests_download_when_file_absent() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist.json");
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/local"),
        missing.clone(),
    );
    assert!(matches!(
        state.handle_key(KeyCode::Char('d')),
        KeyOutcome::DownloadRequested { target } if target == missing
    ));
}

#[test]
fn d_in_diff_requests_download_when_file_exists() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("exists.json");
    std::fs::write(&existing, "old").unwrap();
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/local"),
        existing.clone(),
    );
    assert!(matches!(
        state.handle_key(KeyCode::Char('d')),
        KeyOutcome::DownloadRequested { target } if target == existing
    ));
    assert!(state.screen.is_diff());
}

#[test]
fn confirm_y_returns_download() {
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/x"),
        PathBuf::from("/tmp/x"),
    );
    set_pending(&mut state, PendingAction::Download);
    assert!(matches!(
        state.handle_key(KeyCode::Char('y')),
        KeyOutcome::Download {
            mode: crate::actions::DownloadMode::Overwrite(_)
        }
    ));
}

#[test]
fn confirm_n_returns_to_diff() {
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/x"),
        PathBuf::from("/tmp/x"),
    );
    set_pending(&mut state, PendingAction::Download);
    assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
    assert!(state.screen.is_diff());
}

#[test]
fn q_in_list_quits_on_second_press() {
    let mut state = initial_state();
    // First press only arms the quit (and surfaces a hint); it must not exit.
    assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
    assert!(state.quit_armed);
    // Issue #346: the hint must name every key that confirms the quit, not just `q`.
    assert_eq!(
        state.status.as_deref(),
        Some("Press q or Esc again to quit (any other key cancels)")
    );
    // Second press confirms.
    assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::Quit);
}

#[test]
fn esc_in_list_quits_on_second_press() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
    assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::Quit);
}

#[test]
fn any_key_cancels_a_pending_quit() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
    assert!(state.quit_armed);
    // A non-quit key disarms; the next q then needs two presses again.
    state.handle_key(KeyCode::Tab);
    assert!(!state.quit_armed);
    assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
}

#[test]
fn q_in_confirm_cancels_without_quitting() {
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/x"),
        PathBuf::from("/tmp/x"),
    );
    set_pending(&mut state, PendingAction::Download);
    assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
    assert!(state.screen.is_diff());
}

#[test]
fn confirm_esc_returns_to_diff() {
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/x"),
        PathBuf::from("/tmp/x"),
    );
    set_pending(&mut state, PendingAction::Download);
    assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
    assert!(state.screen.is_diff());
}

#[test]
fn d_in_diff_on_existing_requests_download() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("exists.json");
    std::fs::write(&existing, "old").unwrap();
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/local"),
        existing.clone(),
    );
    assert!(matches!(
        state.handle_key(KeyCode::Char('d')),
        KeyOutcome::DownloadRequested { target } if target == existing
    ));
    assert!(state.screen.is_diff());
}

#[test]
fn p_pins_unpinned_pair_then_unpins() {
    let mut state = state_with_selection();
    assert!(matches!(
        state.handle_key(KeyCode::Char('p')),
        KeyOutcome::Pin { .. }
    ));
    state.pinned = vec![PinnedMapping {
        local_path: PathBuf::from("/tmp/settings.json"),
        gist_id: "a".into(),
        gist_filename: "settings.json".into(),
        direction: None,
        last_seen_hash: None,
    }];
    assert!(matches!(
        state.handle_key(KeyCode::Char('p')),
        KeyOutcome::Unpin { .. }
    ));
}

#[test]
fn p_without_local_or_gist_is_noop() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::None);
}

#[test]
fn u_adds_when_gist_lacks_filename() {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/config"),
        pinned: false,
        modified: None,
    }];
    state.gists = vec![GistFile {
        description: "x".into(),
        updated_at: "x".into(),
        created_at: "x".into(),
        ..GistFile::fixture("a", "settings.json")
    }];
    state.focus = FocusPane::Gist;
    assert!(matches!(
        state.handle_key(KeyCode::Char('u')),
        KeyOutcome::UploadAdd { .. }
    ));
}

#[test]
fn u_previews_when_gist_has_same_filename() {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/settings.json"),
        pinned: false,
        modified: None,
    }];
    state.gists = vec![GistFile {
        description: "x".into(),
        updated_at: "x".into(),
        created_at: "x".into(),
        ..GistFile::fixture("a", "settings.json")
    }];
    state.focus = FocusPane::Gist;
    assert!(matches!(
        state.handle_key(KeyCode::Char('u')),
        KeyOutcome::UploadPreview { .. }
    ));
}

#[test]
fn u_without_selection_is_noop() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::None);
}

#[test]
fn y_copies_gist_url_on_list_gists_and_detail() {
    let mut list = state_with_two_gists();
    assert!(matches!(
        list.handle_key(KeyCode::Char('y')),
        KeyOutcome::CopyGistUrl { .. }
    ));

    let mut gists = state_with_two_gists();
    gists.screen = Screen::Gists(Box::default());
    assert!(matches!(
        gists.handle_key(KeyCode::Char('y')),
        KeyOutcome::CopyGistUrl { .. }
    ));

    let mut detail = state_with_gists();
    detail.screen = Screen::GistDetail(Box::default());
    detail_mut(&mut detail).gist_id = Some("g1".into());
    assert!(matches!(
        detail.handle_key(KeyCode::Char('y')),
        KeyOutcome::CopyGistUrl { .. }
    ));
}

#[test]
fn c_in_detail_requests_compaction_not_gist_manager() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists(Box::default());
    assert_eq!(state.handle_key(KeyCode::Char('c')), KeyOutcome::None);
    state.screen = Screen::GistDetail(Box::default());
    detail_mut(&mut state).gist_id = Some("a".into());
    assert!(matches!(
        state.handle_key(KeyCode::Char('c')),
        KeyOutcome::CompactGist { .. }
    ));
    // `c` on the main list is not a compaction trigger.
    let mut list = state_with_two_gists();
    assert_eq!(list.handle_key(KeyCode::Char('c')), KeyOutcome::None);
}

#[test]
fn e_edits_local_with_file_selected() {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/config"),
        pinned: false,
        modified: None,
    }];
    assert!(matches!(
        state.handle_key(KeyCode::Char('e')),
        KeyOutcome::EditLocal { .. }
    ));
}

#[test]
fn e_without_local_is_noop() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::None);
}

#[test]
fn confirm_upload_y_returns_upload() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Upload {
            gist_id: "a".into(),
            filename: "settings.json".into(),
            local_path: PathBuf::from("/tmp/settings.json"),
        },
    );
    assert_eq!(state.handle_key(KeyCode::Char('y')), KeyOutcome::Upload);
}

#[test]
fn confirm_upload_e_returns_edit_upload() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Upload {
            gist_id: "a".into(),
            filename: "settings.json".into(),
            local_path: PathBuf::from("/tmp/settings.json"),
        },
    );
    assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::EditUpload);
}

#[test]
fn confirm_upload_y_is_blocked_while_watching() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Upload {
            gist_id: "a".into(),
            filename: "settings.json".into(),
            local_path: PathBuf::from("/tmp/settings.json"),
        },
    );
    state.upload.watching = true;

    assert_eq!(state.handle_key(KeyCode::Char('y')), KeyOutcome::None);
    assert_eq!(
        state.status.as_deref(),
        Some("editor still open — finish editing first")
    );
}

#[test]
fn confirm_upload_e_is_blocked_while_watching() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Upload {
            gist_id: "a".into(),
            filename: "settings.json".into(),
            local_path: PathBuf::from("/tmp/settings.json"),
        },
    );
    state.upload.watching = true;

    assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::None);
    assert_eq!(state.status.as_deref(), Some("editor already open"));
}

#[test]
fn confirm_upload_json_toggles() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Upload {
            gist_id: "a".into(),
            filename: "settings.json".into(),
            local_path: PathBuf::from("/tmp/settings.json"),
        },
    );
    assert!(!state.upload.json_pretty);
    assert!(!state.upload.json_sort);

    // Toggle pretty
    assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::None);
    assert!(state.upload.json_pretty);

    // Toggle sort
    assert_eq!(state.handle_key(KeyCode::Char('s')), KeyOutcome::None);
    assert!(state.upload.json_sort);

    // Toggle pretty off
    assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::None);
    assert!(!state.upload.json_pretty);
}

// The upload buffer (and the local-file edit) shell out to `$EDITOR` and read the file back
// once the editor exits. GUI editors that fork and return immediately (zed, code, …) must be
// given a wait flag, or the read happens before the user saves and the *pre-edit* content is
// uploaded — silently defeating a redact. `editor_command` injects that flag.

#[test]
fn editor_command_injects_wait_for_gui_editors() {
    for ed in ["zed", "code", "code-insiders", "cursor", "windsurf", "subl"] {
        let (program, args) = super::bg::editor_command(ed).unwrap();
        assert_eq!(program, ed);
        assert!(
            args.iter().any(|a| a == "--wait" || a == "-w"),
            "expected a wait flag for GUI editor {ed:?}, got {args:?}"
        );
    }
}

#[test]
fn editor_command_matches_gui_editor_by_basename() {
    // A full path or a `.exe` suffix must still be recognised as a GUI editor.
    let (program, args) = super::bg::editor_command("/usr/local/bin/zed -n").unwrap();
    assert_eq!(program, "/usr/local/bin/zed");
    assert_eq!(args, vec!["-n", "--wait"]);
}

#[test]
fn editor_command_leaves_terminal_editors_untouched() {
    for ed in ["vi", "vim", "nvim", "nano", "emacs", "hx"] {
        let (program, args) = super::bg::editor_command(ed).unwrap();
        assert_eq!(program, ed);
        assert!(
            args.is_empty(),
            "terminal editor {ed:?} should get no injected flag, got {args:?}"
        );
    }
}

#[test]
fn editor_command_keeps_an_existing_wait_flag() {
    // Don't duplicate a wait flag the user already configured (either spelling).
    let (_, args) = super::bg::editor_command("code --wait").unwrap();
    assert_eq!(args, vec!["--wait"]);
    let (_, args) = super::bg::editor_command("subl -w").unwrap();
    assert_eq!(args, vec!["-w"]);
}

#[test]
fn editor_command_blank_is_none() {
    assert!(super::bg::editor_command("").is_none());
    assert!(super::bg::editor_command("   ").is_none());
}

#[test]
fn editor_is_gui_matches_known_gui_editors() {
    for ed in [
        "zed",
        "code",
        "code-insiders",
        "codium",
        "vscodium",
        "cursor",
        "windsurf",
        "subl",
        "sublime_text",
    ] {
        assert!(
            super::bg::editor_is_gui(ed),
            "{ed} should be recognised as a GUI editor"
        );
    }
}

#[test]
fn editor_is_gui_rejects_terminal_editors() {
    for ed in ["vi", "vim", "nvim", "nano", "emacs", "hx"] {
        assert!(
            !super::bg::editor_is_gui(ed),
            "{ed} should not be recognised as a GUI editor"
        );
    }
}

#[test]
fn editor_is_gui_matches_by_basename_from_full_path() {
    assert!(super::bg::editor_is_gui("/usr/local/bin/zed"));
    assert!(super::bg::editor_is_gui("C:\\Tools\\code.exe"));
}

// Whichever editor is used, the confirmed upload must send the edited (redacted) buffer, not
// the original file snapshot taken at preview time.

#[test]
fn content_to_upload_prefers_edited_content() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Upload {
            gist_id: "a".into(),
            filename: "notes.txt".into(),
            local_path: PathBuf::from("/tmp/notes.txt"),
        },
    );
    state.upload.original_content = "token=abc123secret".into();
    state.upload.edited_content = Some("token=REDACTED".into());
    assert_eq!(state.content_to_upload(), "token=REDACTED");
}

#[test]
fn content_to_upload_prefers_edited_content_for_json() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Upload {
            gist_id: "a".into(),
            filename: "settings.json".into(),
            local_path: PathBuf::from("/tmp/settings.json"),
        },
    );
    state.upload.original_content = r#"{"token":"abc123secret"}"#.into();
    state.upload.edited_content = Some(r#"{"token":"REDACTED"}"#.into());
    assert_eq!(state.content_to_upload(), r#"{"token":"REDACTED"}"#);
}

#[test]
fn n_opens_create_confirm() {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/config.toml"),
        pinned: false,
        modified: None,
    }];
    assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
    assert!(state.screen.is_confirm());
    assert_eq!(
        state.pending_action().cloned(),
        Some(PendingAction::Create {
            local_path: PathBuf::from("/tmp/config.toml")
        })
    );
}

#[test]
fn x_removes_selected_file_from_a_multifile_gist() {
    let mut state = initial_state();
    state.focus = FocusPane::Gist;
    state.gists = vec![
        GistFile {
            description: "my notes".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            ..GistFile::fixture("abc123", "a.md")
        },
        GistFile {
            description: "my notes".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            ..GistFile::fixture("abc123", "b.md")
        },
    ];
    // X stages a single-file removal (not a whole-gist delete) and asks to confirm.
    assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
    assert!(state.screen.is_confirm());
    assert_eq!(
        state.pending_action().cloned(),
        Some(PendingAction::RemoveFile {
            gist_id: "abc123".into(),
            filename: "a.md".into(),
            label: "my notes".into(),
        })
    );
}

#[test]
fn remove_file_confirm_y_returns_execute_remove_file() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::RemoveFile {
            gist_id: "abc123".into(),
            filename: "a.md".into(),
            label: "my notes".into(),
        },
    );
    assert_eq!(
        state.handle_key(KeyCode::Char('y')),
        KeyOutcome::ExecuteRemoveFile
    );
}

#[test]
fn delete_confirm_y_returns_execute_delete() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Delete {
            gist_id: "abc123".into(),
            label: "my notes".into(),
        },
    );
    assert_eq!(
        state.handle_key(KeyCode::Char('y')),
        KeyOutcome::ExecuteDelete
    );
}

#[test]
fn delete_from_list_returns_to_list() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Delete {
            gist_id: "abc123".into(),
            label: "my notes".into(),
        },
    );
    assert_eq!(
        state.handle_key(KeyCode::Char('y')),
        KeyOutcome::ExecuteDelete
    );
    state.cancel_confirm_after_delete();
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn delete_from_gist_detail_opened_via_gists_returns_to_gists() {
    let mut state = initial_state();
    gists_mut(&mut state);
    state.enter(Screen::GistDetail(Box::default()));
    detail_mut(&mut state).gist_id = Some("abc123".into());
    set_pending(
        &mut state,
        PendingAction::Delete {
            gist_id: "abc123".into(),
            label: "my notes".into(),
        },
    );
    assert_eq!(
        state.handle_key(KeyCode::Char('y')),
        KeyOutcome::ExecuteDelete
    );
    state.cancel_confirm_after_delete();
    assert!(state.screen.is_gists());
}

#[test]
fn delete_from_gist_detail_opened_via_pins_returns_to_pins() {
    let mut state = initial_state();
    pins_mut(&mut state);
    state.enter(Screen::GistDetail(Box::default()));
    detail_mut(&mut state).gist_id = Some("abc123".into());
    set_pending(
        &mut state,
        PendingAction::Delete {
            gist_id: "abc123".into(),
            label: "my notes".into(),
        },
    );
    assert_eq!(
        state.handle_key(KeyCode::Char('y')),
        KeyOutcome::ExecuteDelete
    );
    state.cancel_confirm_after_delete();
    assert!(state.screen.is_pins());
}

#[test]
fn input_line_reverses_the_char_under_the_cursor() {
    let mut input = TextInput::from("abc");
    input.left(); // ab|c → cursor on 'c'
    let line = input_line("/", &input, "");
    // Exactly one span carries the reverse-video cursor, and it's the char at the cursor.
    let reversed: Vec<&str> = line
        .spans
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(reversed, vec!["c"]);
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "/abc");
}

#[test]
fn input_line_cursor_at_end_reverses_trailing_space() {
    let input = TextInput::from("ab");
    let line = input_line("", &input, "");
    let reversed: Vec<&str> = line
        .spans
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(reversed, vec![" "]);
}

#[test]
fn create_description_edits_mid_string_with_cursor_keys() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Create {
            local_path: PathBuf::from("notes.txt"),
        },
    );
    state.editing_description = true;
    for c in "helo".chars() {
        state.handle_key(KeyCode::Char(c));
    }
    // Fix the typo: go back one char and insert the missing 'l'.
    state.handle_key(KeyCode::Left);
    state.handle_key(KeyCode::Char('l'));
    assert_eq!(state.description_input, "hello");
    // Enter advances to the visibility step without losing the text.
    state.handle_key(KeyCode::Enter);
    assert!(!state.editing_description);
    assert_eq!(state.description_input, "hello");
}

#[test]
fn gist_view_q_returns_to_list() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists(Box::default());
    state.handle_key(KeyCode::Char('q'));
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn create_confirm_s_and_p_choose_visibility() {
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Create {
            local_path: PathBuf::from("/tmp/config.toml"),
        },
    );
    assert_eq!(
        state.handle_key(KeyCode::Char('s')),
        KeyOutcome::Create(false)
    );

    set_pending(
        &mut state,
        PendingAction::Create {
            local_path: PathBuf::from("/tmp/config.toml"),
        },
    );
    assert_eq!(
        state.handle_key(KeyCode::Char('p')),
        KeyOutcome::Create(true)
    );
}

pub(super) fn state_ready_to_create() -> AppState {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/config.toml"),
        pinned: false,
        modified: None,
    }];
    state
}

#[test]
fn n_starts_create_in_the_description_editor() {
    let mut state = state_ready_to_create();
    state.handle_key(KeyCode::Char('n'));
    assert!(state.screen.is_confirm());
    assert!(state.editing_description);
    // While editing, letters (incl. s/p) are typed into the description, not
    // interpreted as the visibility choice.
    for c in "notes".chars() {
        assert_eq!(state.handle_key(KeyCode::Char(c)), KeyOutcome::None);
    }
    assert_eq!(state.description_input, "notes");
}

#[test]
fn create_enter_advances_to_visibility_then_s_creates() {
    let mut state = state_ready_to_create();
    state.handle_key(KeyCode::Char('n'));
    state.handle_key(KeyCode::Char('h'));
    state.handle_key(KeyCode::Char('i'));
    // Enter ends the description step (does not create yet).
    assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
    assert!(!state.editing_description);
    assert_eq!(state.description_input, "hi");
    // Now s/p choose visibility and trigger the create.
    assert_eq!(
        state.handle_key(KeyCode::Char('s')),
        KeyOutcome::Create(false)
    );
}

pub(super) fn pins_state_with_long_home_path() -> AppState {
    let mut state = initial_state();
    state.screen = Screen::Pins(Box::default());
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    state.pinned = vec![PinnedMapping {
        local_path: home.join("code/very/deeply/nested/project/config.json"),
        gist_id: "g1".into(),
        gist_filename: "config.json".into(),
        direction: None,
        last_seen_hash: None,
    }];
    pins_mut(&mut state).cursor.index = 0;
    state
}

#[test]
fn create_diff_title_shortens_home_path() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Create {
            local_path: home.join("notes.txt"),
        },
    );
    assert_eq!(diff_title(&state), "Create gist from ~/notes.txt");
}

#[test]
fn diff_view_title_shortens_single_home_path() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    let mut state = initial_state();
    state.enter_diff(
        String::new(),
        String::new(),
        PathBuf::new(),
        home.join("notes.txt"),
    );
    assert_eq!(diff_title(&state), "Diff → ~/notes.txt");
}

#[test]
fn diff_view_title_shortens_both_home_paths() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    let mut state = initial_state();
    state.enter_diff(
        String::new(),
        String::new(),
        home.join("src").join("a.txt"),
        home.join("b.txt"),
    );
    assert_eq!(diff_title(&state), "Diff: ~/src/a.txt → ~/b.txt");
}

#[test]
fn create_confirm_prompt_shortens_home_path() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    let mut state = initial_state();
    set_pending(
        &mut state,
        PendingAction::Create {
            local_path: home.join("notes.txt"),
        },
    );
    assert!(
        confirm_prompt(&state).starts_with("Create gist from ~/notes.txt"),
        "got {}",
        confirm_prompt(&state)
    );
}

#[test]
fn pin_row_label_shows_home_as_tilde() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    let label = pin_row_label(PinLabelParams {
        icon: "✓",
        local_path: &home.join("code/gistui"),
        gist_id: "abc123",
        gist_description: None,
        gist_filename: "notes.txt",
        local_age: "2h",
        gist_age: "3h",
    });
    assert!(
        label.contains("~/code/gistui"),
        "expected ~ home in label, got {label}"
    );
    assert!(!label.contains(home.to_string_lossy().as_ref()));
}

/// Issue #347: the gist description leads the identity (before the filename), and the full id
/// is demoted to a trailing, `#`-prefixed abbreviation rather than sitting between the local
/// path and the filename.
#[test]
fn pin_row_label_leads_with_description_and_abbreviates_id() {
    let label = pin_row_label(PinLabelParams {
        icon: "✓",
        local_path: std::path::Path::new("/tmp/x"),
        gist_id: "abcdef0123456789abcdef0123456789",
        gist_description: Some("My cool gist"),
        gist_filename: "notes.txt",
        local_age: "2h",
        gist_age: "3h",
    });
    assert!(
        label.contains("My cool gist / notes.txt"),
        "description should lead the filename: {label}"
    );
    assert!(
        !label.contains("abcdef0123456789abcdef0123456789"),
        "full id must not appear: {label}"
    );
    assert!(
        label.contains("#abcdef0"),
        "abbreviated id should still be reachable: {label}"
    );
}

/// Issue #347: without a known description, the identity falls back to the filename alone
/// (not a redundant "filename / filename").
#[test]
fn pin_row_label_falls_back_to_filename_without_description() {
    let label = pin_row_label(PinLabelParams {
        icon: "✓",
        local_path: std::path::Path::new("/tmp/x"),
        gist_id: "abc123",
        gist_description: None,
        gist_filename: "notes.txt",
        local_age: "2h",
        gist_age: "3h",
    });
    assert!(label.contains("↔  notes.txt "), "got {label}");
    assert!(!label.contains("notes.txt / notes.txt"));
}

#[test]
fn entering_pins_screen_resets_hscroll() {
    let mut state = pins_state_with_long_home_path();
    state.handle_key(KeyCode::Right);
    assert!(pins_ref(&state).cursor.hscroll > 0);
    state.screen = Screen::List;
    state.handle_key(KeyCode::Char('P'));
    assert!(state.screen.is_pins());
    assert_eq!(pins_ref(&state).cursor.hscroll, 0);
}

#[test]
fn top_bar_pins_click_opens_pins_from_any_screen() {
    let mut state = pins_state_with_long_home_path();
    state.handle_key(KeyCode::Right); // dirty the hscroll so the reset is observable
    assert!(pins_ref(&state).cursor.hscroll > 0);
    state.screen = Screen::Preview(Box::default());
    let layout = MouseLayout {
        top_bar_pins: Some(Rect::new(20, 0, 6, 1)),
        ..Default::default()
    };
    let out = state.handle_mouse(MouseInput::Click { col: 22, row: 0 }, &layout);
    assert!(state.screen.is_pins());
    assert_eq!(pins_ref(&state).cursor.hscroll, 0);
    assert_eq!(out, KeyOutcome::None);
}

#[test]
fn top_bar_help_click_while_already_on_help_does_not_trap_keyboard_exit() {
    let mut state = state_with_gists();
    state.screen = Screen::Preview(Box::default());
    let layout = MouseLayout {
        top_bar_help: Some(Rect::new(30, 0, 7, 1)),
        ..Default::default()
    };
    // First click opens Help from Preview, remembering Preview as the return screen.
    state.handle_mouse(MouseInput::Click { col: 32, row: 0 }, &layout);
    assert!(state.screen.is_help());
    assert!(state.nav_stack.last().is_some_and(Screen::is_preview));

    // A second click on the same top-bar Help hotspot, now that Help is already open, must
    // be a no-op — it must not overwrite return_screen with Screen::Help, which would trap
    // Esc/`?`/the close button in Help with no keyboard way out.
    let out = state.handle_mouse(MouseInput::Click { col: 32, row: 0 }, &layout);
    assert!(state.screen.is_help());
    assert!(state.nav_stack.last().is_some_and(Screen::is_preview));
    assert_eq!(out, KeyOutcome::None);

    // Esc must still return to the real origin screen, not stay stuck on Help.
    state.handle_key(KeyCode::Esc);
    assert!(state.screen.is_preview());
}

#[test]
fn list_screen_capital_s_syncs_selected_pair() {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("a.txt"),
        pinned: true,
        modified: None,
    }];
    state.gists = vec![GistFile::fixture("g1", "a.txt")];
    assert!(matches!(
        state.handle_key(KeyCode::Char('S')),
        KeyOutcome::SyncSelectedPair { .. }
    ));
}

#[test]
fn gist_group_row_age_tracks_active_sort() {
    let group = GistGroup {
        id: "g1".into(),
        description: "demo".into(),
        public: false,
        updated_at: "2026-06-10T00:00:00Z".into(),
        created_at: "2026-06-01T00:00:00Z".into(),
        file_count: 2,
        owner_login: String::new(),
        fork_of_id: None,
    };
    let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
    // Sorting by updated shows the updated age (1 day ago); sorting by created shows the
    // created age (10 days ago → "1w"), so the 🕒 column matches the ordering key.
    let updated = gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None);
    let created = gist_group_row_label(&group, now, GistGroupSort::Created, (0, 0, 0), false, None);
    assert!(updated.ends_with("🕒 1d"), "{updated}");
    assert!(created.ends_with("🕒 1w"), "{created}");
}

#[test]
fn gist_group_row_shows_comment_marker_only_when_present() {
    let group = GistGroup {
        id: "g1".into(),
        description: "demo".into(),
        public: false,
        updated_at: "2026-06-10T00:00:00Z".into(),
        created_at: "2026-06-01T00:00:00Z".into(),
        file_count: 2,
        owner_login: String::new(),
        fork_of_id: None,
    };
    let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
    assert!(
        !gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None)
            .contains('💬')
    );
    assert!(
        gist_group_row_label(&group, now, GistGroupSort::Updated, (3, 0, 0), false, None)
            .contains("💬 3")
    );
}

#[test]
fn gist_group_row_shows_foreign_owner() {
    let group = GistGroup {
        id: "g1".into(),
        description: "demo".into(),
        public: true,
        updated_at: "2026-06-10T00:00:00Z".into(),
        created_at: "2026-06-01T00:00:00Z".into(),
        file_count: 1,
        owner_login: "karpathy".into(),
        fork_of_id: None,
    };
    let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
    let foreign = gist_group_row_label(
        &group,
        now,
        GistGroupSort::Updated,
        (0, 0, 0),
        false,
        Some("me"),
    );
    assert!(foreign.contains("@karpathy"));
    let own = gist_group_row_label(
        &group,
        now,
        GistGroupSort::Updated,
        (0, 0, 0),
        false,
        Some("karpathy"),
    );
    assert!(!own.contains("@karpathy"));
}

#[test]
fn gist_group_row_shows_fork_marker_only_when_present() {
    let group = GistGroup {
        id: "g1".into(),
        description: "demo".into(),
        public: false,
        updated_at: "2026-06-10T00:00:00Z".into(),
        created_at: "2026-06-01T00:00:00Z".into(),
        file_count: 2,
        owner_login: String::new(),
        fork_of_id: None,
    };
    let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
    assert!(
        !gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None)
            .contains('⑂')
    );
    assert!(
        gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 2), false, None)
            .contains("⑂ 2")
    );
}

#[test]
fn gist_group_row_shows_star_marker_only_when_present() {
    let group = GistGroup {
        id: "g1".into(),
        description: "demo".into(),
        public: false,
        updated_at: "2026-06-10T00:00:00Z".into(),
        created_at: "2026-06-01T00:00:00Z".into(),
        file_count: 2,
        owner_login: String::new(),
        fork_of_id: None,
    };
    let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
    assert!(
        !gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None)
            .contains('☆')
    );
    assert!(
        gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 3, 0), false, None)
            .contains("☆ 3")
    );
}

/// Issue #347: the description leads the row (after the fixed-width badge/owner columns), and
/// the full 32-char id no longer dominates it — only a short, `#`-prefixed abbreviation trails.
#[test]
fn gist_group_row_description_leads_and_id_is_abbreviated() {
    let group = GistGroup {
        id: "abcdef0123456789abcdef0123456789".into(),
        description: "My cool gist".into(),
        public: false,
        updated_at: "2026-06-10T00:00:00Z".into(),
        created_at: "2026-06-01T00:00:00Z".into(),
        file_count: 2,
        owner_login: String::new(),
        fork_of_id: None,
    };
    let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
    let row = gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None);
    assert!(
        row.trim_start().starts_with("My cool gist"),
        "description should lead the row, got {row}"
    );
    assert!(!row.contains(&group.id), "full id must not appear: {row}");
    assert!(
        row.contains(&format!("#{}", &group.id[..7])),
        "abbreviated id should still be reachable inline: {row}"
    );
}

/// Issue #347: the badge column is fixed-width, so a starred row's description starts at the
/// same column as an unstarred row's.
#[test]
fn gist_group_row_badge_column_is_fixed_width() {
    let group = GistGroup {
        id: "g1".into(),
        description: "demo".into(),
        public: false,
        updated_at: "2026-06-10T00:00:00Z".into(),
        created_at: "2026-06-01T00:00:00Z".into(),
        file_count: 1,
        owner_login: String::new(),
        fork_of_id: None,
    };
    let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
    let unbadged =
        gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None);
    let starred = gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), true, None);
    // Compare by char count, not byte offset — `★` is multi-byte, so a byte-offset comparison
    // would report misalignment even though the two rows line up on screen.
    let char_col = |s: &str| s.find("demo").map(|byte_idx| s[..byte_idx].chars().count());
    assert_eq!(
        char_col(&unbadged),
        char_col(&starred),
        "description column must align with and without a badge: {unbadged:?} vs {starred:?}"
    );
}

/// Issue #347: a legacy (shorter than the abbreviation width) gist id still pads the id column
/// out to its usual width, so the `📄` segment that follows stays aligned across rows.
#[test]
fn gist_group_row_legacy_short_id_still_aligns() {
    let short = GistGroup {
        id: "abc12".into(),
        description: "demo".into(),
        public: false,
        updated_at: "2026-06-10T00:00:00Z".into(),
        created_at: "2026-06-01T00:00:00Z".into(),
        file_count: 1,
        owner_login: String::new(),
        fork_of_id: None,
    };
    let long = GistGroup {
        id: "abcdef0123456789".into(),
        ..short.clone()
    };
    let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
    let short_row =
        gist_group_row_label(&short, now, GistGroupSort::Updated, (0, 0, 0), false, None);
    let long_row = gist_group_row_label(&long, now, GistGroupSort::Updated, (0, 0, 0), false, None);
    assert_eq!(
        short_row.find('📄'),
        long_row.find('📄'),
        "the file-count marker must land at the same column regardless of id length: \
         {short_row:?} vs {long_row:?}"
    );
}

#[test]
fn gist_info_line_shows_counts_when_nonzero() {
    let group = GistGroup {
        id: "616796de59282c8bfdae3005511c588e".into(),
        description: "demo".into(),
        public: true,
        updated_at: "2026-06-10T00:00:00Z".into(),
        created_at: "2026-06-01T00:00:00Z".into(),
        file_count: 1,
        owner_login: String::new(),
        fork_of_id: None,
    };
    let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
    let quiet = gist_info_line(&group, now, None, false, (0, 0, 0));
    assert!(!quiet.contains('☆'));
    assert!(!quiet.contains('⑂'));
    assert!(!quiet.contains('💬'));

    let rich = gist_info_line(&group, now, None, true, (2, 3, 1));
    assert!(rich.starts_with("★ starred · "));
    assert!(rich.contains("☆ 3 · ⑂ 1 · 💬 2"));
    assert!(rich.contains(&group.id));
}

// A gist you own *and* starred lands in both `gists` and `starred_gists`. The detail
// file list (gist_filenames -> all_gist_files) must not show each file twice (issue #188).
#[test]
fn gist_filenames_dedupes_owned_gist_that_is_also_starred() {
    let make = |filename: &str| GistFile {
        description: "My ZSH profile".into(),
        public: true,
        updated_at: "2026-06-10T00:00:00Z".into(),
        created_at: "2020-01-01T00:00:00Z".into(),
        owner_login: "akunzai".into(),
        ..GistFile::fixture("g1", filename)
    };
    let mut state = initial_state();
    state.gists = vec![make(".zprofile"), make(".zshenv"), make(".zshrc")];
    // Same gist, fetched again from /gists/starred because the owner starred it.
    state.starred_gists = vec![make(".zprofile"), make(".zshenv"), make(".zshrc")];

    assert_eq!(
        state.gist_filenames("g1"),
        vec![".zprofile", ".zshenv", ".zshrc"]
    );
    assert_eq!(state.gist_file_display_names("g1").len(), 3);
}

#[test]
fn list_filter_routes_chars_to_focused_pane() {
    let mut state = state_with_local_paths(&["/cwd/a.json", "/cwd/b.txt"]);
    state.focus = FocusPane::Local;
    state.filtering = true;

    state.handle_key(KeyCode::Char('j'));
    state.handle_key(KeyCode::Char('s'));
    assert_eq!(state.local_filter_query, "js");
    assert_eq!(state.filter_query, ""); // gist pane untouched
}

#[test]
fn list_filter_focus_gist_routes_to_gist_query() {
    let mut state = state_with_local_paths(&["/cwd/a.json"]);
    state.focus = FocusPane::Gist;
    state.filtering = true;

    state.handle_key(KeyCode::Char('x'));
    assert_eq!(state.filter_query, "x");
    assert_eq!(state.local_filter_query, "");
}

#[test]
fn list_filter_navigates_while_typing() {
    let mut state = state_with_local_paths(&["/cwd/a.txt", "/cwd/b.txt", "/cwd/c.txt"]);
    state.focus = FocusPane::Local;
    state.filtering = true;

    state.handle_key(KeyCode::Down);
    assert_eq!(state.local_index, 1);
    assert!(state.filtering); // still in filter input
    state.handle_key(KeyCode::Up);
    assert_eq!(state.local_index, 0);
}

#[test]
fn list_filter_empty_backspace_exits() {
    let mut state = state_with_local_paths(&["/cwd/a.txt"]);
    state.focus = FocusPane::Local;
    state.filtering = true;

    state.handle_key(KeyCode::Char('a'));
    state.handle_key(KeyCode::Backspace); // back to empty, still filtering
    assert!(state.filtering);
    assert_eq!(state.local_filter_query, "");
    state.handle_key(KeyCode::Backspace); // empty -> exit
    assert!(!state.filtering);
}

#[test]
fn list_filter_tab_commits_and_switches_pane() {
    let mut state = state_with_local_paths(&["/cwd/a.json"]);
    state.focus = FocusPane::Local;
    state.filtering = true;
    state.handle_key(KeyCode::Char('j'));

    state.handle_key(KeyCode::Tab);
    assert!(!state.filtering); // committed, left input
    assert_eq!(state.local_filter_query, "j"); // query kept
    assert_eq!(state.focus, FocusPane::Gist); // switched pane
}

#[test]
fn list_filter_esc_clears_focused_query() {
    let mut state = state_with_local_paths(&["/cwd/a.json"]);
    state.focus = FocusPane::Local;
    state.filtering = true;
    state.handle_key(KeyCode::Char('j'));

    state.handle_key(KeyCode::Esc);
    assert!(!state.filtering);
    assert_eq!(state.local_filter_query, "");
}

#[test]
fn list_filter_char_resets_focused_index() {
    let mut state = state_with_local_paths(&["/cwd/a.txt", "/cwd/ab.txt", "/cwd/abc.txt"]);
    state.focus = FocusPane::Local;
    state.filtering = true;
    state.local_index = 2; // cursor not at top

    state.handle_key(KeyCode::Char('a')); // edit -> reset to top
    assert_eq!(state.local_index, 0);
}

#[test]
fn list_filter_enter_keeps_query_and_exits() {
    let mut state = state_with_local_paths(&["/cwd/a.json"]);
    state.focus = FocusPane::Local;
    state.filtering = true;
    state.handle_key(KeyCode::Char('j'));

    state.handle_key(KeyCode::Enter);
    assert!(!state.filtering); // exited input
    assert_eq!(state.local_filter_query, "j"); // query kept
}

// ── Pins screen filter ────────────────────────────────────────────────────────

#[test]
fn help_topic_all_is_ordered_and_titled() {
    let all = HelpTopic::all();
    assert_eq!(all.len(), 11);
    assert_eq!(all[0], HelpTopic::List);
    assert_eq!(all[4], HelpTopic::Revisions);
    assert_eq!(all[8], HelpTopic::Config);
    assert_eq!(all[9], HelpTopic::General);
    assert_eq!(all[10], HelpTopic::About);
    assert_eq!(HelpTopic::Pins.title(), "Pinned Mappings");
    assert_eq!(HelpTopic::Config.title(), "Settings");
    assert_eq!(HelpTopic::About.title(), "About");
}

#[test]
fn help_topic_for_screen_maps_key_dense_screens() {
    assert_eq!(HelpTopic::for_screen(&Screen::List), HelpTopic::List);
    assert_eq!(
        HelpTopic::for_screen(&Screen::Pins(Box::default())),
        HelpTopic::Pins
    );
    assert_eq!(
        HelpTopic::for_screen(&Screen::Gists(Box::default())),
        HelpTopic::GistManager
    );
    assert_eq!(
        HelpTopic::for_screen(&Screen::GistDetail(Box::default())),
        HelpTopic::GistDetail
    );
    assert_eq!(
        HelpTopic::for_screen(&Screen::Revisions(Box::default())),
        HelpTopic::Revisions
    );
    assert_eq!(
        HelpTopic::for_screen(&Screen::Diff(Box::default())),
        HelpTopic::List
    );
}

#[test]
fn capital_h_from_list_opens_revisions_for_selected_gist_file() {
    let mut state = list_state_with_matches();
    state.focus = FocusPane::Gist;
    state.gist_index = 0;
    let outcome = state.handle_key(KeyCode::Char('H'));
    assert!(matches!(outcome, KeyOutcome::FetchRevisions { .. }));
    assert!(state.screen.is_revisions());
    assert_eq!(revision_ref(&state).gist_id.as_deref(), Some("a"));
    assert_eq!(revision_ref(&state).target_file, "settings.json");
    assert_eq!(state.nav_stack.last(), Some(&Screen::List));
}

#[test]
fn capital_h_from_gist_detail_opens_revisions_and_fetches() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail(Box::default());
    detail_mut(&mut state).gist_id = Some("g1".into());
    detail_mut(&mut state).file_cursor = 1;
    let outcome = state.handle_key(KeyCode::Char('H'));
    assert!(matches!(outcome, KeyOutcome::FetchRevisions { .. }));
    assert!(state.screen.is_revisions());
    assert_eq!(revision_ref(&state).gist_id.as_deref(), Some("g1"));
    assert_eq!(revision_ref(&state).target_file, "b.txt");
    assert!(state.nav_stack.last().is_some_and(Screen::is_gist_detail));
    assert!(revision_ref(&state).entries.is_none());
}

#[test]
fn vim_j_k_move_list_selection() {
    let mut state = list_state_with_matches();
    state.focus = FocusPane::Gist;
    state.gist_index = 0;
    state.handle_key(KeyCode::Char('j'));
    assert_eq!(state.gist_index, 1);
    state.handle_key(KeyCode::Char('k'));
    assert_eq!(state.gist_index, 0);
}

#[test]
fn vim_h_scrolls_focused_row_left() {
    let mut state = list_state_with_matches();
    state.focus = FocusPane::Gist;
    state.gist_hscroll = 2;
    state.handle_key(KeyCode::Char('h'));
    assert_eq!(state.gist_hscroll, 1);
}

#[test]
fn shift_t_toggles_theme() {
    use crossterm::event::KeyModifiers;
    let mut state = initial_state();
    assert_eq!(state.theme_choice, crate::config::ThemeChoice::Dark);
    let outcome = state.handle_key_with(KeyCode::Char('T'), KeyModifiers::SHIFT);
    assert_eq!(outcome, KeyOutcome::ThemeToggle);
    assert_eq!(state.theme_choice, crate::config::ThemeChoice::Light);
}

#[test]
fn list_page_keys_jump_local_selection() {
    let paths: Vec<String> = (0..15).map(|i| format!("/cwd/f{i:02}.txt")).collect();
    let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    let mut state = state_with_local_paths(&path_refs);
    state.focus = FocusPane::Local;
    state.handle_key(KeyCode::PageDown);
    assert_eq!(state.local_index, 10);
    state.handle_key(KeyCode::PageDown);
    assert_eq!(state.local_index, 14);
    state.handle_key(KeyCode::PageUp);
    assert_eq!(state.local_index, 4);
}

#[test]
fn list_filter_ctrl_f_pages_without_typing_f() {
    use crossterm::event::KeyModifiers;
    let paths: Vec<String> = (0..12).map(|i| format!("/cwd/f{i:02}.txt")).collect();
    let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    let mut state = state_with_local_paths(&path_refs);
    state.focus = FocusPane::Local;
    state.filtering = true;
    state.local_filter_query.set("f");
    state.handle_key_with(KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert_eq!(state.local_index, 10);
    assert_eq!(state.local_filter_query, "f");
}

#[test]
fn question_mark_opens_contextual_help_from_pins() {
    let mut state = initial_state();
    state.screen = Screen::Pins(Box::default());
    state.handle_key(KeyCode::Char('?'));
    assert!(state.screen.is_help());
    assert_eq!(help_ref(&state).topic, HelpTopic::Pins);
    assert!(state.nav_stack.last().is_some_and(Screen::is_pins));
    assert!(!help_ref(&state).index_open);
}

#[test]
fn repo_link_click_opens_repo_url_regardless_of_which_screen_set_the_rect() {
    let mut state = initial_state();
    let layout = MouseLayout {
        repo_link: Some(Rect::new(5, 10, 20, 1)),
        ..Default::default()
    };
    let out = state.handle_mouse(MouseInput::Click { col: 10, row: 10 }, &layout);
    assert!(matches!(out, KeyOutcome::OpenRepoUrl { .. }));
}

#[test]
fn help_topic_view_esc_returns_to_origin() {
    let mut state = initial_state();
    state.screen = Screen::Help(Box::default());
    state.nav_stack.push(Screen::Gists(Box::default()));
    state.handle_key(KeyCode::Esc);
    assert!(state.screen.is_gists());
}

#[test]
fn help_index_esc_returns_to_origin() {
    let mut state = initial_state();
    state.screen = Screen::Help(Box::default());
    help_mut(&mut state).index_open = true;
    state.nav_stack.push(Screen::List);
    state.handle_key(KeyCode::Esc);
    assert_eq!(state.screen, Screen::List);
    assert!(state.help().is_none());
}

#[test]
fn help_index_question_mark_exits_help() {
    let mut state = initial_state();
    state.screen = Screen::Help(Box::default());
    help_mut(&mut state).index_open = true;
    state.nav_stack.push(Screen::Pins(Box::default()));
    state.handle_key(KeyCode::Char('?'));
    assert!(state.screen.is_pins());
    assert!(state.help().is_none());
}

#[test]
fn pin_mtimes_local_falls_back_to_disk_when_not_discovered() {
    // A pin pointing outside cwd is absent from state.locals, but the Pins list
    // and sync status should still reflect the file's real mtime by stat-ing it.
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("settings.json");
    std::fs::write(&outside, "{}").unwrap();

    let mut state = initial_state();
    state.locals.clear();
    state.pinned = vec![crate::domain::PinnedMapping {
        local_path: outside.clone(),
        gist_id: "g1".into(),
        gist_filename: "settings.json".into(),
        direction: None,
        last_seen_hash: None,
    }];

    let (local_ts, _remote_ts) = state.pin_mtimes(0);
    assert!(
        local_ts.is_some(),
        "local mtime should fall back to disk for pins outside cwd"
    );
}

#[test]
fn pin_sync_status_is_missing_when_local_file_absent() {
    // A pinned local path that doesn't exist on disk should report Missing,
    // not the generic Unknown ambiguity used when a timestamp is merely
    // unavailable for other reasons.
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("settings.json");
    // Deliberately never created — this path must not exist.

    let mut state = initial_state();
    state.locals.clear();
    state.pinned = vec![crate::domain::PinnedMapping {
        local_path: gone,
        gist_id: "g1".into(),
        gist_filename: "settings.json".into(),
        direction: None,
        last_seen_hash: None,
    }];
    state.gists = vec![GistFile {
        updated_at: "2026-01-01T00:00:00Z".into(),
        ..GistFile::fixture("g1", "settings.json")
    }];

    assert_eq!(
        {
            state.refresh_pin_sync_cache();
            state.cached_pin_sync_status(0)
        },
        crate::domain::SyncStatus::Missing,
        "a pin whose local file doesn't exist must report Missing even though \
         the gist side has a known mtime"
    );
}

#[test]
fn pin_sync_status_upgrades_to_in_sync_when_content_hash_matches_baseline() {
    // Timestamps disagree (forcing Push), but the content hash still matches what was
    // last recorded as synced — the Pins list should show synced (✓), not a misleading
    // push arrow, since nothing has actually changed content-wise.
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("settings.json");
    let content = b"{\"key\":\"value\"}";
    std::fs::write(&local, content).unwrap();
    let hash = crate::domain::sha256_hex(content);

    let mut state = initial_state();
    state.locals.clear();
    state.pinned = vec![crate::domain::PinnedMapping {
        local_path: local,
        gist_id: "g1".into(),
        gist_filename: "settings.json".into(),
        direction: None,
        last_seen_hash: Some(hash),
    }];
    state.gists = vec![GistFile {
        // Far in the past, so the just-written local file (mtime ~ now) reads as newer —
        // sync_status(Some(local_ts), Some(remote_ts)) would normally resolve to Push.
        updated_at: "2020-01-01T00:00:00Z".into(),
        ..GistFile::fixture("g1", "settings.json")
    }];

    assert_eq!(
        {
            state.refresh_pin_sync_cache();
            state.cached_pin_sync_status(0)
        },
        crate::domain::SyncStatus::InSync,
        "a matching content hash must override a stale-timestamp Push into InSync"
    );
}

#[test]
fn pin_sync_status_keeps_push_when_content_hash_does_not_match_baseline() {
    // Same timestamp setup as above, but the recorded baseline hash doesn't match the
    // file's actual current content — a real, unrecorded local change. Must stay Push.
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("settings.json");
    std::fs::write(&local, b"{\"key\":\"value\"}").unwrap();

    let mut state = initial_state();
    state.locals.clear();
    state.pinned = vec![crate::domain::PinnedMapping {
        local_path: local,
        gist_id: "g1".into(),
        gist_filename: "settings.json".into(),
        direction: None,
        last_seen_hash: Some("does-not-match-anything".into()),
    }];
    state.gists = vec![GistFile {
        updated_at: "2020-01-01T00:00:00Z".into(),
        ..GistFile::fixture("g1", "settings.json")
    }];

    assert_eq!(
        {
            state.refresh_pin_sync_cache();
            state.cached_pin_sync_status(0)
        },
        crate::domain::SyncStatus::Push,
        "a non-matching baseline hash must not mask a real content change"
    );
}

#[test]
fn pin_sync_status_keeps_push_when_no_baseline_hash_recorded() {
    // Regression guard: a pin that was never synced (no baseline hash at all) must fall
    // back to the plain timestamp-based status, not attempt a hash comparison.
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("settings.json");
    std::fs::write(&local, b"{\"key\":\"value\"}").unwrap();

    let mut state = initial_state();
    state.locals.clear();
    state.pinned = vec![crate::domain::PinnedMapping {
        local_path: local,
        gist_id: "g1".into(),
        gist_filename: "settings.json".into(),
        direction: None,
        last_seen_hash: None,
    }];
    state.gists = vec![GistFile {
        updated_at: "2020-01-01T00:00:00Z".into(),
        ..GistFile::fixture("g1", "settings.json")
    }];

    state.refresh_pin_sync_cache();
    assert_eq!(
        state.cached_pin_sync_status(0),
        crate::domain::SyncStatus::Push
    );
}

#[test]
fn forked_filter_shows_only_forks() {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            description: "mine".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: "me".into(),
            ..GistFile::fixture("owned", "a.txt")
        },
        GistFile {
            description: "fork".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: "me".into(),
            fork_of_id: Some("upstream".into()),
            ..GistFile::fixture("forked", "b.txt")
        },
    ];
    state.current_user_login = Some("me".into());
    state.gist_type_filter = GistTypeFilter::Forked;
    let ids: Vec<_> = state
        .ranked_gists()
        .into_iter()
        .map(|g| g.file.gist_id)
        .collect();
    assert_eq!(ids, vec!["forked"]);
}

#[test]
fn foreign_gist_blocks_pin() {
    let mut state = initial_state();
    state.current_user_login = Some("me".into());
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/cwd/a.txt"),
        pinned: false,
        modified: None,
    }];
    state.gists = vec![GistFile {
        description: "x".into(),
        public: true,
        updated_at: "x".into(),
        created_at: "x".into(),
        owner_login: "other".into(),
        ..GistFile::fixture("foreign", "a.txt")
    }];
    state.local_index = 0;
    state.gist_index = 0;
    assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::None);
    assert!(state.status.as_ref().unwrap().contains("cannot pin"));
}

#[test]
fn star_key_returns_toggle_intent() {
    let mut state = initial_state();
    state.gists = vec![GistFile {
        description: "x".into(),
        public: true,
        updated_at: "x".into(),
        created_at: "x".into(),
        ..GistFile::fixture("g1", "a.txt")
    }];
    state.gist_index = 0;
    assert!(matches!(
        state.handle_key(KeyCode::Char('*')),
        KeyOutcome::ToggleGistStar { .. }
    ));
}

#[test]
fn starred_filter_lists_only_starred_gists() {
    // With the Starred type filter active, ranked_gists must draw from starred_gists, not the
    // owned list — exercises the owned/starred source switch with data on both sides.
    let mut state = initial_state();
    state.gists = vec![GistFile {
        description: "mine".into(),
        public: true,
        updated_at: "x".into(),
        created_at: "x".into(),
        owner_login: "me".into(),
        ..GistFile::fixture("owned", "a.txt")
    }];
    state.starred_gists = vec![GistFile {
        description: "theirs".into(),
        public: true,
        updated_at: "x".into(),
        created_at: "x".into(),
        owner_login: "other".into(),
        ..GistFile::fixture("starred", "b.txt")
    }];
    state.gist_type_filter = GistTypeFilter::Starred;

    let ranked = state.ranked_gists();
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].file.gist_id, "starred");
}

#[test]
fn initial_state_enables_mouse_by_default() {
    assert!(super::initial_state().mouse_enabled);
}

#[test]
fn pane_hit_maps_rows_to_indices() {
    // A pane at y=2, height 6: top border row 2, content rows 3..=6, bottom border row 7.
    let hit = PaneHit {
        rect: Rect::new(0, 2, 40, 6),
        offset: 0,
    };
    assert_eq!(hit.index_at(3, 4), Some(0)); // first content row
    assert_eq!(hit.index_at(6, 4), Some(3)); // fourth content row
    assert_eq!(hit.index_at(2, 4), None); // top border
    assert_eq!(hit.index_at(7, 4), None); // bottom border
    assert_eq!(hit.index_at(6, 2), None); // row maps to idx 3 >= visible_len 2
}

#[test]
fn pane_hit_respects_scroll_offset() {
    let hit = PaneHit {
        rect: Rect::new(0, 0, 40, 10),
        offset: 5,
    };
    // content starts at row 1; row 1 -> offset 5
    assert_eq!(hit.index_at(1, 20), Some(5));
    assert_eq!(hit.index_at(3, 20), Some(7));
}

#[test]
fn pane_hit_empty_list_selects_nothing() {
    let hit = PaneHit {
        rect: Rect::new(0, 0, 40, 10),
        offset: 0,
    };
    assert_eq!(hit.index_at(1, 0), None);
}

#[test]
fn classify_click_detects_double_click() {
    // Same cell within the threshold -> DoubleClick.
    let r = super::classify_click(Some((5, 5)), 100, 5, 5);
    assert_eq!(r, MouseInput::DoubleClick { col: 5, row: 5 });
}

#[test]
fn classify_click_single_when_too_slow() {
    let r = super::classify_click(Some((5, 5)), super::DOUBLE_CLICK_MS + 1, 5, 5);
    assert_eq!(r, MouseInput::Click { col: 5, row: 5 });
}

#[test]
fn classify_click_single_on_different_cell() {
    let r = super::classify_click(Some((5, 5)), 100, 6, 5);
    assert_eq!(r, MouseInput::Click { col: 6, row: 5 });
}

#[test]
fn classify_click_single_when_no_prior() {
    let r = super::classify_click(None, 0, 5, 5);
    assert_eq!(r, MouseInput::Click { col: 5, row: 5 });
}

#[test]
fn classify_click_at_exact_threshold() {
    // Exactly at the boundary: still counts as a double-click (inclusive `<=`).
    let r = super::classify_click(Some((5, 5)), super::DOUBLE_CLICK_MS, 5, 5);
    assert_eq!(r, MouseInput::DoubleClick { col: 5, row: 5 });
}

// ── handle_mouse tests ────────────────────────────────────────────────────────

#[test]
fn scroll_down_moves_content_three_lines() {
    // Set up a Diff screen with enough lines that diff_scroll can reach 3.
    let mut state = state_with_selection();
    state.enter_diff(
        "line1\nline2\nline3\nline4\nline5".into(),
        "remote".into(),
        std::path::PathBuf::from("/tmp/x"),
        std::path::PathBuf::from("/tmp/cwd/x"),
    );
    assert!(state.screen.is_diff());
    assert_eq!(state.diff_scroll(), 0);
    state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
    assert_eq!(state.diff_scroll(), 3);
}

#[test]
fn scroll_up_moves_content_three_lines() {
    let mut state = state_with_selection();
    state.enter_diff(
        "line1\nline2\nline3\nline4\nline5".into(),
        "remote".into(),
        std::path::PathBuf::from("/tmp/x"),
        std::path::PathBuf::from("/tmp/cwd/x"),
    );
    set_diff_scroll(&mut state, 3);
    state.handle_mouse(MouseInput::ScrollUp, &MouseLayout::default());
    assert_eq!(state.diff_scroll(), 0);
}

#[test]
fn close_button_click_returns_from_help() {
    let mut state = state_with_gists();
    // Simulate entering Help (mirrors what open_help() does).
    state.screen = Screen::Help(Box::default());
    let layout = MouseLayout {
        close_button: Some(Rect::new(36, 0, 5, 1)),
        ..Default::default()
    };
    let out = state.handle_mouse(MouseInput::Click { col: 38, row: 0 }, &layout);
    assert_eq!(out, KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn close_button_click_confirm_cancel_clears_pending() {
    // Close button on Screen::Confirm(Box::default()) dispatches Esc, which cancels the pending action.
    // Using PendingAction::Download: Esc sets pending_action = None and screen = Screen::Diff(Box::default()).
    let mut state = state_with_gists();
    set_diff_body(&mut state, "line1\nline2\nline3");
    set_pending(&mut state, PendingAction::Download);
    let layout = MouseLayout {
        close_button: Some(Rect::new(36, 0, 5, 1)),
        ..Default::default()
    };
    let out = state.handle_mouse(MouseInput::Click { col: 38, row: 0 }, &layout);
    assert_eq!(out, KeyOutcome::None);
    assert!(state.pending_action().is_none());
    assert!(state.screen.is_diff());
}

#[test]
fn close_button_click_create_description_cancels_not_types() {
    // Regression: close button while editing the create-description sub-state must cancel
    // (Esc), NOT append 'n' to the description field.  This test fails against the old
    // `KeyCode::Char('n')` dispatch and passes with `KeyCode::Esc`.
    let mut state = initial_state();
    state.screen = Screen::Confirm(Box::default());
    set_pending(
        &mut state,
        PendingAction::Create {
            local_path: std::path::PathBuf::from("notes.txt"),
        },
    );
    state.editing_description = true;
    // Pre-fill description so we can assert it was cleared (not grown by a typed 'n').
    state.description_input = "my desc".into();
    let layout = MouseLayout {
        close_button: Some(Rect::new(36, 0, 5, 1)),
        ..Default::default()
    };
    state.handle_mouse(MouseInput::Click { col: 38, row: 0 }, &layout);
    // Esc on create-description clears description, exits editing, and calls back_to_list.
    assert!(
        !state.editing_description,
        "editing_description must be false after close"
    );
    assert!(
        state.description_input.is_empty(),
        "description must be cleared, not have 'n' appended"
    );
    assert_eq!(state.screen, Screen::List);
    assert!(state.pending_action().is_none());
}

// ── palette tests ─────────────────────────────────────────────────────────────

#[test]
fn ctrl_p_opens_command_palette() {
    let mut state = crate::tui::initial_state();
    state.handle_key_with(KeyCode::Char('p'), KeyModifiers::CONTROL);
    assert!(state.screen.is_palette());
    assert_eq!(
        state.palette().unwrap().mode,
        crate::tui::palette::PaletteMode::Command
    );
}

#[test]
fn menu_palette_hides_disabled_actions() {
    let mut state = crate::tui::initial_state();
    state.open_palette_menu(None);
    let labels: Vec<_> = state
        .palette_visible_items()
        .iter()
        .map(|i| i.label.as_str())
        .collect();
    assert!(!labels.iter().any(|l| l.contains("Upload")));
}

#[test]
fn command_palette_includes_cross_screen_quit() {
    let mut state = crate::tui::initial_state();
    state.open_palette_command();
    assert!(state
        .palette_visible_items()
        .iter()
        .any(|i| i.label == "Quit"));
}

#[test]
fn command_palette_fuzzy_filter_narrows_items() {
    let mut state = crate::tui::initial_state();
    state.open_palette_command();
    let before = state.palette_visible_items().len();
    state.palette_mut().unwrap().query.set("quit");
    let after = state.palette_visible_items().len();
    assert!(after < before);
    assert!(state
        .palette_visible_items()
        .iter()
        .all(|i| i.label.to_ascii_lowercase().contains("quit")));
}

#[test]
fn right_click_opens_menu_palette() {
    let mut state = crate::tui::initial_state();
    let out = state.handle_mouse(
        MouseInput::RightClick { col: 10, row: 5 },
        &MouseLayout::default(),
    );
    assert_eq!(out, KeyOutcome::None);
    assert!(state.screen.is_palette());
    assert_eq!(state.palette().unwrap().anchor, Some((10, 5)));
}

#[test]
fn palette_row_line_aligns_long_keys() {
    let item = PaletteItem {
        key_hint: "Enter".to_string(),
        label: "Diff local ↔ gist".to_string(),
        exec: crate::tui::palette::PaletteExec::Key(KeyCode::Enter, KeyModifiers::NONE),
        enabled: true,
        category: crate::tui::keymap::Category::Read,
        search: String::new(),
    };
    let line = palette_row_line(
        &item,
        palette_key_width(&[&item]),
        &Theme::DARK,
        Style::default(),
    );
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.starts_with("  Enter  Diff"));
    assert!(!text.contains("EnterDiff"));
}

#[test]
fn bg_task_generation_bumps_on_begin_and_invalidate() {
    let mut state = crate::tui::initial_state();
    assert_eq!(state.bg_task_generation, 0);
    assert_eq!(state.begin_bg_task(), 1);
    state.bg_task_msg = Some("working…".into());
    assert!(state.is_current_bg_generation(1));
    assert!(!state.is_current_bg_generation(0));

    state.invalidate_bg_task();
    assert_eq!(state.bg_task_generation, 2);
    assert!(state.bg_task_msg.is_none());
    assert!(
        !state.is_current_bg_generation(1),
        "cancelled gen must be stale"
    );
    assert!(state.is_current_bg_generation(2));
}

#[test]
fn local_scan_generation_ignores_stale_results() {
    let mut state = crate::tui::initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/old.txt"),
        pinned: false,
        modified: None,
    }];
    state.local_scanning = true;

    let gen1 = state.begin_local_scan();
    let gen2 = state.begin_local_scan();
    assert_ne!(gen1, gen2);

    // Stale gen1 must not replace the list.
    assert!(!state.apply_local_scan_if_current(
        gen1,
        vec![LocalCandidate {
            path: PathBuf::from("/tmp/stale.txt"),
            pinned: false,
            modified: None,
        }]
    ));
    assert_eq!(state.locals[0].path, PathBuf::from("/tmp/old.txt"));
    assert!(state.local_scanning);

    // Current gen2 applies.
    assert!(state.apply_local_scan_if_current(
        gen2,
        vec![LocalCandidate {
            path: PathBuf::from("/tmp/fresh.txt"),
            pinned: false,
            modified: None,
        }]
    ));
    assert_eq!(state.locals[0].path, PathBuf::from("/tmp/fresh.txt"));
    assert!(!state.local_scanning);
}

#[test]
fn config_field_values_round_trip_via_save_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut state = initial_state();
    state.theme_choice = crate::config::ThemeChoice::Light;
    state.config_mouse = false;
    state.config_check_updates = false;
    state.ignore_trailing_newline = false;
    state.scan_depth = 5;
    state.diff_context = 7;
    // Simulate persist_settings write path using shipped save/load.
    let config = crate::config::AppConfig {
        theme: state.theme_choice,
        mouse: state.config_mouse,
        check_updates: state.config_check_updates,
        ignore_trailing_newline: state.ignore_trailing_newline,
        scan_depth: state.scan_depth,
        diff_context: state.diff_context,
        ..crate::config::AppConfig::default()
    };
    crate::config::save_config(&path, &config).unwrap();
    assert!(path.exists());
    let loaded = crate::config::load_config(&path).unwrap();
    assert_eq!(loaded.theme, crate::config::ThemeChoice::Light);
    assert!(!loaded.mouse);
    assert!(!loaded.check_updates);
    assert!(!loaded.ignore_trailing_newline);
    assert_eq!(loaded.scan_depth, 5);
    assert_eq!(loaded.diff_context, 7);
}

#[test]
fn mouse_capture_applies_to_stdout_matches_is_terminal() {
    // Guard used by sync_mouse_capture: must agree with std's TTY check so CI
    // (non-TTY) skips execute! and real sessions still apply capture.
    use std::io::IsTerminal;
    assert_eq!(
        super::bg::mouse_capture_applies_to_stdout(),
        std::io::stdout().is_terminal()
    );
}

#[test]
fn persist_settings_dispatch_path_syncs_mouse_capture() {
    // Structural: PersistSettings arm must call sync_mouse_capture after save so a
    // Settings mouse toggle takes effect without restart (skeptic fix).
    let src = include_str!("dispatch.rs");
    let persist_idx = src
        .find("KeyOutcome::PersistSettings")
        .expect("PersistSettings arm");
    let arm = &src[persist_idx..persist_idx + 280];
    assert!(
        arm.contains("sync_mouse_capture"),
        "PersistSettings must sync terminal mouse capture: {arm}"
    );
    assert!(
        arm.contains("mouse_enabled"),
        "must pass current mouse_enabled into sync: {arm}"
    );
}
