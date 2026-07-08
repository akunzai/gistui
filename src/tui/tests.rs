use super::*;
use crossterm::event::KeyCode;
use ratatui::layout::Rect;
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        },
        GistFile {
            gist_id: "g1".into(),
            description: "demo".into(),
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
    state.gist_manager.index = 0;
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        })
        .collect();
    state
}

fn state_with_local_paths(paths: &[&str]) -> AppState {
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        },
        GistFile {
            gist_id: "b".into(),
            description: "misc".into(),
            filename: "zzz.txt".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
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
fn detail_focus_and_cursor_default_to_files_and_zero() {
    let state = initial_state();
    assert_eq!(state.detail.focus, DetailFocus::Files);
    assert_eq!(state.detail.file_cursor, 0);
}

#[test]
fn detail_tab_toggles_focus() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    assert_eq!(state.detail.focus, DetailFocus::Files);
    let outcome = state.handle_key(KeyCode::Tab);
    assert!(matches!(outcome, KeyOutcome::FetchComments));
    assert_eq!(state.detail.focus, DetailFocus::Comments);
    let outcome = state.handle_key(KeyCode::Tab);
    assert!(matches!(outcome, KeyOutcome::None));
    assert_eq!(state.detail.focus, DetailFocus::Files);
}

#[test]
fn detail_tab_to_comments_skips_fetch_when_already_loaded() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    state.detail.comments = Some(Vec::new());
    let outcome = state.handle_key(KeyCode::Tab);
    assert!(matches!(outcome, KeyOutcome::None));
    assert_eq!(state.detail.focus, DetailFocus::Comments);
}

#[test]
fn detail_files_focus_arrows_move_cursor_and_clamp() {
    let mut state = state_with_gists(); // g1 has 2 files: a.txt, b.txt
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    state.detail.focus = DetailFocus::Files;

    state.handle_key(KeyCode::Up); // already at 0, clamps
    assert_eq!(state.detail.file_cursor, 0);
    state.handle_key(KeyCode::Down);
    assert_eq!(state.detail.file_cursor, 1);
    state.handle_key(KeyCode::Down); // only 2 files, clamps at index 1
    assert_eq!(state.detail.file_cursor, 1);
    state.handle_key(KeyCode::PageUp); // jumps to 0
    assert_eq!(state.detail.file_cursor, 0);
    state.handle_key(KeyCode::PageDown); // +10 clamps to last (1)
    assert_eq!(state.detail.file_cursor, 1);
    // Comment scroll is untouched while files-focused.
    assert_eq!(state.detail.scroll, 0);
}

#[test]
fn detail_comments_focus_arrows_still_scroll_comments() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.focus = DetailFocus::Comments;
    state.handle_key(KeyCode::Down);
    assert_eq!(state.detail.scroll, 1);
    assert_eq!(state.detail.file_cursor, 0); // cursor untouched
}

#[test]
fn detail_enter_previews_cursor_file_including_tenth() {
    let mut state = state_with_many_files(12);
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    state.detail.focus = DetailFocus::Files;
    state.detail.file_cursor = 9; // the 10th file — unreachable via 1-9
    let outcome = state.handle_key(KeyCode::Enter);
    assert!(matches!(outcome, KeyOutcome::PreviewContent));
    assert_eq!(state.preview_request, Some(("g1".into(), "f9.txt".into())));
    assert_eq!(state.preview_return, Screen::GistDetail);
}

#[test]
fn detail_enter_in_comments_focus_is_noop() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    state.detail.focus = DetailFocus::Comments;
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
    state.detail.focus = DetailFocus::Comments;
    state.detail.scroll = 0;
    state.handle_key(KeyCode::Up);
    assert_eq!(state.detail.scroll, 0);
    state.handle_key(KeyCode::Down);
    assert_eq!(state.detail.scroll, 1);
}

#[test]
fn detail_c_triggers_compaction_and_records_origin() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    let outcome = state.handle_key(KeyCode::Char('c'));
    assert!(matches!(outcome, KeyOutcome::CompactGist));
    assert_eq!(state.detail.compact_return_screen, Screen::GistDetail);
}

#[test]
fn detail_number_key_requests_file_preview() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    let outcome = state.handle_key(KeyCode::Char('1'));
    assert!(matches!(outcome, KeyOutcome::PreviewContent));
    assert_eq!(state.preview_request, Some(("g1".into(), "a.txt".into())));
    assert_eq!(state.preview_return, Screen::GistDetail);
}

#[test]
fn detail_number_key_out_of_range_is_ignored() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
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
fn diff_w_toggles_wrap_and_resets_hscroll() {
    let mut state = initial_state();
    state.screen = Screen::Diff;
    state.diff_hscroll = 5;
    assert!(!state.diff_wrap);
    state.handle_key(KeyCode::Char('w'));
    assert!(state.diff_wrap);
    // Horizontal offset is meaningless once wrapping, so it resets.
    assert_eq!(state.diff_hscroll, 0);
    state.handle_key(KeyCode::Char('w'));
    assert!(!state.diff_wrap);
}

#[test]
fn diff_footer_reflects_wrap_toggle() {
    let mut state = initial_state();
    state.screen = Screen::Diff;
    assert!(diff_footer(&state).contains("w wrap [off]"));
    state.diff_wrap = true;
    let footer = diff_footer(&state);
    assert!(footer.contains("w wrap [on]"));
    // The horizontal-scroll arrows are dropped from the hint when wrapping.
    assert!(!footer.contains("←→"));
}

#[test]
fn detail_x_requests_gist_delete_confirm() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
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
fn star_key_in_detail_returns_toggle_intent() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    assert_eq!(
        state.handle_key(KeyCode::Char('*')),
        KeyOutcome::ToggleGistStar
    );
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
fn context_gist_id_uses_detail_id_on_detail_screen() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    assert_eq!(state.context_gist_id().as_deref(), Some("g1"));
}

#[test]
fn context_gist_id_uses_group_cursor_on_gists_screen() {
    let mut state = state_with_gists();
    state.screen = Screen::Gists;
    state.gist_manager.index = 0;
    assert_eq!(
        state.context_gist_id(),
        state.selected_group().map(|g| g.id)
    );
}

#[test]
fn about_metadata_is_available_for_help() {
    // The footer renders the repo URL; guard against dropping it from Cargo.toml.
    assert!(env!("CARGO_PKG_REPOSITORY").contains("github.com/akunzai/gistui"));
}

