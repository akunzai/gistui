use super::*;
use crossterm::event::KeyCode;
use ratatui::style::{Color, Modifier};
use std::path::PathBuf;

fn state_with_gists() -> AppState {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            gist_id: "g1".into(),
            description: "demo".into(),
            filename: "a.txt".into(),
            public: false,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
        },
        GistFile {
            gist_id: "g1".into(),
            description: "demo".into(),
            filename: "b.txt".into(),
            public: false,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
        },
    ];
    state.gists_index = 0;
    state
}

fn state_with_many_files(n: usize) -> AppState {
    let mut state = initial_state();
    state.gists = (0..n)
        .map(|i| GistFile {
            gist_id: "g1".into(),
            description: "demo".into(),
            filename: format!("f{i}.txt"),
            public: false,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
        })
        .collect();
    state
}

fn list_state_with_matches() -> AppState {
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
            gist_id: "a".into(),
            description: "Zed".into(),
            filename: "settings.json".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
        },
        GistFile {
            gist_id: "b".into(),
            description: "misc".into(),
            filename: "zzz.txt".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
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
fn enter_on_gist_opens_detail() {
    let mut state = state_with_gists();
    state.screen = Screen::Gists;
    let outcome = state.handle_key(KeyCode::Enter);
    assert!(matches!(outcome, KeyOutcome::OpenGistDetail));
}

#[test]
fn detail_focus_and_cursor_default_to_comments_and_zero() {
    let state = initial_state();
    assert_eq!(state.detail_focus, DetailFocus::Comments);
    assert_eq!(state.detail_file_cursor, 0);
}

#[test]
fn detail_tab_toggles_focus() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    assert_eq!(state.detail_focus, DetailFocus::Comments);
    state.handle_key(KeyCode::Tab);
    assert_eq!(state.detail_focus, DetailFocus::Files);
    state.handle_key(KeyCode::Tab);
    assert_eq!(state.detail_focus, DetailFocus::Comments);
}

#[test]
fn detail_files_focus_arrows_move_cursor_and_clamp() {
    let mut state = state_with_gists(); // g1 has 2 files: a.txt, b.txt
    state.screen = Screen::GistDetail;
    state.detail_gist_id = Some("g1".into());
    state.detail_focus = DetailFocus::Files;

    state.handle_key(KeyCode::Up); // already at 0, clamps
    assert_eq!(state.detail_file_cursor, 0);
    state.handle_key(KeyCode::Down);
    assert_eq!(state.detail_file_cursor, 1);
    state.handle_key(KeyCode::Down); // only 2 files, clamps at index 1
    assert_eq!(state.detail_file_cursor, 1);
    state.handle_key(KeyCode::PageUp); // jumps to 0
    assert_eq!(state.detail_file_cursor, 0);
    state.handle_key(KeyCode::PageDown); // +10 clamps to last (1)
    assert_eq!(state.detail_file_cursor, 1);
    // Comment scroll is untouched while files-focused.
    assert_eq!(state.detail_scroll, 0);
}

#[test]
fn detail_comments_focus_arrows_still_scroll_comments() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail_focus = DetailFocus::Comments;
    state.handle_key(KeyCode::Down);
    assert_eq!(state.detail_scroll, 1);
    assert_eq!(state.detail_file_cursor, 0); // cursor untouched
}

#[test]
fn detail_enter_previews_cursor_file_including_tenth() {
    let mut state = state_with_many_files(12);
    state.screen = Screen::GistDetail;
    state.detail_gist_id = Some("g1".into());
    state.detail_focus = DetailFocus::Files;
    state.detail_file_cursor = 9; // the 10th file — unreachable via 1-9
    let outcome = state.handle_key(KeyCode::Enter);
    assert!(matches!(outcome, KeyOutcome::PreviewContent));
    assert_eq!(state.preview_request, Some(("g1".into(), "f9.txt".into())));
    assert_eq!(state.preview_return, Screen::GistDetail);
}

#[test]
fn detail_enter_in_comments_focus_is_noop() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail_gist_id = Some("g1".into());
    state.detail_focus = DetailFocus::Comments;
    let outcome = state.handle_key(KeyCode::Enter);
    assert!(matches!(outcome, KeyOutcome::None));
    assert_eq!(state.preview_request, None);
}

#[test]
fn detail_q_returns_to_gists() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.handle_key(KeyCode::Char('q'));
    assert_eq!(state.screen, Screen::Gists);
}

#[test]
fn detail_scroll_saturates_at_zero() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail_scroll = 0;
    state.handle_key(KeyCode::Up);
    assert_eq!(state.detail_scroll, 0);
    state.handle_key(KeyCode::Down);
    assert_eq!(state.detail_scroll, 1);
}

#[test]
fn detail_c_triggers_compaction_and_records_origin() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    let outcome = state.handle_key(KeyCode::Char('c'));
    assert!(matches!(outcome, KeyOutcome::CompactGist));
    assert_eq!(state.compact_return_screen, Screen::GistDetail);
}

#[test]
fn detail_number_key_requests_file_preview() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail_gist_id = Some("g1".into());
    let outcome = state.handle_key(KeyCode::Char('1'));
    assert!(matches!(outcome, KeyOutcome::PreviewContent));
    assert_eq!(state.preview_request, Some(("g1".into(), "a.txt".into())));
    assert_eq!(state.preview_return, Screen::GistDetail);
}

#[test]
fn detail_number_key_out_of_range_is_ignored() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail_gist_id = Some("g1".into());
    // Only two files exist; pressing 5 must do nothing (no fetch requested).
    let outcome = state.handle_key(KeyCode::Char('5'));
    assert!(matches!(outcome, KeyOutcome::None));
    assert_eq!(state.preview_request, None);
}

