//! `Screen::Preview` — key handling, view-model, paint, and palette items colocated in one
//! file (issue #287, Phase 2).

use crate::tui::view_model::{ChromeVm, PreviewVm};
use crate::tui::{AppState, HelpTopic, KeyOutcome, MouseLayout};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Padding, Paragraph, Wrap},
    Frame,
};

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::List;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    3
}

impl AppState {
    pub(crate) fn handle_key_preview(&mut self, code: KeyCode) -> KeyOutcome {
        // One-shot: any key dismisses a lingering status (e.g. a previous "fetch failed: …"); the
        // run_loop refresh helper may set a fresh one afterwards.
        self.status = None;
        match code {
            // In the preview, q and Esc return to wherever it was launched from (the list, or
            // the gist detail view) — never an accidental app exit.
            KeyCode::Char('q') | KeyCode::Esc => {
                self.leave();
            }
            KeyCode::Char('R') => {
                let Some(p) = self.preview() else {
                    return KeyOutcome::None;
                };
                let Some((gist_id, filename)) = p.gist_key.clone() else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::RefreshPreview {
                    file: crate::domain::GistFileRef::id_name(gist_id, filename),
                };
            }
            KeyCode::Char('w') => self.preview_wrap = !self.preview_wrap,
            KeyCode::Char('y') => {
                let gist_id = self
                    .preview()
                    .and_then(|p| p.gist_key.as_ref().map(|(id, _)| id.clone()))
                    .or_else(|| self.context_gist_id());
                let Some(gist_id) = gist_id else {
                    return KeyOutcome::None;
                };
                return KeyOutcome::CopyGistUrl { gist_id };
            }
            KeyCode::Char('Y') => return KeyOutcome::CopyPreviewContent,
            _ => {}
        }
        KeyOutcome::None
    }
}

/// Preview body — usable under Palette-over-Preview as well.
pub(crate) fn build_preview_vm(state: &AppState) -> PreviewVm {
    let p = state.preview().cloned().unwrap_or_default();
    let hints = if state.preview_wrap {
        "↑↓ PgUp/Dn scroll  ·  w wrap [on]  ·  y/Y copy url/content  ·  R refresh  ·  Esc/q back"
    } else {
        "↑↓←→ PgUp/Dn scroll  ·  w wrap [off]  ·  y/Y copy url/content  ·  R refresh  ·  Esc/q back"
    };
    let (footer, footer_colored) = crate::tui::footer_with_status(state.status.as_deref(), hints);
    let ext = p
        .gist_key
        .as_ref()
        .and_then(|(_, filename)| crate::tui::view_model::file_ext(filename));
    PreviewVm {
        title: p.title,
        body: p.text,
        footer,
        footer_colored,
        wrap: state.preview_wrap,
        scroll: p.scroll,
        hscroll: p.hscroll,
        syntax_highlight: state.syntax_highlight,
        ext,
    }
}

/// The preview body as per-line span vectors: syntax-highlighted when the feature is enabled and
/// the file type is known, otherwise one plain span per line.
fn preview_line_spans(
    body: &str,
    syntax_highlight: bool,
    ext: Option<&str>,
    theme: &crate::tui::Theme,
) -> Vec<Vec<Span<'static>>> {
    let lines: Vec<String> = body.lines().map(str::to_string).collect();
    match (syntax_highlight, ext) {
        (true, Some(ext)) => crate::tui::highlight::highlight_buffer(ext, &lines, theme),
        _ => lines.into_iter().map(|l| vec![Span::raw(l)]).collect(),
    }
}