#[test]
fn hint_line_colours_keys_by_action_category() {
    let line = hint_line(
        "Tab panes  ·  d download  ·  X delete  ·  Esc/q back",
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
    assert_eq!(action_color("pins", &Theme::DARK), Color::Cyan);
    assert_eq!(
        action_color("synced ↑ local-newer", &Theme::DARK),
        Color::Cyan
    );
    assert_eq!(action_color("remove file", &Theme::DARK), Color::Red);
    assert_eq!(action_color("pin", &Theme::DARK), Color::Green);
}

#[test]
fn hint_line_preserves_every_character() {
    // Sizing relies on wrap_line_count over the raw text, so styling must not add/drop chars.
    let text = "↑↓ move  ·  Enter diff · q back";
    let joined: String = hint_line(text, &Theme::DARK)
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
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
        gist_id: "a".into(),
        description: String::new(),
        filename: "settings.json".into(),
        public: false,
        updated_at: "x".into(),
        created_at: "x".into(),
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
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
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        },
        GistFile {
            gist_id: "a".into(),
            description: "".into(),
            filename: "alpha.json".into(),
            public: true,
            updated_at: "2026-09-09T00:00:00Z".into(),
            created_at: "2026-09-09T00:00:00Z".into(),
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        },
        GistFile {
            gist_id: "sec".into(),
            description: "s".into(),
            filename: "b.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        },
        GistFile {
            gist_id: "b".into(),
            description: "SSH config".into(),
            filename: "ssh_config".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
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
fn question_opens_contextual_help_from_list() {
    let mut state = initial_state();
    state.handle_key(KeyCode::Char('?'));
    assert_eq!(state.screen, Screen::Help);
    assert_eq!(state.help.topic, HelpTopic::List);
    assert_eq!(state.help.return_screen, Screen::List);
    assert!(!state.help.index_open);
    // Arrow keys scroll help
    state.handle_key(KeyCode::Down);
    assert_eq!(state.screen, Screen::Help);
    assert_eq!(state.help.scroll, 1);
    state.handle_key(KeyCode::Up);
    assert_eq!(state.screen, Screen::Help);
    assert_eq!(state.help.scroll, 0);
    // Esc closes help and resets scroll
    state.help.scroll = 5;
    state.handle_key(KeyCode::Esc);
    assert_eq!(state.screen, Screen::List);
    assert_eq!(state.help.scroll, 0);
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
fn minimal_hint_is_empty_when_idle() {
    assert_eq!(MINIMAL_HINT, "");
    let (hint, colored) = footer_with_status(None, MINIMAL_HINT);
    assert_eq!(hint, "");
    assert!(colored);
    let (status, colored) = footer_with_status(Some("Downloaded file.txt"), MINIMAL_HINT);
    assert_eq!(status, "Downloaded file.txt");
    assert!(!colored);
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
fn confirm_prompt_shows_watching_indicator_for_upload() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: "a".into(),
        filename: "notes.txt".into(),
        local_path: PathBuf::from("/tmp/notes.txt"),
    });
    state.screen = Screen::Confirm;
    state.upload.watching = true;

    let prompt = confirm_prompt(&state);
    assert!(prompt.contains("watching for edits"));
    assert!(
        !prompt.contains("y yes"),
        "y/e hints should be hidden while watching"
    );
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
fn row_mark_none_for_empty_reasons() {
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
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
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
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
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        },
        GistFile {
            gist_id: "b".into(),
            description: "second long description here".into(),
            filename: "b.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        },
        GistFile {
            gist_id: "b".into(),
            description: "second".into(),
            filename: "beta.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
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
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        },
        GistFile {
            gist_id: "b".into(),
            description: "status".into(),
            filename: "statusline.sh".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        },
    ];

    assert_eq!(state.ranked_gists()[0].file.filename, "settings.json");
    state.handle_key(KeyCode::Down);
    assert_eq!(state.ranked_gists()[0].file.filename, "statusline.sh");
}

/// `ranked_gists` / `visible_locals` are recomputed from current state on every call —
/// there is intentionally NO cache for them (see #154, closed by-design: a content-hash /
/// epoch memo could silently render a stale ordering the unit suite would not catch).
/// `selected_gist` / `selected_local` are defined as `list[index]`, so they must stay
/// identical to a fresh recompute even after an earlier read and an input mutation. This
/// test pins that invariant: a future stale cache would break it loudly here.
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
            gist_id: "a".into(),
            description: "settings".into(),
            filename: "settings.json".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: String::new(),
            fork_of_id: None,
            raw_url: None,
            content_type: None,
            node_id: None,
        },
        GistFile {
            gist_id: "b".into(),
            description: "status".into(),
            filename: "statusline.sh".into(),
            public: false,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: String::new(),
            fork_of_id: None,
            raw_url: None,
            content_type: None,
            node_id: None,
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
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
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
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
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
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
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
    detail.detail.gist_id = Some("g1".into());
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
fn c_in_detail_requests_compaction_not_gist_manager() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists;
    assert_eq!(state.handle_key(KeyCode::Char('c')), KeyOutcome::None);
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("a".into());
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
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
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
fn confirm_upload_y_is_blocked_while_watching() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: "a".into(),
        filename: "settings.json".into(),
        local_path: PathBuf::from("/tmp/settings.json"),
    });
    state.screen = Screen::Confirm;
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
    state.pending_action = Some(PendingAction::Upload {
        gist_id: "a".into(),
        filename: "settings.json".into(),
        local_path: PathBuf::from("/tmp/settings.json"),
    });
    state.screen = Screen::Confirm;
    state.upload.watching = true;

    assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::None);
    assert_eq!(state.status.as_deref(), Some("editor already open"));
}

#[test]
fn confirm_upload_n_cancels_and_resets_watching() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: "a".into(),
        filename: "settings.json".into(),
        local_path: PathBuf::from("/tmp/settings.json"),
    });
    state.screen = Screen::Confirm;
    state.upload.watching = true;

    assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
    assert!(state.pending_action.is_none());
    assert_eq!(state.screen, Screen::List);
    assert!(
        !state.upload.watching,
        "cancelling must reset watching so a future upload-edit session isn't blocked forever \
         by a stale flag (the background thread is not force-killed and cleans up on its own)"
    );
}

#[test]
fn confirm_upload_n_cancels_to_diff_return_screen() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: "a".into(),
        filename: "settings.json".into(),
        local_path: PathBuf::from("/tmp/settings.json"),
    });
    state.screen = Screen::Confirm;
    state.diff_return = Screen::Pins;

    assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
    assert!(state.pending_action.is_none());
    assert_eq!(
        state.screen,
        Screen::Pins,
        "cancelling an upload initiated from Pins must return to Pins, not always List"
    );
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
        let (program, args) = super::run_loop::editor_command(ed).unwrap();
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
    let (program, args) = super::run_loop::editor_command("/usr/local/bin/zed -n").unwrap();
    assert_eq!(program, "/usr/local/bin/zed");
    assert_eq!(args, vec!["-n", "--wait"]);
}

#[test]
fn editor_command_leaves_terminal_editors_untouched() {
    for ed in ["vi", "vim", "nvim", "nano", "emacs", "hx"] {
        let (program, args) = super::run_loop::editor_command(ed).unwrap();
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
    let (_, args) = super::run_loop::editor_command("code --wait").unwrap();
    assert_eq!(args, vec!["--wait"]);
    let (_, args) = super::run_loop::editor_command("subl -w").unwrap();
    assert_eq!(args, vec!["-w"]);
}

#[test]
fn editor_command_blank_is_none() {
    assert!(super::run_loop::editor_command("").is_none());
    assert!(super::run_loop::editor_command("   ").is_none());
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
            super::run_loop::editor_is_gui(ed),
            "{ed} should be recognised as a GUI editor"
        );
    }
}

#[test]
fn editor_is_gui_rejects_terminal_editors() {
    for ed in ["vi", "vim", "nvim", "nano", "emacs", "hx"] {
        assert!(
            !super::run_loop::editor_is_gui(ed),
            "{ed} should not be recognised as a GUI editor"
        );
    }
}

#[test]
fn editor_is_gui_matches_by_basename_from_full_path() {
    assert!(super::run_loop::editor_is_gui("/usr/local/bin/zed"));
    assert!(super::run_loop::editor_is_gui("C:\\Tools\\code.exe"));
}

// Whichever editor is used, the confirmed upload must send the edited (redacted) buffer, not
// the original file snapshot taken at preview time.

#[test]
fn content_to_upload_prefers_edited_content() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: "a".into(),
        filename: "notes.txt".into(),
        local_path: PathBuf::from("/tmp/notes.txt"),
    });
    state.upload.original_content = "token=abc123secret".into();
    state.upload.edited_content = Some("token=REDACTED".into());
    assert_eq!(state.content_to_upload(), "token=REDACTED");
}