#[test]
fn preview_w_toggles_line_wrapping() {
    let mut state = initial_state();
    state.screen = Screen::Preview;
    assert!(!state.preview_wrap);
    state.handle_key(KeyCode::Char('w'));
    assert!(state.preview_wrap);
    state.handle_key(KeyCode::Char('w'));
    assert!(!state.preview_wrap);
}

#[test]
fn detail_x_requests_gist_delete_confirm() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail_gist_id = Some("g1".into());
    let outcome = state.handle_key(KeyCode::Char('X'));
    assert!(matches!(outcome, KeyOutcome::None));
    assert_eq!(state.screen, Screen::Confirm);
    assert!(matches!(
        state.pending_action,
        Some(PendingAction::Delete { ref gist_id, .. }) if gist_id == "g1"
    ));
}

#[test]
fn preview_q_returns_to_launch_screen() {
    let mut state = state_with_gists();
    state.screen = Screen::Preview;
    state.preview_return = Screen::GistDetail;
    state.handle_key(KeyCode::Char('q'));
    assert_eq!(state.screen, Screen::GistDetail);
    // Reset so a later list-launched preview returns to the list.
    assert_eq!(state.preview_return, Screen::List);
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
fn pins_key_clears_lingering_status_for_one_shot_display() {
    let mut state = initial_state();
    state.screen = Screen::Pins;
    state.status = Some("already in sync".into());
    state.handle_key(KeyCode::Up); // any key
    assert_eq!(state.status, None);
}

#[test]
fn preview_key_clears_lingering_status_for_one_shot_display() {
    let mut state = initial_state();
    state.screen = Screen::Preview;
    state.status = Some("fetch failed: boom".into());
    state.handle_key(KeyCode::Down); // any key
    assert_eq!(state.status, None);
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
fn detail_footer_surfaces_status_else_hints() {
    let (msg, colored) = detail_footer(Some("nothing to compact"), DetailFocus::Comments);
    assert_eq!(msg, "nothing to compact");
    assert!(!colored);
    let (hint, colored) = detail_footer(None, DetailFocus::Comments);
    assert!(hint.contains("1-9") && hint.contains("compact"));
    assert!(colored);
}

#[test]
fn detail_footer_is_focus_aware() {
    let (comments, _) = detail_footer(None, DetailFocus::Comments);
    assert!(comments.contains("Tab files") && comments.contains("scroll"));
    let (files, _) = detail_footer(None, DetailFocus::Files);
    assert!(files.contains("Tab comments") && files.contains("preview"));
}

#[test]
fn context_gist_id_uses_detail_id_on_detail_screen() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail_gist_id = Some("g1".into());
    assert_eq!(state.context_gist_id().as_deref(), Some("g1"));
}

#[test]
fn context_gist_id_uses_group_cursor_on_gists_screen() {
    let mut state = state_with_gists();
    state.screen = Screen::Gists;
    state.gists_index = 0;
    assert_eq!(
        state.context_gist_id(),
        state.selected_group().map(|g| g.id)
    );
}

#[test]
fn about_metadata_is_available_for_help() {
    // The footer renders both the repo URL and the version; guard against dropping
    // either from Cargo.toml.
    assert!(!env!("CARGO_PKG_VERSION").is_empty());
    assert!(env!("CARGO_PKG_REPOSITORY").contains("github.com/akunzai/gistui"));
}

#[test]
fn hint_line_colours_keys_by_action_category() {
    let line = hint_line("Tab panes  ·  d download  ·  X delete  ·  Esc/q back");
    let key_fg = |k: &str| {
        line.spans
            .iter()
            .find(|s| s.content == k)
            .unwrap_or_else(|| panic!("key span {k}"))
            .style
            .fg
    };
    assert_eq!(key_fg("Tab"), Some(Color::Cyan)); // navigation
    assert_eq!(key_fg("d"), Some(Color::Green)); // write/sync
    assert_eq!(key_fg("X"), Some(Color::Red)); // destructive
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
fn action_color_matches_whole_words_only() {
    // `pins` opens a view, not the `pin` write action, so it must not read as Green.
    assert_eq!(action_color("pins"), Color::Cyan);
    assert_eq!(action_color("synced ↑ local-newer"), Color::Cyan);
    assert_eq!(action_color("remove file"), Color::Red);
    assert_eq!(action_color("pin"), Color::Green);
}

#[test]
fn hint_line_preserves_every_character() {
    // Sizing relies on wrap_line_count over the raw text, so styling must not add/drop chars.
    let text = "↑↓ move  ·  Enter diff · q back";
    let joined: String = hint_line(text)
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
            gist_id: "abc".into(),
            description: "My Ghostty config".into(),
            filename: "config".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
        },
        score: 1,
        reasons: Vec::new(),
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
        gist_id: "a".into(),
        description: String::new(),
        filename: "settings.json".into(),
        public: false,
        updated_at: "x".into(),
        created_at: "x".into(),
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
    assert!(visible[0].score > 0);
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
        gist_id: "a".into(),
        description: String::new(),
        filename: "f".into(),
        public: false,
        updated_at: "x".into(),
        created_at: "x".into(),
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
            gist_id: "z".into(),
            description: "".into(),
            filename: "zeta.json".into(),
            public: true,
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        },
        GistFile {
            gist_id: "a".into(),
            description: "".into(),
            filename: "alpha.json".into(),
            public: true,
            updated_at: "2026-09-09T00:00:00Z".into(),
            created_at: "2026-09-09T00:00:00Z".into(),
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
            gist_id: "pub".into(),
            description: "p".into(),
            filename: "a.json".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
        },
        GistFile {
            gist_id: "sec".into(),
            description: "s".into(),
            filename: "b.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
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

fn state_with_two_gists() -> AppState {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            gist_id: "a".into(),
            description: "My Ghostty config".into(),
            filename: "config.ghostty".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
        },
        GistFile {
            gist_id: "b".into(),
            description: "SSH config".into(),
            filename: "ssh_config".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
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
    state.pending_action = Some(PendingAction::Download);
    state.screen = Screen::Confirm;
    state.diff_text = "l1\nl2\nl3".into();
    assert_eq!(state.handle_key(KeyCode::Down), KeyOutcome::None);
    assert_eq!(state.diff_scroll, 1);
    state.handle_key(KeyCode::Up);
    assert_eq!(state.diff_scroll, 0);
}

#[test]
fn space_on_selected_gist_returns_preview_content() {
    let mut state = state_with_two_gists();
    assert_eq!(
        state.handle_key(KeyCode::Char(' ')),
        KeyOutcome::PreviewContent
    );
}

#[test]
fn space_without_gist_is_noop() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Char(' ')), KeyOutcome::None);
}

