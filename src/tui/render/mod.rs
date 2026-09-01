//! Rendering façade: canvas setup, `ScreenVm` dispatch, and shared rendering helpers.

use super::keymap::{category_for_footer_key, Binding, Category};
use super::{
    screens::{
        config::render_config_vm as render_config, confirm::render_confirm_vm as render_confirm,
        detail::render_gist_detail_vm as render_detail, diff::render_diff_vm as render_diff,
        gists::render_gists_vm as render_gists, help::render_help_vm as render_help,
        list::render_list_vm as render_list, palette::render_palette_vm as render_palette,
        pins::render_pins_vm as render_pins, preview::render_preview_vm as render_preview,
        revisions::render_revisions_vm as render_revisions,
    },
    theme::Theme,
    *,
};
use crate::tui::screens::ScreenVm;
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
    Frame,
};
use similar::{ChangeTag, TextDiff};

#[derive(Debug, Default)]
pub(super) struct RenderFeedback {
    pub comments_max_scroll: Option<u16>,
}

pub(super) fn render(
    frame: &mut Frame,
    state: &AppState,
    layout: &mut MouseFrame,
    feedback: &mut RenderFeedback,
) {
    layout.clear();
    *feedback = RenderFeedback::default();
    // Paint the full canvas so every unfilled cell uses the theme background (no-op for dark
    // theme where bg=Reset, effective for light theme which sets a grey canvas).
    frame.render_widget(
        Block::default().style(state.settings.theme().base_style()),
        frame.area(),
    );
    // Pure presentation seam (issues #241 / #250): every screen paints from the view model.
    // Pin sync IO is never done here — only cache reads.
    let vm = super::build_view_model(state);
    render_screen_vm_with_feedback(frame, state, &vm.screen, &vm.chrome, layout, feedback);
    if let Some(ref msg) = vm.chrome.bg_task_msg {
        render_loading_overlay(frame, msg, vm.chrome.spinner_frame, &state.settings.theme());
    }
}

/// Paints one `ScreenVm`. Shared by `render()` (the primary per-frame path) and
/// `render_palette_vm` (the palette's already-built background, issue #272) — one seam, two
/// real callers, so a new `Screen` variant only needs wiring here once.
pub(crate) fn render_screen_vm_with_feedback(
    frame: &mut Frame,
    state: &AppState,
    screen: &ScreenVm,
    chrome: &crate::tui::view_model::ChromeVm,
    layout: &mut MouseFrame,
    feedback: &mut RenderFeedback,
) {
    match screen {
        ScreenVm::List(list) => render_list(frame, state, list, chrome, layout),
        ScreenVm::Gists(gists) => render_gists(frame, state, gists, chrome, layout),
        ScreenVm::GistDetail(detail) => {
            render_detail(frame, state, detail, chrome, layout, feedback)
        }
        ScreenVm::Revisions(revs) => render_revisions(frame, state, revs, chrome, layout),
        ScreenVm::Config(config) => render_config(frame, state, config, chrome, layout),
        ScreenVm::Diff(diff) => render_diff(frame, state, diff, chrome, layout),
        ScreenVm::Preview(preview) => render_preview(frame, state, preview, chrome, layout),
        ScreenVm::Pins(pins) => render_pins(frame, state, pins, chrome, layout),
        ScreenVm::Confirm(confirm) => render_confirm(frame, state, confirm, chrome, layout),
        ScreenVm::Help(help) => render_help(frame, state, help, chrome, layout),
        ScreenVm::Palette(palette) => {
            render_palette(frame, state, palette, chrome, layout, feedback)
        }
    }
}

#[cfg(test)]
pub(crate) fn render_screen_vm(
    frame: &mut Frame,
    state: &AppState,
    screen: &ScreenVm,
    chrome: &crate::tui::view_model::ChromeVm,
    layout: &mut MouseFrame,
) {
    render_screen_vm_with_feedback(
        frame,
        state,
        screen,
        chrome,
        layout,
        &mut RenderFeedback::default(),
    );
}