#[test]
fn content_to_upload_prefers_edited_content_for_json() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: "a".into(),
        filename: "settings.json".into(),
        local_path: PathBuf::from("/tmp/settings.json"),
    });
    state.upload.original_content = r#"{"token":"abc123secret"}"#.into();
    state.upload.edited_content = Some(r#"{"token":"REDACTED"}"#.into());
    assert_eq!(state.content_to_upload(), r#"{"token":"REDACTED"}"#);
}

fn upload_pending(gist_id: &str, filename: &str) -> PendingAction {
    PendingAction::Upload {
        gist_id: gist_id.into(),
        filename: filename.into(),
        local_path: PathBuf::from(format!("/tmp/{filename}")),
    }
}

#[test]
fn apply_upload_edit_event_content_changed_updates_diff_live() {
    let mut state = initial_state();
    state.screen = Screen::Confirm;
    state.pending_action = Some(upload_pending("a", "notes.txt"));
    state.upload.watching = true;
    state.upload.remote_content = Some("old\n".into());
    state.upload.local_label = Some("local".into());
    state.upload.gist_label = Some("gist".into());

    state.apply_upload_edit_event(super::run_loop::UploadEditWatchEvent::ContentChanged {
        gist_id: "a".into(),
        filename: "notes.txt".into(),
        content: "new\n".into(),
    });

    assert_eq!(state.upload.edited_content.as_deref(), Some("new\n"));
    assert!(
        state.upload.watching,
        "still watching — editor hasn't closed yet"
    );
    assert!(state.diff_text.contains("new"));
}

#[test]
fn apply_upload_edit_event_editor_closed_stops_watching() {
    let mut state = initial_state();
    state.screen = Screen::Confirm;
    state.pending_action = Some(upload_pending("a", "notes.txt"));
    state.upload.watching = true;

    state.apply_upload_edit_event(super::run_loop::UploadEditWatchEvent::EditorClosed {
        gist_id: "a".into(),
        filename: "notes.txt".into(),
        content: "final\n".into(),
    });

    assert_eq!(state.upload.edited_content.as_deref(), Some("final\n"));
    assert!(!state.upload.watching);
}

#[test]
fn apply_upload_edit_event_read_error_stops_watching_and_sets_status() {
    let mut state = initial_state();
    state.screen = Screen::Confirm;
    state.pending_action = Some(upload_pending("a", "notes.txt"));
    state.upload.watching = true;

    state.apply_upload_edit_event(super::run_loop::UploadEditWatchEvent::ReadError {
        gist_id: "a".into(),
        filename: "notes.txt".into(),
        message: "permission denied".into(),
    });

    assert!(!state.upload.watching);
    assert_eq!(
        state.status.as_deref(),
        Some("failed to read edited file: permission denied")
    );
}

#[test]
fn apply_upload_edit_event_discards_when_context_is_stale() {
    let mut state = initial_state();
    // The user already left Confirm (e.g. cancelled) before this late event arrived.
    state.screen = Screen::List;
    state.pending_action = None;
    state.upload.watching = false;
    state.upload.edited_content = None;

    state.apply_upload_edit_event(super::run_loop::UploadEditWatchEvent::ContentChanged {
        gist_id: "a".into(),
        filename: "notes.txt".into(),
        content: "should be ignored".into(),
    });

    assert_eq!(state.upload.edited_content, None);
}

#[test]
fn apply_upload_edit_event_discards_when_a_different_upload_is_now_pending() {
    let mut state = initial_state();
    // A new upload edit session started before the OLD one's final event arrived.
    state.screen = Screen::Confirm;
    state.pending_action = Some(upload_pending("a", "other.txt"));
    state.upload.watching = true;
    state.upload.edited_content = Some("current session content".into());

    state.apply_upload_edit_event(super::run_loop::UploadEditWatchEvent::EditorClosed {
        gist_id: "a".into(),
        filename: "notes.txt".into(), // stale session's filename, not "other.txt"
        content: "stale content".into(),
    });

    assert_eq!(
        state.upload.edited_content.as_deref(),
        Some("current session content")
    );
    assert!(
        state.upload.watching,
        "the current session's watch must not be cancelled"
    );
}

#[test]
fn apply_upload_edit_event_discards_stale_event_after_cancel_reentry_same_identity() {
    let mut state = initial_state();
    // Simulates: user cancelled a GUI-editor watch session (n resets watching to false but
    // does NOT kill the background thread), then re-entered upload for the SAME gist/file
    // without pressing `e` again. An event from the abandoned first session's thread must
    // not silently overwrite this new, non-watching session's content.
    state.screen = Screen::Confirm;
    state.pending_action = Some(upload_pending("a", "notes.txt"));
    state.upload.watching = false; // never re-entered edit mode this session
    state.upload.edited_content = None;

    state.apply_upload_edit_event(super::run_loop::UploadEditWatchEvent::ContentChanged {
        gist_id: "a".into(),
        filename: "notes.txt".into(),
        content: "leaked from abandoned session".into(),
    });

    assert_eq!(
        state.upload.edited_content, None,
        "an event from an abandoned (cancelled, still-running) watch session must not \
         leak into a new, non-watching session with the same gist/file identity"
    );
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        },
        GistFile {
            gist_id: "abc123".into(),
            description: "my notes".into(),
            filename: "b.md".into(),
            public: false,
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
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
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
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
    assert_eq!(state.gist_manager.index, 1);
    assert_eq!(state.selected_group().unwrap().id, "b");
}

#[test]
fn g_with_no_gists_is_blocked() {
    let mut state = initial_state();
    assert_eq!(state.handle_key(KeyCode::Char('g')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn detail_e_edits_description_with_prefill_and_enter_applies() {
    let mut state = state_with_two_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("a".into());
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
fn detail_description_edits_mid_string_with_cursor_keys() {
    let mut state = state_with_two_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("a".into());
    state.handle_key(KeyCode::Char('e'));
    assert_eq!(state.description_input, "My Ghostty config");
    // Jump to the start, step right past "My", and insert without retyping the rest.
    state.handle_key(KeyCode::Home);
    state.handle_key(KeyCode::Right);
    state.handle_key(KeyCode::Right);
    state.handle_key(KeyCode::Char(' '));
    state.handle_key(KeyCode::Char('o'));
    state.handle_key(KeyCode::Char('w'));
    state.handle_key(KeyCode::Char('n'));
    assert_eq!(state.description_input, "My own Ghostty config");
    // Delete removes the char at the cursor (the space before "Ghostty").
    state.handle_key(KeyCode::Delete);
    assert_eq!(state.description_input, "My ownGhostty config");
}

#[test]
fn create_description_edits_mid_string_with_cursor_keys() {
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Create {
        local_path: PathBuf::from("notes.txt"),
    });
    state.editing_description = true;
    state.screen = Screen::Confirm;
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
fn detail_esc_cancels_description_edit() {
    let mut state = state_with_two_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("a".into());
    state.handle_key(KeyCode::Char('e'));
    assert!(state.editing_description);
    state.handle_key(KeyCode::Esc);
    assert!(!state.editing_description);
    assert!(state.description_input.is_empty());
}

#[test]
fn detail_x_stages_whole_gist_delete() {
    let mut state = state_with_two_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("b".into());
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
    state.screen = Screen::Gists;
    state.handle_key(KeyCode::Char('/'));
    assert!(state.gist_manager.filtering);
    for c in "ssh".chars() {
        state.handle_key(KeyCode::Char(c));
    }
    let vis = state.visible_gist_groups();
    assert_eq!(vis.len(), 1);
    assert_eq!(vis[0].id, "b"); // "SSH config"

    state.handle_key(KeyCode::Esc);
    assert!(!state.gist_manager.filtering);
    assert!(state.gist_manager.filter_query.is_empty());
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
    assert_eq!(state.gist_manager.sort, GistGroupSort::Updated);
    assert_eq!(state.visible_gist_groups()[0].id, "new-upd");
    // s -> sort by created (newest created first).
    state.handle_key(KeyCode::Char('s'));
    assert_eq!(state.gist_manager.sort, GistGroupSort::Created);
    assert_eq!(state.visible_gist_groups()[0].id, "old-upd");
}

#[test]
fn gist_view_left_right_scrolls_horizontally() {
    let mut state = state_with_two_gists();
    state.screen = Screen::Gists;
    assert_eq!(state.gist_manager.hscroll, 0);
    state.handle_key(KeyCode::Right);
    assert_eq!(state.gist_manager.hscroll, 1);
    state.handle_key(KeyCode::Left);
    assert_eq!(state.gist_manager.hscroll, 0);
    // Left at the origin saturates at 0.
    state.handle_key(KeyCode::Left);
    assert_eq!(state.gist_manager.hscroll, 0);
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

fn pins_state_with_long_home_path() -> AppState {
    let mut state = initial_state();
    state.screen = Screen::Pins;
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    state.pinned = vec![PinnedMapping {
        local_path: home.join("code/very/deeply/nested/project/config.json"),
        gist_id: "g1".into(),
        gist_filename: "config.json".into(),
        direction: None,
        last_seen_hash: None,
    }];
    state.pins.index = 0;
    state
}

#[test]
fn pins_hscroll_starts_at_zero() {
    assert_eq!(initial_state().pins.hscroll, 0);
}

#[test]
fn create_diff_title_shortens_home_path() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Create {
        local_path: home.join("notes.txt"),
    });
    assert_eq!(diff_title(&state), "Create gist from ~/notes.txt");
}

#[test]
fn diff_view_title_shortens_single_home_path() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    let mut state = initial_state();
    state.pending_action = None;
    state.preview_local = PathBuf::new();
    state.download_target = home.join("notes.txt");
    assert_eq!(diff_title(&state), "Diff → ~/notes.txt");
}

#[test]
fn diff_view_title_shortens_both_home_paths() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    let mut state = initial_state();
    state.pending_action = None;
    state.preview_local = home.join("src").join("a.txt");
    state.download_target = home.join("b.txt");
    assert_eq!(diff_title(&state), "Diff: ~/src/a.txt → ~/b.txt");
}

#[test]
fn create_confirm_prompt_shortens_home_path() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    let mut state = initial_state();
    state.pending_action = Some(PendingAction::Create {
        local_path: home.join("notes.txt"),
    });
    assert!(
        confirm_prompt(&state).starts_with("Create gist from ~/notes.txt"),
        "got {}",
        confirm_prompt(&state)
    );
}

#[test]
fn pin_row_label_shows_home_as_tilde() {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
    let label = pin_row_label(
        "✓",
        &home.join("code/gistui"),
        "abc123",
        "notes.txt",
        "2h",
        "3h",
    );
    assert!(
        label.contains("~/code/gistui"),
        "expected ~ home in label, got {label}"
    );
    assert!(!label.contains(home.to_string_lossy().as_ref()));
}

#[test]
fn pins_right_scrolls_then_clamps_at_a_bound() {
    let mut state = pins_state_with_long_home_path();
    state.handle_key(KeyCode::Right);
    assert_eq!(state.pins.hscroll, 1, "Right should advance the scroll");
    // Far past the end clamps to a stable maximum (does not run away).
    for _ in 0..500 {
        state.handle_key(KeyCode::Right);
    }
    let clamped = state.pins.hscroll;
    state.handle_key(KeyCode::Right);
    assert_eq!(state.pins.hscroll, clamped, "scroll must clamp at its max");
    assert!(clamped > 0, "a long path must be scrollable");
}

#[test]
fn pins_left_clamps_at_zero() {
    let mut state = pins_state_with_long_home_path();
    state.handle_key(KeyCode::Right);
    state.handle_key(KeyCode::Left);
    state.handle_key(KeyCode::Left);
    assert_eq!(state.pins.hscroll, 0);
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
    assert!(state.pins.hscroll > 0);
    state.handle_key(KeyCode::Down);
    assert_eq!(state.pins.hscroll, 0, "moving selection resets hscroll");
}

#[test]
fn entering_pins_screen_resets_hscroll() {
    let mut state = pins_state_with_long_home_path();
    state.handle_key(KeyCode::Right);
    assert!(state.pins.hscroll > 0);
    state.screen = Screen::List;
    state.handle_key(KeyCode::Char('P'));
    assert_eq!(state.screen, Screen::Pins);
    assert_eq!(state.pins.hscroll, 0);
}

#[test]
fn top_bar_gists_click_opens_gist_manager_from_any_screen() {
    let mut state = state_with_gists();
    state.screen = Screen::Preview; // arbitrary screen that has no 'g' binding of its own
    let layout = MouseLayout {
        top_bar_gists: Some(Rect::new(10, 0, 7, 1)),
        ..Default::default()
    };
    let out = state.handle_mouse(MouseInput::Click { col: 12, row: 0 }, &layout);
    assert_eq!(state.screen, Screen::Gists);
    assert_eq!(out, KeyOutcome::None);
}

#[test]
fn top_bar_pins_click_opens_pins_from_any_screen() {
    let mut state = pins_state_with_long_home_path();
    state.handle_key(KeyCode::Right); // dirty the hscroll so the reset is observable
    assert!(state.pins.hscroll > 0);
    state.screen = Screen::Preview;
    let layout = MouseLayout {
        top_bar_pins: Some(Rect::new(20, 0, 6, 1)),
        ..Default::default()
    };
    let out = state.handle_mouse(MouseInput::Click { col: 22, row: 0 }, &layout);
    assert_eq!(state.screen, Screen::Pins);
    assert_eq!(state.pins.hscroll, 0);
    assert_eq!(out, KeyOutcome::None);
}

#[test]
fn top_bar_help_click_opens_help_and_remembers_return_screen_from_any_screen() {
    let mut state = state_with_gists();
    state.screen = Screen::Preview;
    let layout = MouseLayout {
        top_bar_help: Some(Rect::new(30, 0, 7, 1)),
        ..Default::default()
    };
    let out = state.handle_mouse(MouseInput::Click { col: 32, row: 0 }, &layout);
    assert_eq!(state.screen, Screen::Help);
    assert_eq!(state.help.return_screen, Screen::Preview);
    assert_eq!(out, KeyOutcome::None);
}

#[test]
fn top_bar_help_click_while_already_on_help_does_not_trap_keyboard_exit() {
    let mut state = state_with_gists();
    state.screen = Screen::Preview;
    let layout = MouseLayout {
        top_bar_help: Some(Rect::new(30, 0, 7, 1)),
        ..Default::default()
    };
    // First click opens Help from Preview, remembering Preview as the return screen.
    state.handle_mouse(MouseInput::Click { col: 32, row: 0 }, &layout);
    assert_eq!(state.screen, Screen::Help);
    assert_eq!(state.help.return_screen, Screen::Preview);

    // A second click on the same top-bar Help hotspot, now that Help is already open, must
    // be a no-op — it must not overwrite return_screen with Screen::Help, which would trap
    // Esc/`?`/the close button in Help with no keyboard way out.
    let out = state.handle_mouse(MouseInput::Click { col: 32, row: 0 }, &layout);
    assert_eq!(state.screen, Screen::Help);
    assert_eq!(state.help.return_screen, Screen::Preview);
    assert_eq!(out, KeyOutcome::None);

    // Esc must still return to the real origin screen, not stay stuck on Help.
    state.handle_key(KeyCode::Esc);
    assert_eq!(state.screen, Screen::Preview);
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
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
    }];
    assert_eq!(
        state.handle_key(KeyCode::Char('S')),
        KeyOutcome::SyncSelectedPair
    );
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
        gist_id: "g1".into(),
        description: "My ZSH profile".into(),
        filename: filename.into(),
        public: true,
        updated_at: "2026-06-10T00:00:00Z".into(),
        created_at: "2020-01-01T00:00:00Z".into(),
        owner_login: "akunzai".into(),
        fork_of_id: None,
        raw_url: None,
        content_type: None,
        node_id: None,
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
    state.screen = Screen::Gists;
    state
}