#[test]
fn esc_in_preview_returns_to_list_and_clears() {
    let mut state = initial_state();
    state.screen = Screen::Preview;
    state.diff_text = "raw content".into();
    state.preview_title = "Preview: a / x".into();
    assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
    assert!(state.diff_text.is_empty());
    assert!(state.preview_title.is_empty());
}

#[test]
fn preview_scrolls_with_arrows() {
    let mut state = initial_state();
    state.screen = Screen::Preview;
    state.diff_text = "l1\nl2\nl3".into();
    state.handle_key(KeyCode::Down);
    assert_eq!(state.diff_scroll, 1);
    state.handle_key(KeyCode::Up);
    assert_eq!(state.diff_scroll, 0);
}

#[test]
fn page_keys_jump_by_ten_clamped_to_bounds() {
    let mut state = initial_state();
    state.screen = Screen::Preview;
    // 30 lines → bottom is line 29 (count - 1).
    state.diff_text = (0..30)
        .map(|i| format!("l{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    state.handle_key(KeyCode::PageDown);
    assert_eq!(state.diff_scroll, 10);
    // A second page-down would reach 20; a third clamps at the 29-line bottom, not 30.
    state.handle_key(KeyCode::PageDown);
    state.handle_key(KeyCode::PageDown);
    assert_eq!(state.diff_scroll, 29);
    state.handle_key(KeyCode::PageUp);
    assert_eq!(state.diff_scroll, 19);
}

#[test]
fn page_up_saturates_at_top_in_diff() {
    let mut state = initial_state();
    state.screen = Screen::Diff;
    state.diff_text = "a\nb\nc".into();
    state.diff_scroll = 1;
    state.handle_key(KeyCode::PageUp);
    assert_eq!(state.diff_scroll, 0);
}

#[test]
fn question_opens_help_and_any_key_closes_it() {
    let mut state = initial_state();
    state.handle_key(KeyCode::Char('?'));
    assert_eq!(state.screen, Screen::Help);
    // Arrow keys scroll help instead of closing
    state.handle_key(KeyCode::Down);
    assert_eq!(state.screen, Screen::Help);
    assert_eq!(state.help_scroll, 1);
    state.handle_key(KeyCode::Up);
    assert_eq!(state.screen, Screen::Help);
    assert_eq!(state.help_scroll, 0);
    // Other keys close help
    assert_eq!(state.handle_key(KeyCode::Char('x')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
    // Closing resets scroll
    state.screen = Screen::Help;
    state.help_scroll = 5;
    state.handle_key(KeyCode::Char('q'));
    assert_eq!(state.screen, Screen::List);
    assert_eq!(state.help_scroll, 0);
}

#[test]
fn q_in_help_closes_to_list() {
    let mut state = initial_state();
    state.screen = Screen::Help;
    assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn diff_context_toggle_flips_effective_radius() {
    let mut state = initial_state();
    state.diff_context = 3;
    assert_eq!(state.effective_diff_context(), Some(3));

    // Pressing `c` in the diff view flips to full view and resets the scroll.
    state.screen = Screen::Diff;
    state.diff_scroll = 12;
    let outcome = state.handle_key(KeyCode::Char('c'));
    assert_eq!(outcome, KeyOutcome::PersistDiffContext);
    assert!(state.diff_show_full);
    assert_eq!(state.diff_scroll, 0);
    assert_eq!(state.effective_diff_context(), None);

    // Pressing it again returns to the configured radius.
    state.handle_key(KeyCode::Char('c'));
    assert!(!state.diff_show_full);
    assert_eq!(state.effective_diff_context(), Some(3));
}

#[test]
fn diff_view_applies_vertical_and_horizontal_scroll() {
    let text = "--- a\n+++ b\nabcdef\n more";
    let v = diff_view_highlighted(text, 2, 2, None, false); // skip 2 lines, drop 2 leading chars
    assert_eq!(v.lines.len(), 2);
    assert_eq!(v.lines[0].spans[0].content, "cdef");
}

#[test]
fn diff_view_inline_highlights_changed_words() {
    // A single-line modification: "hello world" → "hello planet"
    let text = "--- a\n+++ b\n-hello world\n+hello planet\n";
    let v = diff_view_highlighted(text, 2, 0, None, false); // skip header lines
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
    let v = diff_view_highlighted(text, 0, 0, Some("rs"), true);
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
    let v = diff_view_highlighted(text, 0, 0, Some("rs"), false);
    assert!(v.lines[2].spans.iter().all(|s| s.style.fg.is_none()));
}

#[test]
fn diff_view_skips_tabbed_context_lines() {
    // A tab in the context line keeps it plain so indentation stays aligned with -/+ lines.
    let text = "--- a\n+++ b\n \tlet x = 1;\n";
    let v = diff_view_highlighted(text, 0, 0, Some("rs"), true);
    assert!(v.lines[2].spans.iter().all(|s| s.style.fg.is_none()));
}

#[test]
fn header_line_tints_local_yellow_and_gist_blue() {
    let local = header_line("--- local: notes.txt (2026-06-10 14:25 UTC)", 0);
    let kw = local.spans.iter().find(|s| s.content == "local").unwrap();
    assert_eq!(kw.style.fg, Some(Color::Yellow));

    let gist = header_line("+++ gist abc123 / notes.txt (2026-06-10 13:10 UTC)", 0);
    let kw = gist.spans.iter().find(|s| s.content == "gist").unwrap();
    assert_eq!(kw.style.fg, Some(Color::Blue));
}

#[test]
fn preview_diff_text_flips_with_focus() {
    // Download orientation (gist pane focused): old = local, new = gist.
    let dl = preview_diff_text(false, "local: a", "old\n", "gist b", "new\n");
    assert!(dl.starts_with("--- local: a\n+++ gist b\n"));

    // Upload orientation (local pane focused): old = gist, new = local.
    let ul = preview_diff_text(true, "local: a", "old\n", "gist b", "new\n");
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
fn commands_hint_is_focus_aware() {
    let local = commands_hint(FocusPane::Local);
    assert!(local.contains("e edit"));
    assert!(local.contains("n create"));
    assert!(!local.contains("d download"));

    let gist = commands_hint(FocusPane::Gist);
    assert!(gist.contains("d download"));
    assert!(gist.contains("g gists"));
    assert!(!gist.contains("e edit"));

    // Always-available keys appear in both.
    for hint in [local, gist] {
        assert!(hint.contains("? help"));
        assert!(hint.contains("Esc/q quit"));
    }
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
fn confirm_prompt_covers_each_pending_action() {
    let mut state = initial_state();

    state.download_target = PathBuf::from("notes.txt");
    state.pending_action = Some(PendingAction::Download);
    assert_eq!(confirm_prompt(&state), "Overwrite notes.txt? (y/n)");
    assert_eq!(confirm_modal_style(&state), ("Overwrite", Color::Red));

    state.pending_action = Some(PendingAction::Delete {
        gist_id: "abc".into(),
        label: "my config".into(),
    });
    assert_eq!(
        confirm_prompt(&state),
        "Permanently delete \"my config\" (abc)? (y/n)"
    );
    assert_eq!(confirm_modal_style(&state), ("Delete", Color::Red));

    state.pending_action = Some(PendingAction::Upload {
        gist_id: "g1".into(),
        filename: "main.rs".into(),
        local_path: PathBuf::from("main.rs"),
    });
    assert!(confirm_prompt(&state).starts_with("Upload main.rs to gist g1?"));
    assert_eq!(confirm_modal_style(&state), ("Upload", Color::Yellow));

    state.pending_action = Some(PendingAction::CompactGist {
        gist_id: "abc".into(),
        label: "my config".into(),
        count: 4,
    });
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
    state.pending_action = Some(PendingAction::Create {
        local_path: PathBuf::from("notes.txt"),
    });
    state.editing_description = true;
    state.description_input = "hello".into();
    assert_eq!(
        confirm_prompt(&state),
        "Description (optional): hello_   ·  Enter next  ·  Esc cancel"
    );
    assert_eq!(confirm_modal_style(&state), ("Description", Color::Cyan));
}

#[test]
fn row_mark_pinned_beats_same_name() {
    assert_eq!(
        row_mark(&[MatchReason::Pinned, MatchReason::ExactFilename]),
        RowMark::Pinned
    );
}
#[test]
fn row_mark_same_name_when_exact_filename_only() {
    assert_eq!(row_mark(&[MatchReason::ExactFilename]), RowMark::SameName);
}
#[test]
fn row_mark_none_for_weak_matches() {
    assert_eq!(row_mark(&[MatchReason::PathSegment]), RowMark::None);
    assert_eq!(row_mark(&[MatchReason::Recent]), RowMark::None);
    assert_eq!(row_mark(&[]), RowMark::None);
}

#[test]
fn gist_row_label_falls_back_to_filename_when_description_empty() {
    let g = RankedGistFile {
        file: GistFile {
            gist_id: "abc".into(),
            description: "  ".into(),
            filename: "config".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
        },
        score: 0,
        reasons: Vec::new(),
    };
    assert_eq!(gist_row_label(&g, GistView::Description), "config");
}

#[test]
fn left_right_scrolls_focused_gist_pane() {
    let mut state = initial_state();
    state.gists = vec![GistFile {
        gist_id: "a".into(),
        description: "a fairly long description for scrolling".into(),
        filename: "f.json".into(),
        public: false,
        updated_at: "x".into(),
        created_at: "x".into(),
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
fn gist_hscroll_caps_at_longest_row() {
    let mut state = initial_state();
    state.gists = vec![GistFile {
        gist_id: "a".into(),
        description: "tiny".into(),
        filename: "f".into(),
        public: false,
        updated_at: "x".into(),
        created_at: "x".into(),
    }];
    state.focus = FocusPane::Gist;
    let row = gist_row_label(&state.ranked_gists()[0], state.gist_view);
    let max = (row.chars().count() - 1) as u16;
    for _ in 0..200 {
        state.handle_key(KeyCode::Right);
    }
    assert_eq!(state.gist_hscroll, max);
}

#[test]
fn moving_gist_selection_resets_hscroll() {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            gist_id: "a".into(),
            description: "first long description here".into(),
            filename: "a.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
        },
        GistFile {
            gist_id: "b".into(),
            description: "second long description here".into(),
            filename: "b.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
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
            gist_id: "a".into(),
            description: "first".into(),
            filename: "alpha.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
        },
        GistFile {
            gist_id: "b".into(),
            description: "second".into(),
            filename: "beta.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
        },
    ];
    let ranked = state.ranked_gists();
    assert_eq!(ranked.len(), 2);
    // Order preserved (unranked) and no scoring applied.
    assert_eq!(ranked[0].file.filename, "alpha.json");
    assert_eq!(ranked[0].score, 0);
    assert!(ranked[0].reasons.is_empty());
}

#[test]
fn enter_with_no_local_but_gist_selected_returns_preview() {
    let mut state = initial_state();
    state.gists = vec![GistFile {
        gist_id: "a".into(),
        description: "first".into(),
        filename: "alpha.json".into(),
        public: false,
        updated_at: "x".into(),
        created_at: "x".into(),
    }];
    state.focus = FocusPane::Gist;
    assert!(state.locals.is_empty());
    assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::PreviewDiff);
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
            gist_id: "a".into(),
            description: "settings".into(),
            filename: "settings.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
        },
        GistFile {
            gist_id: "b".into(),
            description: "status".into(),
            filename: "statusline.sh".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
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
        gist_id: "a".into(),
        description: "settings".into(),
        filename: "settings.json".into(),
        public: false,
        updated_at: "x".into(),
        created_at: "x".into(),
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
    assert_eq!(state.screen, Screen::Diff);
    assert!(state.diff_previewed);
    assert_eq!(state.preview_remote, "remote body");
    assert_eq!(state.preview_local, PathBuf::from("/tmp/x"));
    assert_eq!(state.download_target, PathBuf::from("/tmp/cwd/x"));
    assert_eq!(state.diff_scroll, 0);
}

#[test]
fn back_to_list_clears_preview() {
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/x"),
        PathBuf::from("/tmp/x"),
    );
    state.back_to_list();
    assert_eq!(state.screen, Screen::List);
    assert!(!state.diff_previewed);
    assert!(state.diff_text.is_empty());
    assert!(state.preview_remote.is_empty());
    assert_eq!(state.preview_local, PathBuf::new());
    assert_eq!(state.download_target, PathBuf::new());
}

#[test]
fn enter_in_gist_focus_with_selection_returns_preview() {
    let mut state = state_with_selection();
    assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::PreviewDiff);
}

#[test]
fn enter_in_local_focus_previews_top_gist() {
    let mut state = state_with_selection();
    state.focus = FocusPane::Local;
    assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::PreviewDiff);
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
    assert_eq!(
        state.handle_key(KeyCode::Char('d')),
        KeyOutcome::DownloadGist
    );
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
    assert_eq!(state.diff_scroll, 0);
    state.handle_key(KeyCode::Up); // stays at 0
    assert_eq!(state.diff_scroll, 0);
    state.handle_key(KeyCode::Down);
    assert_eq!(state.diff_scroll, 1);
    state.handle_key(KeyCode::Down);
    assert_eq!(state.diff_scroll, 2);
    state.handle_key(KeyCode::Down); // capped at lines-1 = 2
    assert_eq!(state.diff_scroll, 2);
    state.handle_key(KeyCode::Up);
    assert_eq!(state.diff_scroll, 1);
}

#[test]
fn identical_diff_disables_download_and_upload() {
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/x"),
        PathBuf::from("/tmp/x"),
    );
    state.diff_identical = true;
    assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
    assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::None);
    // Scrolling and leaving still work.
    assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
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
    assert_eq!(state.diff_hscroll, 0);
    state.handle_key(KeyCode::Left); // stays at 0
    assert_eq!(state.diff_hscroll, 0);
    for _ in 0..10 {
        state.handle_key(KeyCode::Right);
    }
    assert_eq!(state.diff_hscroll, 3);
    state.handle_key(KeyCode::Left);
    assert_eq!(state.diff_hscroll, 2);
}

#[test]
fn esc_in_diff_returns_to_list() {
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/x"),
        PathBuf::from("/tmp/x"),
    );
    assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
    assert!(!state.diff_previewed);
}

#[test]
fn d_in_diff_downloads_when_file_absent() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist.json");
    let mut state = initial_state();
    state.enter_diff("d".into(), "r".into(), PathBuf::from("/tmp/local"), missing);
    assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::Download);
}