mod chrome;
mod diff_view;
pub(crate) mod labels;
pub(crate) mod list_pane;
pub(crate) mod text_fit;

pub(crate) use chrome::*;
pub(crate) use diff_view::*;
pub(crate) use labels::*;
pub(crate) use text_fit::*;

#[cfg(test)]
mod tests {
    use super::*;

    use ratatui::{backend::TestBackend, Terminal};

    pub(super) fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn render_screen_vm_top_bar_marks_a_shortcut_that_itself_cannot_fit() {
        let rows = render_rows(&initial_state(), 6, 8);
        let bar = rows[0].trim_end();
        assert!(
            bar.contains('…'),
            "clipped last shortcut has no ellipsis: {bar:?}"
        );
        assert!(!bar.contains("Help"), "clipped tail still visible: {bar:?}");
    }

    pub(super) fn render_state(state: &AppState) -> String {
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let vm = super::super::build_view_model(state);
        let mut layout = MouseFrame::default();
        terminal
            .draw(|frame| render_screen_vm(frame, state, &vm.screen, &vm.chrome, &mut layout))
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    #[test]
    fn render_screen_vm_list_paints_top_bar() {
        let state = initial_state();
        let text = render_state(&state);
        assert!(text.contains("gistui"));
        assert!(text.contains("(g)ists"));
        assert!(text.contains("(P)ins"));
        assert!(text.contains("(C)onfig"));
        assert!(text.contains("(?)Help"));
    }

    #[test]
    fn render_screen_vm_top_bar_does_not_cut_the_app_name_at_forty_columns() {
        let rows = render_rows(&initial_state(), 40, 12);
        let bar = rows[0].trim_end();
        assert!(
            !bar.contains("gist(g)"),
            "brand overwritten mid-word: {bar:?}"
        );
        assert!(
            !bar.contains("gistui"),
            "brand should yield to the shortcuts: {bar:?}"
        );
        assert!(
            bar.contains("(g)ists")
                && bar.contains("(P)ins")
                && bar.contains("(C)onfig")
                && bar.contains("(?)Help"),
            "shortcuts missing from the 40-column bar: {bar:?}"
        );
    }

    #[test]
    fn render_screen_vm_top_bar_keeps_the_app_name_at_sixty_columns() {
        let rows = render_rows(&initial_state(), 60, 12);
        let bar = rows[0].trim_end();
        assert!(
            bar.contains("gistui"),
            "brand missing at 60 columns: {bar:?}"
        );
        assert!(
            bar.contains("(g)ists")
                && bar.contains("(P)ins")
                && bar.contains("(C)onfig")
                && bar.contains("(?)Help"),
            "shortcuts missing from the 60-column bar: {bar:?}"
        );
        assert!(
            !bar.contains("gist(g)"),
            "brand overwritten mid-word: {bar:?}"
        );
    }

    #[test]
    fn render_screen_vm_command_palette_no_matches_shows_empty_state() {
        let mut state = initial_state();
        state.open_palette_command();
        state
            .palette_mut()
            .unwrap()
            .query
            .set("zzz_no_such_command");
        assert!(state.palette_visible_items().is_empty());
        let text = render_state(&state);
        assert!(text.contains("no matches"));
    }

    #[test]
    fn render_screen_vm_list_title_keeps_state_over_cwd() {
        let mut state = initial_state();
        state.cwd = std::path::PathBuf::from("/cwd/some-org/some-project");
        state.locals = vec![
            crate::domain::LocalCandidate {
                path: state.cwd.join("notes.md"),
                modified: None,
            },
            crate::domain::LocalCandidate {
                path: state.cwd.join("a.txt"),
                modified: None,
            },
        ];
        state.local_filter_query = "md".into();
        state.anchor = FocusPane::Local;

        // 137 columns as reported; the Local pane gets 40% of that.
        let backend = TestBackend::new(137, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let vm = super::super::build_view_model(&state);
        let mut layout = MouseFrame::default();
        terminal
            .draw(|frame| render_screen_vm(frame, &state, &vm.screen, &vm.chrome, &mut layout))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());

        assert!(
            text.contains("[1] Local (1/2) ⚑"),
            "local title missing the anchor: {text}"
        );
        assert!(
            text.contains("· sort:match · /md ·"),
            "local title missing state segments: {text}"
        );
        // The cwd is what shortened: only its tail survived, behind an ellipsis.
        assert!(text.contains("…/some-project"), "cwd not elided: {text}");
    }

    #[test]
    fn render_screen_vm_list_title_drops_the_pane_name_when_narrow() {
        let mut state = initial_state();
        state.cwd = std::path::PathBuf::from("/cwd/some-org/some-project");
        state.locals = vec![crate::domain::LocalCandidate {
            path: state.cwd.join("notes.md"),
            modified: None,
        }];
        state.anchor = FocusPane::Local;

        // 70 columns: the Local pane's 40% share cannot hold `[1] Local (1) ⚑ · sort:match`.
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let vm = super::super::build_view_model(&state);
        let mut layout = MouseFrame::default();
        terminal
            .draw(|frame| render_screen_vm(frame, &state, &vm.screen, &vm.chrome, &mut layout))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());

        assert!(
            text.contains("[1] (1) ⚑"),
            "narrow local title lost the anchor: {text}"
        );
        assert!(
            text.contains("· sort:match"),
            "narrow local title lost the sort mode: {text}"
        );
        assert!(
            !text.contains("[1] Local"),
            "pane name should have given way: {text}"
        );
    }