#[test]
fn gists_filter_navigates_while_typing() {
    let mut state = gists_screen_state();
    state.gist_manager.filtering = true;

    state.handle_key(KeyCode::Down);
    assert_eq!(state.gist_manager.index, 1);
    assert!(state.gist_manager.filtering);
    state.handle_key(KeyCode::Up);
    assert_eq!(state.gist_manager.index, 0);
}

#[test]
fn gists_filter_empty_backspace_exits() {
    let mut state = gists_screen_state();
    state.gist_manager.filtering = true;

    state.handle_key(KeyCode::Char('a'));
    state.handle_key(KeyCode::Backspace); // empty again, still filtering
    assert!(state.gist_manager.filtering);
    state.handle_key(KeyCode::Backspace); // empty -> exit
    assert!(!state.gist_manager.filtering);
}

#[test]
fn gists_filter_tab_is_noop() {
    let mut state = gists_screen_state();
    state.gist_manager.filtering = true;
    state.handle_key(KeyCode::Char('a'));

    state.handle_key(KeyCode::Tab);
    assert!(state.gist_manager.filtering); // still typing
    assert_eq!(state.gist_manager.filter_query, "a"); // unchanged
}

// ── Pins screen filter ────────────────────────────────────────────────────────

fn state_with_pins(rows: &[(&str, &str, &str)]) -> AppState {
    let mut state = initial_state();
    state.cwd = PathBuf::from("/cwd");
    state.screen = Screen::Pins;
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

    state.pins.filter_query = "lua".into(); // matches filename of row 1
    assert_eq!(state.visible_pin_indices(), vec![1]);

    state.pins.filter_query = "ZSH".into(); // case-insensitive, matches path of row 0
    assert_eq!(state.visible_pin_indices(), vec![0]);
}

#[test]
fn selected_pin_index_maps_through_filter() {
    let mut state = state_with_pins(&[
        ("/cwd/alpha", "g1", "alpha"),
        ("/cwd/beta", "g2", "beta"),
        ("/cwd/gamma", "g3", "gamma"),
    ]);
    state.pins.filter_query = "gamma".into(); // only row 2 visible
    state.pins.index = 0; // first (and only) visible row
    assert_eq!(state.selected_pin_index(), Some(2)); // TRUE index, not 0
}

#[test]
fn pins_down_clamps_to_filtered_count() {
    let mut state = state_with_pins(&[
        ("/cwd/a", "g1", "a"),
        ("/cwd/blua", "g2", "blua"),
        ("/cwd/c", "g3", "c"),
    ]);
    state.pins.filter_query = "lua".into(); // 1 visible
    state.handle_key(KeyCode::Down);
    assert_eq!(state.pins.index, 0); // clamped to the single filtered row
}

#[test]
fn pins_filter_input_behaviors() {
    let mut state = state_with_pins(&[("/cwd/a", "g1", "a"), ("/cwd/b", "g2", "b")]);
    state.pins.filtering = true;

    // live nav while typing
    state.handle_key(KeyCode::Down);
    assert_eq!(state.pins.index, 1);
    assert!(state.pins.filtering);

    // Tab is a no-op (single pane)
    state.handle_key(KeyCode::Char('a'));
    state.handle_key(KeyCode::Tab);
    assert!(state.pins.filtering);
    assert_eq!(state.pins.filter_query, "a");

    // Esc clears + exits
    state.handle_key(KeyCode::Esc);
    assert!(!state.pins.filtering);
    assert_eq!(state.pins.filter_query, "");

    // Backspace on empty exits
    state.pins.filtering = true;
    state.handle_key(KeyCode::Backspace);
    assert!(!state.pins.filtering);

    // Enter keeps query + exits
    state.pins.filtering = true;
    state.handle_key(KeyCode::Char('b'));
    state.handle_key(KeyCode::Enter);
    assert!(!state.pins.filtering);
    assert_eq!(state.pins.filter_query, "b");
}