#[test]
fn d_in_diff_confirms_when_file_exists() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("exists.json");
    std::fs::write(&existing, "old").unwrap();
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/local"),
        existing,
    );
    assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::Confirm);
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
    state.pending_action = Some(PendingAction::Download);
    state.screen = Screen::Confirm;
    assert_eq!(state.handle_key(KeyCode::Char('y')), KeyOutcome::Download);
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
    state.pending_action = Some(PendingAction::Download);
    state.screen = Screen::Confirm;
    assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::Diff);
}

#[test]
fn q_in_diff_returns_to_list() {
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/x"),
        PathBuf::from("/tmp/x"),
    );
    assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn q_in_list_quits_on_second_press() {
    let mut state = initial_state();
    // First press only arms the quit (and surfaces a hint); it must not exit.
    assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
    assert!(state.quit_armed);
    assert!(state.status.is_some());
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
    state.pending_action = Some(PendingAction::Download);
    state.screen = Screen::Confirm;
    assert_eq!(state.handle_key(KeyCode::Char('q')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::Diff);
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
    state.pending_action = Some(PendingAction::Download);
    state.screen = Screen::Confirm;
    assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
    assert_eq!(state.screen, Screen::Diff);
}

#[test]
fn d_in_diff_on_existing_sets_download_pending() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("exists.json");
    std::fs::write(&existing, "old").unwrap();
    let mut state = initial_state();
    state.enter_diff(
        "d".into(),
        "r".into(),
        PathBuf::from("/tmp/local"),
        existing,
    );
    assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::Confirm);
    assert_eq!(state.pending_action, Some(PendingAction::Download));
}