    fn gist_file(filename: &str, description: &str) -> crate::domain::GistFile {
        crate::domain::GistFile {
            description: description.into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
            ..crate::domain::GistFile::fixture("abc123def", filename)
        }
    }

    pub(super) fn render_state_size(state: &AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let vm = super::super::build_view_model(state);
        let mut layout = MouseFrame::default();
        terminal
            .draw(|frame| render_screen_vm(frame, state, &vm.screen, &vm.chrome, &mut layout))
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    #[test]
    fn render_screen_vm_list_row_marks_clipped_description() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![gist_file(
            "MicrosoftDateTimeJsonConverter.cs",
            "System.Text.Json deserialize legacy JSON data TAILMARKER",
        )];
        let text = render_state_size(&state, 137, 20);
        assert!(
            text.contains("MicrosoftDateTimeJsonConverter.cs"),
            "filename missing: {text}"
        );
        assert!(text.contains('…'), "clipped row has no ellipsis: {text}");
        assert!(
            !text.contains("TAILMARKER"),
            "clipped tail still visible: {text}"
        );
    }

    #[test]
    fn render_screen_vm_list_row_does_not_split_a_wide_character() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![gist_file(
            "日本語テストファイル名前がとても長いです.txt",
            "desc",
        )];
        let text = render_state_size(&state, 80, 16);
        assert!(text.contains('…'), "wide-char row has no ellipsis: {text}");
        assert!(
            !text.contains("です.txt"),
            "wide-char tail still visible (or a split glyph leaked the suffix): {text}"
        );
    }

    #[test]
    fn render_screen_vm_list_row_leaves_a_fitting_label_untouched() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![gist_file("a.txt", "short")];
        let text = render_state_size(&state, 137, 20);
        assert!(
            text.contains("a.txt — short"),
            "fitting label missing: {text}"
        );
        assert!(!text.contains('…'), "fitting label was clipped: {text}");
    }

    #[test]
    fn render_screen_vm_gists_row_marks_clipped_description() {
        let mut state = initial_state();
        state.screen = Screen::Gists(Box::default());
        state.gist_catalog.owned = vec![gist_file(
            "file.txt",
            "a very long gist description that must not survive a narrow manager row TAILMARKER",
        )];
        let text = render_state_size(&state, 60, 16);
        assert!(
            text.contains('…'),
            "clipped gist row has no ellipsis: {text}"
        );
        assert!(
            !text.contains("TAILMARKER"),
            "clipped gist tail still visible: {text}"
        );
    }

    #[test]
    fn render_screen_vm_pins_row_marks_clipped_path() {
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
        state.pinned = vec![crate::domain::PinnedMapping {
            local_path: std::path::PathBuf::from(
                "/cwd/very/deeply/nested/project/with/a/long/path/config.json",
            ),
            gist_id: "abc123def456".into(),
            gist_filename: "config.json".into(),
            direction: None,
            last_seen_hash: None,
        }];
        let text = render_state_size(&state, 50, 16);
        assert!(
            text.contains('…'),
            "clipped pin row has no ellipsis: {text}"
        );
        assert!(
            !text.contains("config.json"),
            "clipped pin tail still visible: {text}"
        );
    }

    #[test]
    fn render_screen_vm_gists_does_not_panic() {
        let mut state = initial_state();
        state.screen = Screen::Gists(Box::default());
        render_state(&state);
    }

    #[test]
    fn render_screen_vm_gist_detail_does_not_panic() {
        let mut state = initial_state();
        state.screen = Screen::GistDetail(Box::default());
        render_state(&state);
    }

    #[test]
    fn render_gist_detail_metadata_fits_at_eighty_columns() {
        let mut state = initial_state();
        state.screen = Screen::GistDetail(Box::default());
        state.gist_catalog.owned = vec![crate::domain::GistFile {
            content_type: Some("text/plain".into()),
            size: 1_536,
            ..gist_file("notes.txt", "metadata")
        }];
        state.detail_mut().unwrap().gist_id = Some("abc123def".into());
        state
            .gist_catalog
            .comment_counts
            .insert("abc123def".into(), 3);

        let text = render_state_size(&state, 80, 24);
        assert!(text.contains("notes.txt · 1.5 KiB · text/plain"), "{text}");
        assert!(text.contains("Files (1): 1.5 KiB total"), "{text}");
        assert!(text.contains("Comments (3)"), "{text}");
    }

    #[test]
    fn render_screen_vm_revisions_does_not_panic() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::default());
        render_state(&state);
    }

    #[test]
    fn render_screen_vm_config_paints_settings_panel() {
        let mut state = initial_state();
        state.screen = Screen::Config(Box::default());
        let text = render_state(&state);
        assert!(text.contains("Settings"));
    }

    #[test]
    fn render_screen_vm_diff_does_not_panic() {
        let mut state = initial_state();
        state.screen = Screen::Diff(Box::default());
        render_state(&state);
    }

    #[test]
    fn render_screen_vm_preview_does_not_panic() {
        let mut state = initial_state();
        state.screen = Screen::Preview(Box::default());
        render_state(&state);
    }

    #[test]
    fn render_screen_vm_pins_paints_empty_state_message() {
        let mut state = initial_state();
        state.screen = Screen::Pins(Box::default());
        let text = render_state(&state);
        assert!(text.contains("No pinned mappings yet"));
    }

    /// A key and the verb it performs must never end up on different lines: at 80 columns the
    /// upload confirm's five keys do not fit one row, so they have to be packed onto further
    /// rows rather than left to word-wrap (which split `e` from `edit first` and cost the
    /// sizing pass a row it had not counted).
    #[test]
    fn render_screen_vm_confirm_packs_keys_that_cannot_share_one_row() {
        let mut state = initial_state();
        state.screen = Screen::Confirm(Box::default());
        crate::tui::test_support::set_pending(
            &mut state,
            crate::tui::PendingAction::Upload {
                gist_id: "g1".into(),
                filename: "settings.json".into(),
                local_path: std::path::PathBuf::from("settings.json"),
            },
        );
        let rows = render_rows(&state, 80, 24);
        for hint in [
            "y  upload",
            "n  cancel",
            "e  edit first",
            "p  pretty [off]",
            "s  sort [off]",
        ] {
            assert!(
                rows.iter().any(|r| r.contains(hint)),
                "{hint:?} was split or clipped at 80 columns: {rows:#?}"
            );
        }
    }

    /// The description editor wraps and the modal grows with it, so the caret stays on screen
    /// instead of typing past a truncated line.
    #[test]
    fn render_screen_vm_confirm_input_grows_with_a_long_description() {
        let mut state = initial_state();
        state.screen = Screen::Confirm(Box::default());
        crate::tui::test_support::set_pending(
            &mut state,
            crate::tui::PendingAction::Create {
                local_path: std::path::PathBuf::from("notes.txt"),
            },
        );
        state.editing_description = true;
        state.description_input =
            "a deliberately long description that will not fit on a single line at all".into();
        let rows = render_rows(&state, 80, 24);
        assert!(
            rows.iter().any(|r| r.contains("single line at all")),
            "the tail of the description was truncated: {rows:#?}"
        );
        assert!(
            rows.iter().any(|r| r.contains("Esc  cancel")),
            "the key row was pushed out of the grown modal: {rows:#?}"
        );
    }

    #[test]
    fn render_screen_vm_confirm_separates_the_question_from_its_keys() {
        let mut state = initial_state();
        state.screen = Screen::Confirm(Box::default());
        crate::tui::test_support::set_pending(
            &mut state,
            crate::tui::PendingAction::Delete {
                gist_id: "abc".into(),
                label: "my config".into(),
            },
        );
        let rows = render_rows(&state, 100, 40);
        let body: Vec<&str> = rows
            .iter()
            .map(|r| r.trim_end())
            .skip_while(|r| !r.contains("╭ Delete "))
            .take_while(|r| !r.contains('╯'))
            .collect();
        // Every row carries the backdrop's own border columns, so "blank" means "nothing but
        // borders and spaces".
        let blank = |row: &str| row.chars().all(|c| c == '│' || c == ' ');
        // Title row, blank padding, question, consequence, blank separator, key row.
        assert!(body[0].contains(" Delete "), "spaced title: {body:?}");
        assert!(blank(body[1]), "top padding: {body:?}");
        assert!(
            body[2].contains("Permanently delete"),
            "question row: {body:?}"
        );
        assert!(body[3].contains("abc"), "consequence row: {body:?}");
        assert!(blank(body[4]), "blank separator: {body:?}");
        // Destructive: cancel is offered before the key that goes through with it.
        let keys = body[5];
        assert!(
            keys.find("cancel") < keys.find("delete"),
            "cancel must lead a destructive confirm: {keys:?}"
        );
    }

    #[test]
    fn render_screen_vm_confirm_paints_without_top_bar() {
        let mut state = initial_state();
        state.screen = Screen::Confirm(Box::default());
        let text = render_state(&state);
        // Confirm is the one screen that skips the persistent top bar (full-bleed modal).
        assert!(!text.contains("gistui"));
    }

    #[test]
    fn render_screen_vm_help_does_not_panic() {
        let mut state = initial_state();
        state.screen = Screen::Help(Box::default());
        render_state(&state);
    }

    #[test]
    fn render_help_body_rewraps_instead_of_clipping_at_eighty_columns() {
        let mut state = initial_state();
        state.screen = Screen::Help(Box::default());
        let text = render_state_size(&state, 80, 60);
        assert!(
            text.contains("pin/upload/delete"),
            "help body clipped mid-word at 80 columns: {text}"
        );
    }

    fn pane_content_indent(row: &str) -> usize {
        let inner = row
            .trim_end()
            .strip_prefix('│')
            .unwrap_or(row)
            .strip_suffix('│')
            .unwrap_or(row);
        inner.len() - inner.trim_start().len()
    }

    fn buffer_rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
        let width = buffer.area().width;
        let height = buffer.area().height;
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn render_rows(state: &AppState, width: u16, height: u16) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let vm = super::super::build_view_model(state);
        let mut layout = MouseFrame::default();
        terminal
            .draw(|frame| render_screen_vm(frame, state, &vm.screen, &vm.chrome, &mut layout))
            .unwrap();
        buffer_rows(terminal.backend().buffer())
    }

    #[test]
    fn render_diff_footer_keeps_whole_hints_including_leave_at_eighty_columns() {
        let mut state = initial_state();
        state.screen = Screen::Diff(Box::default());
        let rows = render_rows(&state, 80, 24);
        let footer = rows
            .last()
            .expect("diff screen has a footer row")
            .trim_end();
        let above = rows[rows.len() - 2].trim_end();
        assert!(
            above.contains('─'),
            "footer should be one trimmed row, not wrapped:\n{above}\n{footer}"
        );
        assert!(
            footer.contains("Esc/q") && footer.contains("back"),
            "leave key missing from narrow diff footer: {footer:?}"
        );
        for item in footer.split('·').map(str::trim).filter(|s| !s.is_empty()) {
            assert!(
                !item.ends_with('…') || item == footer.trim(),
                "mid-hint clip in narrow diff footer ({item:?}): {footer:?}"
            );
        }
    }

    #[test]
    fn key_dense_screen_footers_keep_whole_hints_at_eighty_columns() {
        let mut list = initial_state();
        let list_text = render_rows(&list, 80, 24).join("\n");
        assert!(list_text.contains("Enter diff") && list_text.contains("Esc/q back"));

        let mut pins = initial_state();
        pins.screen = Screen::Pins(Box::default());
        let pins_text = render_rows(&pins, 80, 24).join("\n");
        assert!(pins_text.contains("✓ synced") && pins_text.contains("Esc/q back"));

        list.screen = Screen::Gists(Box::default());
        let gists_text = render_rows(&list, 80, 24).join("\n");
        assert!(gists_text.contains("Enter detail") && gists_text.contains("Esc/q back"));
    }

    #[test]
    fn render_comments_keep_hanging_indent_at_eighty_columns() {
        let mut state = initial_state();
        state.screen = Screen::GistDetail(Box::default());
        state.gist_catalog.owned = vec![crate::domain::GistFile::fixture("g1", "a.txt")];
        if let Some(d) = state.detail_mut() {
            d.gist_id = Some("g1".into());
            d.focus = DetailFocus::Comments;
            d.comments = Some(vec![crate::domain::GistComment {
                author: "alice".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                body: "- a list item that is long enough to wrap at eighty columns and must keep hanging indent WRAPTAIL"
                    .into(),
            }]);
            d.comments_loaded_oldest_page = 1;
        }
        let rows = render_rows(&state, 80, 24);
        let first = rows
            .iter()
            .find(|row| row.contains("list item that is long"))
            .unwrap_or_else(|| panic!("comment body missing: {rows:?}"));
        let cont = rows
            .iter()
            .find(|row| row.contains("WRAPTAIL"))
            .unwrap_or_else(|| panic!("wrapped tail missing: {rows:?}"));
        let first_indent = pane_content_indent(first);
        let cont_indent = pane_content_indent(cont);
        assert_eq!(
            first_indent, cont_indent,
            "wrapped comment lost hanging indent\n{first}\n{cont}"
        );
        assert!(
            first_indent >= 2,
            "comment body should keep its indent: {first:?}"
        );
    }

    #[test]
    fn render_screen_vm_all_screens_paint_at_eighty_columns() {
        let screens = [
            Screen::List,
            Screen::Gists(Box::default()),
            Screen::GistDetail(Box::default()),
            Screen::Revisions(Box::default()),
            Screen::Config(Box::default()),
            Screen::Diff(Box::default()),
            Screen::Preview(Box::default()),
            Screen::Pins(Box::default()),
            Screen::Confirm(Box::default()),
            Screen::Help(Box::default()),
        ];
        for screen in screens {
            let mut state = initial_state();
            state.screen = screen.clone();
            let _ = render_state_size(&state, 80, 24);
        }
        let mut palette = initial_state();
        palette.open_palette_menu(None);
        let _ = render_state_size(&palette, 80, 24);
    }

    #[test]
    fn render_screen_vm_palette_paints_menu_title_over_background() {
        let mut state = initial_state();
        state.open_palette_menu(None);
        assert!(state.screen.is_palette());
        let text = render_state(&state);
        assert!(text.contains("Menu"));
        // The origin screen (List) still paints as the palette's background.
        assert!(text.contains("gistui"));
    }
}