#[test]
fn help_topic_all_is_ordered_and_titled() {
    let all = HelpTopic::all();
    assert_eq!(all.len(), 9);
    assert_eq!(all[0], HelpTopic::List);
    assert_eq!(all[4], HelpTopic::Revisions);
    assert_eq!(all[8], HelpTopic::General);
    assert_eq!(HelpTopic::Pins.title(), "Pinned Mappings");
}

#[test]
fn help_topic_for_screen_maps_key_dense_screens() {
    assert_eq!(HelpTopic::for_screen(Screen::List), HelpTopic::List);
    assert_eq!(HelpTopic::for_screen(Screen::Pins), HelpTopic::Pins);
    assert_eq!(HelpTopic::for_screen(Screen::Gists), HelpTopic::GistManager);
    assert_eq!(
        HelpTopic::for_screen(Screen::GistDetail),
        HelpTopic::GistDetail
    );
    assert_eq!(
        HelpTopic::for_screen(Screen::Revisions),
        HelpTopic::Revisions
    );
    assert_eq!(HelpTopic::for_screen(Screen::Diff), HelpTopic::List);
}

#[test]
fn capital_h_from_list_opens_revisions_for_selected_gist_file() {
    let mut state = list_state_with_matches();
    state.focus = FocusPane::Gist;
    state.gist_index = 0;
    let outcome = state.handle_key(KeyCode::Char('H'));
    assert_eq!(outcome, KeyOutcome::FetchRevisions);
    assert_eq!(state.screen, Screen::Revisions);
    assert_eq!(state.revision.gist_id.as_deref(), Some("a"));
    assert_eq!(state.revision.target_file, "settings.json");
    assert_eq!(state.revision.return_screen, Screen::List);
}

#[test]
fn capital_h_from_gist_detail_opens_revisions_and_fetches() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    state.detail.file_cursor = 1;
    let outcome = state.handle_key(KeyCode::Char('H'));
    assert_eq!(outcome, KeyOutcome::FetchRevisions);
    assert_eq!(state.screen, Screen::Revisions);
    assert_eq!(state.revision.gist_id.as_deref(), Some("g1"));
    assert_eq!(state.revision.target_file, "b.txt");
    assert_eq!(state.revision.return_screen, Screen::GistDetail);
    assert!(state.revision.entries.is_none());
}

#[test]
fn revisions_r_on_head_is_blocked() {
    let mut state = state_with_gists();
    state.screen = Screen::Revisions;
    state.revision.entries = Some(vec![crate::domain::GistRevision {
        version: "abc".into(),
        committed_at: "2026-06-10T00:00:00Z".into(),
        user: "u".into(),
        change_status: crate::domain::GistRevisionChangeStatus {
            total: 1,
            additions: 1,
            deletions: 0,
        },
    }]);
    state.handle_key(KeyCode::Char('r'));
    assert_eq!(
        state.status.as_deref(),
        Some("only one revision — nothing to restore")
    );
}

#[test]
fn revisions_capital_d_on_current_shows_status() {
    let mut state = state_with_gists();
    state.screen = Screen::Revisions;
    state.revision.index = 0;
    state.revision.entries = Some(vec![
        crate::domain::GistRevision {
            version: "v2".into(),
            committed_at: "2026-06-10T00:00:00Z".into(),
            user: "u".into(),
            change_status: crate::domain::GistRevisionChangeStatus {
                total: 1,
                additions: 1,
                deletions: 0,
            },
        },
        crate::domain::GistRevision {
            version: "v1".into(),
            committed_at: "2026-06-01T00:00:00Z".into(),
            user: "u".into(),
            change_status: crate::domain::GistRevisionChangeStatus {
                total: 2,
                additions: 2,
                deletions: 0,
            },
        },
    ]);
    assert_eq!(state.handle_key(KeyCode::Char('D')), KeyOutcome::None);
    assert_eq!(state.status.as_deref(), Some("already at current revision"));
    state.revision.index = 1;
    assert_eq!(
        state.handle_key(KeyCode::Char('D')),
        KeyOutcome::RevisionDiff
    );
}

#[test]
fn revisions_enter_triggers_incremental_diff() {
    let mut state = state_with_gists();
    state.screen = Screen::Revisions;
    state.revision.index = 0;
    state.revision.entries = Some(vec![
        crate::domain::GistRevision {
            version: "v2".into(),
            committed_at: "2026-06-10T00:00:00Z".into(),
            user: "u".into(),
            change_status: crate::domain::GistRevisionChangeStatus {
                total: 1,
                additions: 1,
                deletions: 0,
            },
        },
        crate::domain::GistRevision {
            version: "v1".into(),
            committed_at: "2026-06-01T00:00:00Z".into(),
            user: "u".into(),
            change_status: crate::domain::GistRevisionChangeStatus {
                total: 2,
                additions: 2,
                deletions: 0,
            },
        },
    ]);
    assert_eq!(
        state.handle_key(KeyCode::Enter),
        KeyOutcome::RevisionDiffIncremental
    );
    state.revision.index = 1;
    assert_eq!(
        state.handle_key(KeyCode::Enter),
        KeyOutcome::RevisionDiffIncremental
    );
}

#[test]
fn revision_diff_omits_download_upload() {
    let mut state = initial_state();
    state.screen = Screen::Diff;
    state.diff_return = Screen::Revisions;
    state.diff_identical = false;
    let footer = diff_footer(&state);
    assert!(!footer.contains("download"));
    assert!(!footer.contains("upload"));
    assert_eq!(state.handle_key(KeyCode::Char('d')), KeyOutcome::None);
    assert_eq!(state.handle_key(KeyCode::Char('u')), KeyOutcome::None);
}

#[test]
fn revisions_capital_f_cycles_target_file() {
    let mut state = state_with_gists();
    state.screen = Screen::Revisions;
    state.revision.gist_id = Some("g1".into());
    state.revision.target_file = "a.txt".into();
    state.revision.entries = Some(vec![]);
    state.handle_key(KeyCode::Char('F'));
    assert_eq!(state.revision.target_file, "b.txt");
    state.handle_key(KeyCode::Char('F'));
    assert_eq!(state.revision.target_file, "a.txt");
    assert_eq!(state.revision_target_file_label(), "a.txt (1/2)");
}