#[test]
fn p_pins_unpinned_pair_then_unpins() {
    let mut state = state_with_selection();
    assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::Pin);
    state.pinned = vec![PinnedMapping {
        local_path: PathBuf::from("/tmp/settings.json"),
        gist_id: "a".into(),
        gist_filename: "settings.json".into(),
        direction: None,
        last_seen_hash: None,
    }];
    assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::Unpin);
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
        gist_id: "a".into(),
        description: "x".into(),
        filename: "settings.json".into(),
        public: false,
        updated_at: "x".into(),
        created_at: "x".into(),
    }];
    state.focus = FocusPane::Gist;
    assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::UploadAdd);
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
        gist_id: "a".into(),
        description: "x".into(),
        filename: "settings.json".into(),
        public: false,
        updated_at: "x".into(),
        created_at: "x".into(),
    }];
    state.focus = FocusPane::Gist;
    assert_eq!(
        state.handle_key(KeyCode::Char('u')),
        KeyOutcome::UploadPreview
    );
}

#[test]
fn u_without_selection_is_noop() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::None);
}

#[test]
fn o_in_gist_view_opens_browser() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists;
    assert_eq!(
        state.handle_key(KeyCode::Char('o')),
        KeyOutcome::OpenBrowser
    );
}

#[test]
fn o_on_main_list_is_noop_now_that_browser_moved_to_gist_view() {
    let mut state = state_with_two_gists();
    assert_eq!(state.handle_key(KeyCode::Char('o')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn y_copies_gist_url_on_list_gists_and_detail() {
    let mut list = state_with_two_gists();
    assert_eq!(list.handle_key(KeyCode::Char('y')), KeyOutcome::CopyGistUrl);

    let mut gists = state_with_two_gists();
    gists.screen = Screen::Gists;
    assert_eq!(
        gists.handle_key(KeyCode::Char('y')),
        KeyOutcome::CopyGistUrl
    );

    let mut detail = state_with_gists();
    detail.screen = Screen::GistDetail;
    detail.detail_gist_id = Some("g1".into());
    assert_eq!(
        detail.handle_key(KeyCode::Char('y')),
        KeyOutcome::CopyGistUrl
    );
}

#[test]
fn preview_y_copies_url_and_capital_y_copies_content() {
    let mut state = state_with_gists();
    state.screen = Screen::Preview;
    assert_eq!(
        state.handle_key(KeyCode::Char('y')),
        KeyOutcome::CopyGistUrl
    );
    assert_eq!(
        state.handle_key(KeyCode::Char('Y')),
        KeyOutcome::CopyPreviewContent
    );
}

#[test]
fn c_in_gist_view_requests_compaction() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists;
    assert_eq!(
        state.handle_key(KeyCode::Char('c')),
        KeyOutcome::CompactGist
    );
    // `c` on the main list is not a compaction trigger.
    let mut list = state_with_two_gists();
    assert_eq!(list.handle_key(KeyCode::Char('c')), KeyOutcome::None);
}

#[test]
fn compact_confirm_y_executes_and_n_returns_to_gist_manager() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Confirm;
    state.pending_action = Some(PendingAction::CompactGist {
        gist_id: "a".into(),
        label: "My Ghostty config".into(),
        count: 3,
    });
    assert_eq!(
        state.handle_key(KeyCode::Char('y')),
        KeyOutcome::ExecuteCompactGist
    );

    // Cancelling drops the pending action and lands back in the gist manager.
    state.handle_key(KeyCode::Char('n'));
    assert_eq!(state.screen, Screen::Gists);
    assert!(state.pending_action.is_none());
}

#[test]
fn e_edits_local_with_file_selected() {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/config"),
        pinned: false,
        modified: None,
    }];
    assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::EditLocal);
}