pub(crate) fn render_preview_vm(
    frame: &mut Frame,
    state: &AppState,
    preview: &PreviewVm,
    chrome: &ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    let footer_lines = if preview.footer_colored {
        1
    } else {
        crate::tui::wrap_line_count(&preview.footer, area.width.saturating_sub(2)).max(1)
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(footer_lines)])
        .split(area);

    // When wrapping, horizontal scroll is meaningless — pin the x offset to 0 so long lines
    // wrap into view instead of being scrolled off-screen.
    let block = Block::default()
        .title(crate::tui::render::fit_block_title(
            &preview.title,
            chunks[0].width,
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(state.theme.base_style())
        .padding(Padding::horizontal(1));
    let line_spans = preview_line_spans(
        &preview.body,
        preview.syntax_highlight,
        preview.ext.as_deref(),
        &state.theme,
    );
    let total_lines = line_spans.len();
    let paragraph = if preview.wrap {
        // Wrapping needs the full line set; vertical scroll goes through Paragraph (no hscroll).
        let body = Text::from(line_spans.into_iter().map(Line::from).collect::<Vec<_>>());
        Paragraph::new(body)
            .style(state.theme.base_style())
            .scroll((preview.scroll, 0))
            .wrap(Wrap { trim: false })
            .block(block)
    } else {
        // Manual horizontal + vertical scroll mirrors diff_view, avoiding the styled-line
        // redraw artifacts that Paragraph::scroll leaves on coloured spans.
        let visible: Vec<Line> = line_spans
            .into_iter()
            .map(|spans| crate::tui::apply_hscroll_spans(spans, preview.hscroll as usize))
            .skip(preview.scroll as usize)
            .collect();
        Paragraph::new(Text::from(visible))
            .style(state.theme.base_style())
            .block(block)
    };
    frame.render_widget(paragraph, chunks[0]);
    // Only the non-wrap path keeps a 1:1 line↔row mapping for an accurate thumb; under soft
    // wrapping the logical line count diverges from rendered rows, so skip the scrollbar there.
    if !preview.wrap {
        crate::tui::render_text_scrollbar(frame, chunks[0], total_lines, preview.scroll as usize);
    }
    crate::tui::render_footer(
        frame,
        chunks[1],
        "",
        &preview.footer,
        preview.footer_colored,
        &state.theme,
        layout,
    );
    if chrome.mouse_enabled {
        layout.close_button = Some(crate::tui::render_close_button(frame, area, &state.theme));
    }
}

pub(crate) fn preview_palette_items(_state: &AppState) -> Vec<crate::tui::palette::PaletteItem> {
    use crate::tui::palette::key_item;
    vec![
        key_item("R", "Refresh content", KeyCode::Char('R'), true),
        key_item("w", "Toggle line wrap", KeyCode::Char('w'), true),
        key_item("y", "Copy gist URL", KeyCode::Char('y'), true),
        key_item("Y", "Copy file content", KeyCode::Char('Y'), true),
        key_item("q", "Back", KeyCode::Char('q'), true),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::*;

    use crate::tui::tests::state_with_gists;

    fn preview_ref(state: &AppState) -> &PreviewState {
        state.preview().expect("expected Screen::Preview")
    }

    #[test]
    fn preview_w_toggles_line_wrapping() {
        let mut state = initial_state();
        state.screen = Screen::Preview(Box::default());
        assert!(!state.preview_wrap);
        state.handle_key(KeyCode::Char('w'));
        assert!(state.preview_wrap);
        state.handle_key(KeyCode::Char('w'));
        assert!(!state.preview_wrap);
    }

    #[test]
    fn preview_key_clears_lingering_status_for_one_shot_display() {
        let mut state = initial_state();
        state.screen = Screen::Preview(Box::default());
        state.status = Some("fetch failed: boom".into());
        state.handle_key(KeyCode::Down); // any key
        assert_eq!(state.status, None);
    }

    #[test]
    fn preview_scrolls_with_arrows() {
        let mut state = initial_state();
        state.enter_preview("t".into(), "l1\nl2\nl3".into(), None);
        state.handle_key(KeyCode::Down);
        assert_eq!(preview_ref(&state).scroll, 1);
        state.handle_key(KeyCode::Up);
        assert_eq!(preview_ref(&state).scroll, 0);
    }

    #[test]
    fn page_keys_jump_by_ten_clamped_to_bounds() {
        let mut state = initial_state();
        // 30 lines → bottom is line 29 (count - 1).
        let text = (0..30)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        state.enter_preview("t".into(), text, None);
        state.handle_key(KeyCode::PageDown);
        assert_eq!(preview_ref(&state).scroll, 10);
        // A second page-down would reach 20; a third clamps at the 29-line bottom, not 30.
        state.handle_key(KeyCode::PageDown);
        state.handle_key(KeyCode::PageDown);
        assert_eq!(preview_ref(&state).scroll, 29);
        state.handle_key(KeyCode::PageUp);
        assert_eq!(preview_ref(&state).scroll, 19);
    }

    #[test]
    fn preview_y_copies_url_and_capital_y_copies_content() {
        let mut state = state_with_gists();
        state.screen = Screen::Preview(Box::default());
        assert!(matches!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::CopyGistUrl { .. }
        ));
        assert_eq!(
            state.handle_key(KeyCode::Char('Y')),
            KeyOutcome::CopyPreviewContent
        );
    }

    #[test]
    fn top_bar_gists_click_opens_gist_manager_from_any_screen() {
        let mut state = state_with_gists();
        state.screen = Screen::Preview(Box::default()); // arbitrary screen that has no 'g' binding of its own
        let layout = MouseLayout {
            top_bar_gists: Some(Rect::new(10, 0, 7, 1)),
            ..Default::default()
        };
        let out = state.handle_mouse(MouseInput::Click { col: 12, row: 0 }, &layout);
        assert!(state.screen.is_gists());
        assert_eq!(out, KeyOutcome::None);
    }

    #[test]
    fn top_bar_config_click_opens_settings_from_any_screen() {
        let mut state = state_with_gists();
        state.screen = Screen::Preview(Box::default());
        let layout = MouseLayout {
            top_bar_config: Some(Rect::new(28, 0, 8, 1)),
            ..Default::default()
        };
        let out = state.handle_mouse(MouseInput::Click { col: 30, row: 0 }, &layout);
        assert!(state.screen.is_config());
        assert!(state.nav_stack.last().is_some_and(Screen::is_preview));
        assert_eq!(out, KeyOutcome::None);
    }

    #[test]
    fn top_bar_config_click_while_already_on_config_does_not_trap_keyboard_exit() {
        let mut state = state_with_gists();
        state.screen = Screen::Preview(Box::default());
        let layout = MouseLayout {
            top_bar_config: Some(Rect::new(28, 0, 8, 1)),
            ..Default::default()
        };
        state.handle_mouse(MouseInput::Click { col: 30, row: 0 }, &layout);
        assert!(state.screen.is_config());
        assert!(state.nav_stack.last().is_some_and(Screen::is_preview));

        // Second click on Config while already there must not overwrite return_screen.
        let out = state.handle_mouse(MouseInput::Click { col: 30, row: 0 }, &layout);
        assert!(state.screen.is_config());
        assert!(state.nav_stack.last().is_some_and(Screen::is_preview));
        assert_eq!(out, KeyOutcome::None);

        state.handle_key(KeyCode::Esc);
        assert!(state.screen.is_preview());
    }

    #[test]
    fn top_bar_help_click_opens_help_and_remembers_return_screen_from_any_screen() {
        let mut state = state_with_gists();
        state.screen = Screen::Preview(Box::default());
        let layout = MouseLayout {
            top_bar_help: Some(Rect::new(30, 0, 7, 1)),
            ..Default::default()
        };
        let out = state.handle_mouse(MouseInput::Click { col: 32, row: 0 }, &layout);
        assert!(state.screen.is_help());
        assert!(state.nav_stack.last().is_some_and(Screen::is_preview));
        assert_eq!(out, KeyOutcome::None);
    }
}