#[test]
fn revisions_capital_f_on_single_file_gist_shows_status() {
    let mut state = initial_state();
    state.gists = vec![GistFile {
        gist_id: "g1".into(),
        description: "solo".into(),
        filename: "only.txt".into(),
        public: false,
        updated_at: "x".into(),
        created_at: "x".into(),
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
    }];
    state.screen = Screen::Revisions;
    state.revision.gist_id = Some("g1".into());
    state.revision.target_file = "only.txt".into();
    state.revision.entries = Some(vec![]);
    state.handle_key(KeyCode::Char('F'));
    assert_eq!(state.status.as_deref(), Some("only one file in this gist"));
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
fn ctrl_f_pages_gist_detail_files() {
    use crossterm::event::KeyModifiers;
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    state.detail.file_cursor = 0;
    state.handle_key_with(KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert_eq!(state.detail.file_cursor, 1);
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
fn pins_page_keys_jump_selection() {
    use crossterm::event::KeyModifiers;
    let mut state = initial_state();
    state.screen = Screen::Pins;
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
    assert_eq!(state.pins.index, 10);
    state.handle_key(KeyCode::PageUp);
    assert_eq!(state.pins.index, 0);
}

#[test]
fn gists_page_keys_jump_selection() {
    let mut state = initial_state();
    state.screen = Screen::Gists;
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
    assert_eq!(state.gist_manager.index, 10);
    state.handle_key(KeyCode::PageDown);
    assert_eq!(state.gist_manager.index, 11);
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
fn lowercase_h_does_not_open_revision_history() {
    let mut state = list_state_with_matches();
    state.focus = FocusPane::Gist;
    state.gist_index = 0;
    assert_eq!(state.handle_key(KeyCode::Char('h')), KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn restore_revision_confirm_prompt_and_y_intent() {
    let mut state = state_with_gists();
    state.screen = Screen::Confirm;
    state.pending_action = Some(PendingAction::RestoreRevision {
        gist_id: "g1".into(),
        filename: "a.txt".into(),
        version: "oldsha".into(),
        version_label: "oldsha (3d ago)".into(),
        content: "old\n".into(),
    });
    assert_eq!(
        confirm_modal_style(&state),
        ("Restore revision", Color::Yellow)
    );
    assert!(confirm_prompt(&state).contains("Restore a.txt to revision oldsha (3d ago)"));
    assert_eq!(
        state.handle_key(KeyCode::Char('y')),
        KeyOutcome::ExecuteRestoreRevision
    );
}

#[test]
fn question_mark_opens_contextual_help_from_pins() {
    let mut state = initial_state();
    state.screen = Screen::Pins;
    state.handle_key(KeyCode::Char('?'));
    assert_eq!(state.screen, Screen::Help);
    assert_eq!(state.help.topic, HelpTopic::Pins);
    assert_eq!(state.help.return_screen, Screen::Pins);
    assert!(!state.help.index_open);
}

#[test]
fn help_topic_view_tab_opens_index_at_current_topic() {
    let mut state = initial_state();
    state.screen = Screen::Help;
    state.help.topic = HelpTopic::GistManager; // index 2
    state.handle_key(KeyCode::Tab);
    assert!(state.help.index_open);
    assert_eq!(state.help.index_sel, 2);
}

#[test]
fn help_topic_view_number_switches_topic() {
    let mut state = initial_state();
    state.screen = Screen::Help;
    state.help.topic = HelpTopic::List;
    state.help.scroll = 5;
    state.handle_key(KeyCode::Char('2')); // 2 -> Pins (index 1)
    assert_eq!(state.help.topic, HelpTopic::Pins);
    assert_eq!(state.help.scroll, 0);
    assert!(!state.help.index_open);
}

#[test]
fn help_topic_view_esc_returns_to_origin() {
    let mut state = initial_state();
    state.screen = Screen::Help;
    state.help.return_screen = Screen::Gists;
    state.handle_key(KeyCode::Esc);
    assert_eq!(state.screen, Screen::Gists);
}

#[test]
fn help_index_navigates_and_enter_opens_topic() {
    let mut state = initial_state();
    state.screen = Screen::Help;
    state.help.index_open = true;
    state.help.index_sel = 0;
    state.handle_key(KeyCode::Down); // -> 1
    state.handle_key(KeyCode::Down); // -> 2 (GistManager)
    assert_eq!(state.help.index_sel, 2);
    state.handle_key(KeyCode::Enter);
    assert!(!state.help.index_open);
    assert_eq!(state.help.topic, HelpTopic::GistManager);
}

#[test]
fn help_index_esc_returns_to_origin() {
    let mut state = initial_state();
    state.screen = Screen::Help;
    state.help.index_open = true;
    state.help.return_screen = Screen::List;
    state.handle_key(KeyCode::Esc);
    assert_eq!(state.screen, Screen::List);
    assert!(!state.help.index_open);
}

#[test]
fn help_index_question_mark_exits_help() {
    let mut state = initial_state();
    state.screen = Screen::Help;
    state.help.index_open = true;
    state.help.return_screen = Screen::Pins;
    state.handle_key(KeyCode::Char('?'));
    assert_eq!(state.screen, Screen::Pins);
    assert!(!state.help.index_open);
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
        ..GistFile::for_sync("g1".into(), "settings.json".into(), None)
    }];

    assert_eq!(
        state.pin_sync_status(0),
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
        ..GistFile::for_sync("g1".into(), "settings.json".into(), None)
    }];

    assert_eq!(
        state.pin_sync_status(0),
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
        ..GistFile::for_sync("g1".into(), "settings.json".into(), None)
    }];

    assert_eq!(
        state.pin_sync_status(0),
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
        ..GistFile::for_sync("g1".into(), "settings.json".into(), None)
    }];

    assert_eq!(state.pin_sync_status(0), crate::domain::SyncStatus::Push);
}

#[test]
fn forked_filter_shows_only_forks() {
    let mut state = initial_state();
    state.gists = vec![
        GistFile {
            gist_id: "owned".into(),
            description: "mine".into(),
            filename: "a.txt".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: "me".into(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
        },
        GistFile {
            gist_id: "forked".into(),
            description: "fork".into(),
            filename: "b.txt".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: "me".into(),
            fork_of_id: Some("upstream".into()),

            raw_url: None,

            content_type: None,

            node_id: None,
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
    state.local_index = 0;
    state.gist_index = 0;
    assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::None);
    assert!(state.status.as_ref().unwrap().contains("cannot pin"));
}

#[test]
fn star_key_returns_toggle_intent() {
    let mut state = initial_state();
    state.gists = vec![GistFile {
        gist_id: "g1".into(),
        description: "x".into(),
        filename: "a.txt".into(),
        public: true,
        updated_at: "x".into(),
        created_at: "x".into(),
        owner_login: String::new(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
    }];
    state.gist_index = 0;
    assert_eq!(
        state.handle_key(KeyCode::Char('*')),
        KeyOutcome::ToggleGistStar
    );
}

#[test]
fn starred_filter_lists_only_starred_gists() {
    // With the Starred type filter active, ranked_gists must draw from starred_gists, not the
    // owned list — exercises the owned/starred source switch with data on both sides.
    let mut state = initial_state();
    state.gists = vec![GistFile {
        gist_id: "owned".into(),
        description: "mine".into(),
        filename: "a.txt".into(),
        public: true,
        updated_at: "x".into(),
        created_at: "x".into(),
        owner_login: "me".into(),
        fork_of_id: None,
        raw_url: None,
        content_type: None,
        node_id: None,
    }];
    state.starred_gists = vec![GistFile {
        gist_id: "starred".into(),
        description: "theirs".into(),
        filename: "b.txt".into(),
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

    let ranked = state.ranked_gists();
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].file.gist_id, "starred");
}

#[test]
fn fork_key_returns_fork_intent_for_foreign_gist_in_detail() {
    let mut state = initial_state();
    state.current_user_login = Some("me".into());
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("foreign".into());
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
    assert_eq!(state.handle_key(KeyCode::Char('F')), KeyOutcome::ForkGist);
}

#[test]
fn fork_key_blocked_for_owned_gist_in_detail() {
    let mut state = initial_state();
    state.current_user_login = Some("me".into());
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("mine".into());
    state.gists = vec![GistFile {
        gist_id: "mine".into(),
        description: "x".into(),
        filename: "a.txt".into(),
        public: true,
        updated_at: "x".into(),
        created_at: "x".into(),
        owner_login: "me".into(),
        fork_of_id: None,

        raw_url: None,

        content_type: None,

        node_id: None,
    }];
    assert_eq!(state.handle_key(KeyCode::Char('F')), KeyOutcome::None);
    assert!(state.status.as_ref().unwrap().contains("already yours"));
}

#[test]
fn foreign_detail_mutate_keys_are_silent_noop() {
    let mut state = initial_state();
    state.current_user_login = Some("me".into());
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("foreign".into());
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
    assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::None);
    assert_eq!(state.handle_key(KeyCode::Char('c')), KeyOutcome::None);
    assert_eq!(state.handle_key(KeyCode::Char('X')), KeyOutcome::None);
    assert!(state.status.is_none());
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

    state.screen = Screen::Gists;
    state.gist_manager.type_filter = GistTypeFilter::Starred;
    state.gist_manager.index = 0;
    assert_eq!(state.handle_key(KeyCode::Char('F')), KeyOutcome::None);
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
fn scroll_down_moves_focused_list_by_one() {
    let mut state = state_with_local_paths(&["a.rs", "b.rs", "c.rs"]);
    state.screen = Screen::List;
    state.focus = FocusPane::Local;
    state.local_index = 0;
    let out = state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
    assert_eq!(out, KeyOutcome::None);
    assert_eq!(state.local_index, 1);
}

#[test]
fn scroll_up_moves_focused_list_by_one() {
    let mut state = state_with_local_paths(&["a.rs", "b.rs", "c.rs"]);
    state.screen = Screen::List;
    state.focus = FocusPane::Local;
    state.local_index = 2;
    let out = state.handle_mouse(MouseInput::ScrollUp, &MouseLayout::default());
    assert_eq!(out, KeyOutcome::None);
    assert_eq!(state.local_index, 1);
}

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
    assert_eq!(state.screen, Screen::Diff);
    assert_eq!(state.diff_scroll, 0);
    state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
    assert_eq!(state.diff_scroll, 3);
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
    state.diff_scroll = 3;
    state.handle_mouse(MouseInput::ScrollUp, &MouseLayout::default());
    assert_eq!(state.diff_scroll, 0);
}

#[test]
fn close_button_click_returns_from_help() {
    let mut state = state_with_gists();
    // Simulate entering Help (mirrors what open_help() does).
    state.help.return_screen = Screen::List;
    state.screen = Screen::Help;
    let layout = MouseLayout {
        close_button: Some(Rect::new(36, 0, 5, 1)),
        ..Default::default()
    };
    let out = state.handle_mouse(MouseInput::Click { col: 38, row: 0 }, &layout);
    assert_eq!(out, KeyOutcome::None);
    assert_eq!(state.screen, Screen::List);
}

#[test]
fn close_button_click_outside_is_noop() {
    let mut state = state_with_gists();
    state.help.return_screen = Screen::List;
    state.screen = Screen::Help;
    let layout = MouseLayout {
        close_button: Some(Rect::new(36, 0, 5, 1)),
        ..Default::default()
    };
    // col 35 is just outside the left edge of the close button
    let out = state.handle_mouse(MouseInput::Click { col: 35, row: 0 }, &layout);
    assert_eq!(out, KeyOutcome::None);
    assert_eq!(state.screen, Screen::Help);
}

#[test]
fn list_click_selects_and_focuses_gist_pane() {
    let mut state = state_with_gists();
    state.screen = Screen::List;
    state.focus = FocusPane::Local;
    state.gist_hscroll = 5;
    let hit = PaneHit {
        rect: Rect::new(20, 0, 20, 10),
        offset: 0,
    };
    let layout = MouseLayout {
        gist: Some(hit),
        ..Default::default()
    };
    // row 2 -> content idx 1 (top border is row 0, row 1 = idx 0, row 2 = idx 1)
    let out = state.handle_mouse(MouseInput::Click { col: 25, row: 2 }, &layout);
    assert_eq!(out, KeyOutcome::None);
    assert_eq!(state.focus, FocusPane::Gist);
    assert_eq!(state.gist_index, 1);
    assert_eq!(state.gist_hscroll, 0);
}

#[test]
fn list_click_selects_and_focuses_local_pane() {
    let mut state = state_with_local_paths(&["a.rs", "b.rs", "c.rs"]);
    state.gists = vec![];
    state.screen = Screen::List;
    state.focus = FocusPane::Gist;
    state.local_hscroll = 5;
    let hit = PaneHit {
        rect: Rect::new(0, 0, 20, 10),
        offset: 0,
    };
    let layout = MouseLayout {
        local: Some(hit),
        ..Default::default()
    };
    // row 1 -> idx 0 (first content row after top border)
    let out = state.handle_mouse(MouseInput::Click { col: 5, row: 1 }, &layout);
    assert_eq!(out, KeyOutcome::None);
    assert_eq!(state.focus, FocusPane::Local);
    assert_eq!(state.local_index, 0);
    assert_eq!(state.local_hscroll, 0);
}

#[test]
fn list_double_click_opens_diff() {
    let mut state = state_with_gists();
    state.screen = Screen::List;
    let hit = PaneHit {
        rect: Rect::new(20, 0, 20, 10),
        offset: 0,
    };
    let layout = MouseLayout {
        gist: Some(hit),
        ..Default::default()
    };
    // row 1 -> idx 0 (first gist)
    let out = state.handle_mouse(MouseInput::DoubleClick { col: 25, row: 1 }, &layout);
    assert_eq!(state.focus, FocusPane::Gist);
    assert_eq!(state.gist_index, 0);
    assert_eq!(out, KeyOutcome::PreviewDiff);
}

#[test]
fn click_in_pane_blank_focuses_without_selecting() {
    let mut state = state_with_gists();
    state.screen = Screen::List;
    state.focus = FocusPane::Local;
    state.gist_index = 0;
    let hit = PaneHit {
        rect: Rect::new(20, 0, 20, 4),
        offset: 0,
    };
    let layout = MouseLayout {
        gist: Some(hit),
        ..Default::default()
    };
    // row 0 is the top border (no row there): clicking the gist pane's blank/border area
    // switches focus to it but selects nothing.
    let out = state.handle_mouse(MouseInput::Click { col: 25, row: 0 }, &layout);
    assert_eq!(out, KeyOutcome::None);
    assert_eq!(state.focus, FocusPane::Gist);
    assert_eq!(state.gist_index, 0);
}

#[test]
fn click_off_list_screen_is_noop() {
    let mut state = state_with_gists();
    state.help.return_screen = Screen::List;
    state.screen = Screen::Help;
    let hit = PaneHit {
        rect: Rect::new(20, 0, 20, 10),
        offset: 0,
    };
    let layout = MouseLayout {
        gist: Some(hit),
        ..Default::default()
    };
    let before_screen = state.screen;
    let out = state.handle_mouse(MouseInput::Click { col: 25, row: 1 }, &layout);
    assert_eq!(out, KeyOutcome::None);
    assert_eq!(state.screen, before_screen);
}

#[test]
fn scroll_down_clamps_at_list_end() {
    // Only 1 item in local; scrolling down should clamp (no panic, no index change).
    let mut state = state_with_local_paths(&["a.rs"]);
    state.screen = Screen::List;
    state.focus = FocusPane::Local;
    state.local_index = 0;
    state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
    assert_eq!(state.local_index, 0);
}

#[test]
fn close_button_click_confirm_cancel_clears_pending() {
    // Close button on Screen::Confirm dispatches Esc, which cancels the pending action.
    // Using PendingAction::Download: Esc sets pending_action = None and screen = Screen::Diff.
    let mut state = state_with_gists();
    state.diff_text = "line1\nline2\nline3".into();
    state.screen = Screen::Confirm;
    state.pending_action = Some(PendingAction::Download);
    let layout = MouseLayout {
        close_button: Some(Rect::new(36, 0, 5, 1)),
        ..Default::default()
    };
    let out = state.handle_mouse(MouseInput::Click { col: 38, row: 0 }, &layout);
    assert_eq!(out, KeyOutcome::None);
    assert!(state.pending_action.is_none());
    assert_eq!(state.screen, Screen::Diff);
}

#[test]
fn close_button_click_create_description_cancels_not_types() {
    // Regression: close button while editing the create-description sub-state must cancel
    // (Esc), NOT append 'n' to the description field.  This test fails against the old
    // `KeyCode::Char('n')` dispatch and passes with `KeyCode::Esc`.
    let mut state = initial_state();
    state.screen = Screen::Confirm;
    state.pending_action = Some(PendingAction::Create {
        local_path: std::path::PathBuf::from("notes.txt"),
    });
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
    assert!(state.pending_action.is_none());
}

#[test]
fn wheel_step_gist_detail_moves_three() {
    // GistDetail content pane: one scroll-down tick must advance detail_scroll by 3.
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    // Use Comments focus so detail_nav moves detail_scroll (not the file cursor).
    state.detail.focus = DetailFocus::Comments;
    state.detail.comments = Some(Vec::new());
    assert_eq!(state.detail.scroll, 0);
    state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
    assert_eq!(state.detail.scroll, 3);
}

#[test]
fn wheel_step_help_body_moves_three() {
    // Help body (help_index_open = false): one scroll-down tick must advance help_scroll by 3.
    let mut state = initial_state();
    state.screen = Screen::Help;
    state.help.index_open = false;
    state.help.scroll = 0;
    state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
    assert_eq!(state.help.scroll, 3);
}

#[test]
fn wheel_step_help_index_moves_one() {
    // Help topic index (help_index_open = true): one scroll-down tick must move index by 1.
    let mut state = initial_state();
    state.screen = Screen::Help;
    state.help.index_open = true;
    state.help.index_sel = 0;
    state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
    assert_eq!(state.help.index_sel, 1);
}

#[test]
fn gists_click_selects_and_double_click_matches_enter() {
    let mut state = gists_screen_state(); // 2 groups, Screen::Gists
    state.gist_manager.index = 0;
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
    assert_eq!(state.gist_manager.index, 1);
    // Double-click activates the same row, exactly as Enter would.
    let mut by_key = state.clone();
    let key_out = by_key.handle_key(KeyCode::Enter);
    let by_mouse = state.handle_mouse(MouseInput::DoubleClick { col: 5, row: 2 }, &layout);
    assert_eq!(by_mouse, key_out);
    assert_eq!(by_mouse, KeyOutcome::OpenGistDetail);
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
    assert_eq!(state.pins.index, 1);
    let mut by_key = state.clone();
    let key_out = by_key.handle_key(KeyCode::Enter);
    let by_mouse = state.handle_mouse(MouseInput::DoubleClick { col: 5, row: 2 }, &layout);
    assert_eq!(by_mouse, key_out);
}

#[test]
fn revisions_click_selects_and_double_click_matches_enter() {
    let mut state = state_with_gists();
    state.screen = Screen::Revisions;
    state.revision.index = 0;
    state.revision.entries = Some(vec![
        crate::domain::GistRevision {
            version: "v2".into(),
            committed_at: "2026-06-10T00:00:00Z".into(),
            user: "u".into(),
            change_status: crate::domain::GistRevisionChangeStatus {
                total: 1,
                additions: 1,
                deletions: 0,
            },
        },
        crate::domain::GistRevision {
            version: "v1".into(),
            committed_at: "2026-06-01T00:00:00Z".into(),
            user: "u".into(),
            change_status: crate::domain::GistRevisionChangeStatus {
                total: 2,
                additions: 2,
                deletions: 0,
            },
        },
    ]);
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
    assert_eq!(state.revision.index, 1);
    let mut by_key = state.clone();
    let key_out = by_key.handle_key(KeyCode::Enter);
    let by_mouse = state.handle_mouse(MouseInput::DoubleClick { col: 5, row: 2 }, &layout);
    assert_eq!(by_mouse, key_out);
    assert_eq!(by_mouse, KeyOutcome::RevisionDiffIncremental);
}

#[test]
fn gist_detail_file_click_selects_and_double_previews() {
    let mut state = state_with_gists(); // g1: a.txt (0), b.txt (1)
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    state.detail.focus = DetailFocus::Comments; // start elsewhere to prove the focus switch
    let hit = PaneHit {
        rect: Rect::new(0, 0, 40, 10),
        offset: 0,
    };
    let layout = MouseLayout {
        detail_files: Some(hit),
        ..Default::default()
    };
    // Click the 2nd file row -> Files focus + cursor 1, but no open yet.
    let out = state.handle_mouse(MouseInput::Click { col: 5, row: 2 }, &layout);
    assert_eq!(out, KeyOutcome::None);
    assert_eq!(state.detail.focus, DetailFocus::Files);
    assert_eq!(state.detail.file_cursor, 1);
    // Double-click previews that file (there is no Enter for files).
    let out = state.handle_mouse(MouseInput::DoubleClick { col: 5, row: 2 }, &layout);
    assert_eq!(out, KeyOutcome::PreviewContent);
    assert_eq!(state.preview_request, Some(("g1".into(), "b.txt".into())));
}

#[test]
fn gist_detail_tab_click_switches_focus() {
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    state.detail.focus = DetailFocus::Files;
    // Header at chunks[0] y=0: content_x = 2, tabs_y = 2; " Files " (7), " Comments " (10 @ +10).
    let layout = MouseLayout {
        detail_tab_files: Some(Rect::new(2, 2, 7, 1)),
        detail_tab_comments: Some(Rect::new(12, 2, 10, 1)),
        ..Default::default()
    };
    // Click the Comments tab: switches focus and (comments unloaded) requests a fetch.
    let out = state.handle_mouse(MouseInput::Click { col: 14, row: 2 }, &layout);
    assert_eq!(state.detail.focus, DetailFocus::Comments);
    assert_eq!(out, KeyOutcome::FetchComments);
    // Click the Files tab back.
    let out = state.handle_mouse(MouseInput::Click { col: 4, row: 2 }, &layout);
    assert_eq!(state.detail.focus, DetailFocus::Files);
    assert_eq!(out, KeyOutcome::None);
}

#[test]
fn wheel_step_gist_detail_files_moves_one() {
    // The file list (Files tab) steps one file per wheel tick, not 3.
    let mut state = state_with_gists();
    state.screen = Screen::GistDetail;
    state.detail.gist_id = Some("g1".into());
    state.detail.focus = DetailFocus::Files;
    state.detail.file_cursor = 0;
    state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
    assert_eq!(state.detail.file_cursor, 1);
}

#[test]
fn comment_lines_count_matches_built_lines() {
    use crate::domain::GistComment;
    use crate::tui::render::comment_lines;
    use crate::tui::text::comment_lines_count;
    let theme = crate::tui::theme::Theme::for_choice(crate::config::ThemeChoice::Dark);
    let comments = vec![
        GistComment {
            author: "alice".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            body: "one line".into(),
        },
        GistComment {
            author: "bob".into(),
            created_at: "2026-01-02T00:00:00Z".into(),
            body: "two\nlines".into(),
        },
    ];
    // Each comment: 1 header + body.lines() + 1 blank.
    // alice: 1 + 1 + 1 = 3 ; bob: 1 + 2 + 1 = 4 ; total 7.
    assert_eq!(comment_lines_count(&comments), 7);
    assert_eq!(comment_lines(&comments, &theme, 0).len(), 7);
}

fn sample_comment(author: &str, body: &str) -> crate::domain::GistComment {
    crate::domain::GistComment {
        author: author.into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        body: body.into(),
    }
}

#[test]
fn apply_initial_comments_sets_window_and_requests_bottom_scroll() {
    use crate::tui::InitialComments;
    let mut s = crate::tui::initial_state();
    s.detail.gist_id = Some("g1".into());
    s.apply_initial_comments(
        "g1",
        Ok(InitialComments {
            comments: vec![sample_comment("a", "x")],
            total: 910,
            oldest_page: 31,
        }),
    );
    assert_eq!(s.detail.comments_total, Some(910));
    assert_eq!(s.detail.comments_loaded_oldest_page, 31);
    assert!(s.detail.comments_scroll_to_bottom);
    assert!(s.can_load_older_comments()); // page 31 > 1
    assert_eq!(s.detail.comments.as_ref().unwrap().len(), 1);
}

#[test]
fn apply_initial_comments_ignored_when_gist_changed() {
    use crate::tui::InitialComments;
    let mut s = crate::tui::initial_state();
    s.detail.gist_id = Some("g2".into());
    s.apply_initial_comments(
        "g1",
        Ok(InitialComments {
            comments: vec![],
            total: 0,
            oldest_page: 1,
        }),
    );
    assert!(s.detail.comments.is_none()); // stale response dropped
}

#[test]
fn apply_older_comments_prepends_and_compensates_scroll() {
    use crate::tui::InitialComments;
    let mut s = crate::tui::initial_state();
    s.detail.gist_id = Some("g1".into());
    s.apply_initial_comments(
        "g1",
        Ok(InitialComments {
            comments: vec![sample_comment("newer", "n")],
            total: 60,
            oldest_page: 2,
        }),
    );
    s.detail.scroll = 5;
    // One older comment = 1 header + 1 body + 1 blank = 3 lines prepended.
    s.apply_older_comments("g1", Ok(vec![sample_comment("older", "o")]));
    assert_eq!(s.detail.comments_loaded_oldest_page, 1);
    assert!(!s.can_load_older_comments()); // reached page 1
    assert_eq!(s.detail.comments.as_ref().unwrap()[0].author, "older"); // prepended
    assert_eq!(s.detail.scroll, 5 + 3); // viewport held in place
    assert!(!s.detail.comments_loading_more);
}

#[test]
fn can_load_older_false_while_loading_more() {
    use crate::tui::InitialComments;
    let mut s = crate::tui::initial_state();
    s.detail.gist_id = Some("g1".into());
    s.apply_initial_comments(
        "g1",
        Ok(InitialComments {
            comments: vec![sample_comment("a", "x")],
            total: 90,
            oldest_page: 3,
        }),
    );
    s.detail.comments_loading_more = true;
    assert!(!s.can_load_older_comments());
}

#[test]
fn m_key_loads_older_when_available() {
    use crate::tui::InitialComments;
    let mut s = crate::tui::initial_state();
    s.screen = Screen::GistDetail;
    s.detail.focus = DetailFocus::Comments;
    s.detail.gist_id = Some("g1".into());
    s.apply_initial_comments(
        "g1",
        Ok(InitialComments {
            comments: vec![sample_comment("a", "x")],
            total: 90,
            oldest_page: 3,
        }),
    );
    let out = s.handle_key(KeyCode::Char('m'));
    assert!(matches!(out, KeyOutcome::LoadOlderComments));
}

#[test]
fn m_key_noop_when_at_oldest_page() {
    use crate::tui::InitialComments;
    let mut s = crate::tui::initial_state();
    s.screen = Screen::GistDetail;
    s.detail.focus = DetailFocus::Comments;
    s.detail.gist_id = Some("g1".into());
    s.apply_initial_comments(
        "g1",
        Ok(InitialComments {
            comments: vec![sample_comment("a", "x")],
            total: 10,
            oldest_page: 1,
        }),
    );
    let out = s.handle_key(KeyCode::Char('m'));
    assert!(matches!(out, KeyOutcome::None));
}