#[test]
fn e_without_local_is_noop() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::None);
}

#[test]
fn u_in_diff_screen_returns_upload_intent() {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("/tmp/config"),
        pinned: false,
        modified: None,
    }];
    state.gists = vec![GistFile {
        gist_id: "a".into(),
        description: "x".into(),
        filename: "settings.json".into(),
        public: false,
        updated_at: "x".into(),
        created_at: "x".into(),
    }];
    state.screen = Screen::Diff;
    // The gist has no "config" file -> case B -> add directly.
    assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::UploadAdd);
}

#[test]
fn confirm_upload_y_returns_upload() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: "a".into(),
        filename: "settings.json".into(),
        local_path: PathBuf::from("/tmp/settings.json"),
    });
    state.screen = Screen::Confirm;
    assert_eq!(state.handle_key(KeyCode::Char('y')), KeyOutcome::Upload);
}

#[test]
fn confirm_upload_e_returns_edit_upload() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: "a".into(),
        filename: "settings.json".into(),
        local_path: PathBuf::from("/tmp/settings.json"),
    });
    state.screen = Screen::Confirm;
    assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::EditUpload);
}

#[test]
fn confirm_upload_json_toggles() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: "a".into(),
        filename: "settings.json".into(),
        local_path: PathBuf::from("/tmp/settings.json"),
    });
    state.screen = Screen::Confirm;
    assert!(!state.upload_json_pretty);
    assert!(!state.upload_json_sort);

    // Toggle pretty
    assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::None);
    assert!(state.upload_json_pretty);

    // Toggle sort
    assert_eq!(state.handle_key(KeyCode::Char('s')), KeyOutcome::None);
    assert!(state.upload_json_sort);

    // Toggle pretty off
    assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::None);
    assert!(!state.upload_json_pretty);
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
    assert_eq!(state.screen, Screen::Confirm);
    assert_eq!(
        state.pending_action,
        Some(PendingAction::Create {
            local_path: PathBuf::from("/tmp/config.toml")
        })
    );
}

#[test]
fn n_without_local_is_noop() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn x_without_gist_is_noop() {
    let mut state = initial_state();
    state.focus = FocusPane::Gist;
    assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn x_removes_selected_file_from_a_multifile_gist() {
    let mut state = initial_state();
    state.focus = FocusPane::Gist;
    state.gists = vec![
        GistFile {
            gist_id: "abc123".into(),
            description: "my notes".into(),
            filename: "a.md".into(),
            public: false,
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        },
        GistFile {
            gist_id: "abc123".into(),
            description: "my notes".into(),
            filename: "b.md".into(),
            public: false,
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        },
    ];
    // X stages a single-file removal (not a whole-gist delete) and asks to confirm.
    assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::Confirm);
    assert_eq!(
        state.pending_action,
        Some(PendingAction::RemoveFile {
            gist_id: "abc123".into(),
            filename: "a.md".into(),
            label: "my notes".into(),
        })
    );
}

#[test]
fn x_on_a_gists_only_file_is_blocked() {
    let mut state = initial_state();
    state.focus = FocusPane::Gist;
    state.gists = vec![GistFile {
        gist_id: "abc123".into(),
        description: String::new(),
        filename: "notes.md".into(),
        public: false,
        updated_at: "2026-01-01T00:00:00Z".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    }];
    // Removing the only file would leave a fileless gist, which GitHub forbids.
    assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
    assert!(state.pending_action.is_none());
    assert!(state.status.as_deref().unwrap().contains("only file"));
}

#[test]
fn remove_file_confirm_y_returns_execute_remove_file() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::RemoveFile {
        gist_id: "abc123".into(),
        filename: "a.md".into(),
        label: "my notes".into(),
    });
    state.screen = Screen::Confirm;
    assert_eq!(
        state.handle_key(KeyCode::Char('y')),
        KeyOutcome::ExecuteRemoveFile
    );
}

#[test]
fn delete_confirm_y_returns_execute_delete() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Delete {
        gist_id: "abc123".into(),
        label: "my notes".into(),
    });
    state.screen = Screen::Confirm;
    assert_eq!(
        state.handle_key(KeyCode::Char('y')),
        KeyOutcome::ExecuteDelete
    );
}

#[test]
fn delete_confirm_n_returns_to_list() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Delete {
        gist_id: "abc123".into(),
        label: "my notes".into(),
    });
    state.screen = Screen::Confirm;
    assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
    assert!(state.pending_action.is_none());
}

#[test]
fn g_opens_gist_view_landing_on_the_selected_files_gist() {
    let mut state = state_with_two_gists();
    // Select the second gist's row in the main (file) list, then jump to the
    // gist-level view; it should land on that same gist.
    state.gist_index = 1;
    assert_eq!(state.handle_key(KeyCode::Char('g')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::Gists);
    assert_eq!(state.gists_index, 1);
    assert_eq!(state.selected_group().unwrap().id, "b");
}

#[test]
fn g_with_no_gists_is_blocked() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Char('g')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn gist_view_e_edits_description_with_prefill_and_enter_applies() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists;
    state.gists_index = 0;
    state.handle_key(KeyCode::Char('e'));
    assert!(state.editing_description);
    // Prefilled with the current description.
    assert_eq!(state.description_input, "My Ghostty config");
    state.handle_key(KeyCode::Char('!'));
    assert_eq!(state.description_input, "My Ghostty config!");
    assert_eq!(
        state.handle_key(KeyCode::Enter),
        KeyOutcome::ApplyDescription
    );
}

#[test]
fn gist_view_esc_cancels_description_edit() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists;
    state.handle_key(KeyCode::Char('e'));
    assert!(state.editing_description);
    state.handle_key(KeyCode::Esc);
    assert!(!state.editing_description);
    assert!(state.description_input.is_empty());
}

#[test]
fn gist_view_x_stages_whole_gist_delete() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists;
    state.gists_index = 1;
    assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::Confirm);
    assert_eq!(
        state.pending_action,
        Some(PendingAction::Delete {
            gist_id: "b".into(),
            label: "SSH config".into(),
        })
    );
}

#[test]
fn gist_view_q_returns_to_list() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists;
    state.handle_key(KeyCode::Char('q'));
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn gist_view_v_cycles_visibility_filter() {
    // state_with_two_gists: gist "a" is public, gist "b" is secret.
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists;
    assert_eq!(state.visible_gist_groups().len(), 2);

    state.handle_key(KeyCode::Char('v')); // -> public
    let vis = state.visible_gist_groups();
    assert_eq!(vis.len(), 1);
    assert_eq!(vis[0].id, "a");

    state.handle_key(KeyCode::Char('v')); // -> secret
    let vis = state.visible_gist_groups();
    assert_eq!(vis.len(), 1);
    assert_eq!(vis[0].id, "b");

    state.handle_key(KeyCode::Char('v')); // -> all
    assert_eq!(state.visible_gist_groups().len(), 2);
}

#[test]
fn gist_view_filter_narrows_then_esc_clears() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists;
    state.handle_key(KeyCode::Char('/'));
    assert!(state.gists_filtering);
    for c in "ssh".chars() {
        state.handle_key(KeyCode::Char(c));
    }
    let vis = state.visible_gist_groups();
    assert_eq!(vis.len(), 1);
    assert_eq!(vis[0].id, "b"); // "SSH config"

    state.handle_key(KeyCode::Esc);
    assert!(!state.gists_filtering);
    assert!(state.gists_filter_query.is_empty());
    assert_eq!(state.visible_gist_groups().len(), 2);
}

#[test]
fn gist_view_s_cycles_sort_updated_then_created() {
    let mut state = initial_state();
    state.screen = Screen::Gists;
    state.gists = vec![
        GistFile {
            gist_id: "old-upd".into(),
            description: "x".into(),
            filename: "f".into(),
            public: false,
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-12-01T00:00:00Z".into(),
        },
        GistFile {
            gist_id: "new-upd".into(),
            description: "y".into(),
            filename: "g".into(),
            public: false,
            updated_at: "2026-06-01T00:00:00Z".into(),
            created_at: "2026-02-01T00:00:00Z".into(),
        },
    ];
    // Default: sort by updated (newest first).
    assert_eq!(state.gists_sort, GistGroupSort::Updated);
    assert_eq!(state.visible_gist_groups()[0].id, "new-upd");
    // s -> sort by created (newest created first).
    state.handle_key(KeyCode::Char('s'));
    assert_eq!(state.gists_sort, GistGroupSort::Created);
    assert_eq!(state.visible_gist_groups()[0].id, "old-upd");
}

#[test]
fn gist_view_left_right_scrolls_horizontally() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists;
    assert_eq!(state.gists_hscroll, 0);
    state.handle_key(KeyCode::Right);
    assert_eq!(state.gists_hscroll, 1);
    state.handle_key(KeyCode::Left);
    assert_eq!(state.gists_hscroll, 0);
    // Left at the origin saturates at 0.
    state.handle_key(KeyCode::Left);
    assert_eq!(state.gists_hscroll, 0);
}

#[test]
fn create_confirm_s_and_p_choose_visibility() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Create {
        local_path: PathBuf::from("/tmp/config.toml"),
    });
    state.screen = Screen::Confirm;
    assert_eq!(
        state.handle_key(KeyCode::Char('s')),
        KeyOutcome::Create(false)
    );

    state.pending_action = Some(PendingAction::Create {
        local_path: PathBuf::from("/tmp/config.toml"),
    });
    state.screen = Screen::Confirm;
    assert_eq!(
        state.handle_key(KeyCode::Char('p')),
        KeyOutcome::Create(true)
    );
}

#[test]
fn create_confirm_esc_cancels() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Create {
        local_path: PathBuf::from("/tmp/config.toml"),
    });
    state.screen = Screen::Confirm;
    assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
    assert_eq!(state.pending_action, None);
}

fn state_ready_to_create() -> AppState {
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
    assert_eq!(state.screen, Screen::Confirm);
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

#[test]
fn create_esc_while_editing_description_cancels() {
    let mut state = state_ready_to_create();
    state.handle_key(KeyCode::Char('n'));
    state.handle_key(KeyCode::Char('x'));
    assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
    assert_eq!(state.pending_action, None);
    assert!(!state.editing_description);
    assert!(state.description_input.is_empty());
}

#[test]
fn pins_screen_sync_keys_emit_outcomes() {
    let mut state = initial_state();
    state.screen = Screen::Pins;
    state.pinned = vec![PinnedMapping {
        local_path: PathBuf::from("/tmp/a.txt"),
        gist_id: "g1".into(),
        gist_filename: "a.txt".into(),
        direction: None,
        last_seen_hash: None,
    }];
    assert_eq!(
        state.handle_key(KeyCode::Char('s')),
        KeyOutcome::SyncPinAuto
    );
    assert_eq!(
        state.handle_key(KeyCode::Char('u')),
        KeyOutcome::SyncPinPush
    );
    assert_eq!(
        state.handle_key(KeyCode::Char('d')),
        KeyOutcome::SyncPinPull
    );
    assert_eq!(state.handle_key(KeyCode::Char('x')), KeyOutcome::UnpinAtPin);
}

#[test]
fn pins_screen_enter_emits_preview_pin_diff() {
    let mut state = initial_state();
    state.screen = Screen::Pins;
    state.pinned = vec![PinnedMapping {
        local_path: PathBuf::from("/tmp/a.txt"),
        gist_id: "g1".into(),
        gist_filename: "a.txt".into(),
        direction: None,
        last_seen_hash: None,
    }];
    assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::PreviewPinDiff);
}

#[test]
fn list_screen_capital_s_syncs_selected_pair() {
    let mut state = initial_state();
    state.locals = vec![LocalCandidate {
        path: PathBuf::from("a.txt"),
        pinned: true,
        modified: None,
    }];
    state.gists = vec![GistFile {
        gist_id: "g1".into(),
        description: String::new(),
        filename: "a.txt".into(),
        public: false,
        updated_at: String::new(),
        created_at: String::new(),
    }];
    assert_eq!(
        state.handle_key(KeyCode::Char('S')),
        KeyOutcome::SyncSelectedPair
    );
}

#[test]
fn fetched_comments_apply_when_viewing_same_gist() {
    let mut state = initial_state();
    state.detail_gist_id = Some("g1".into());
    state.apply_fetched_comments(
        "g1",
        Ok(vec![GistComment {
            author: "alice".into(),
            created_at: "2026-06-10T00:00:00Z".into(),
            body: "hi".into(),
        }]),
    );
    assert_eq!(state.detail_comments.as_ref().unwrap().len(), 1);
    assert!(state.detail_comments_error.is_none());
}

#[test]
fn fetched_comments_ignored_when_gist_changed() {
    let mut state = initial_state();
    state.detail_gist_id = Some("g2".into());
    state.detail_comments = None;
    state.apply_fetched_comments(
        "g1",
        Ok(vec![GistComment {
            author: "alice".into(),
            created_at: "2026-06-10T00:00:00Z".into(),
            body: "stale".into(),
        }]),
    );
    // Result was for g1 but user is on g2 → ignored.
    assert!(state.detail_comments.is_none());
}

#[test]
fn fetched_comments_error_sets_empty_list_and_message() {
    let mut state = initial_state();
    state.detail_gist_id = Some("g1".into());
    state.apply_fetched_comments("g1", Err("boom".into()));
    assert_eq!(state.detail_comments.as_ref().unwrap().len(), 0);
    assert_eq!(state.detail_comments_error.as_deref(), Some("boom"));
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
    };
    let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
    // Sorting by updated shows the updated age (1 day ago); sorting by created shows the
    // created age (10 days ago → "1w"), so the 🕒 column matches the ordering key.
    let updated = gist_group_row_label(&group, now, GistGroupSort::Updated);
    let created = gist_group_row_label(&group, now, GistGroupSort::Created);
    assert!(updated.ends_with("🕒 1d"), "{updated}");
    assert!(created.ends_with("🕒 1w"), "{created}");
}
