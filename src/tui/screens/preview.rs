//! `Screen::Preview` — key handling, view-model, paint, palette items, and apply handlers
//! colocated in one file (issue #287, Phase 2; issue #383).

use crate::tui::bg::LoopFlow;
use crate::tui::view_model::{ChromeVm, PreviewVm};
use crate::tui::{AppState, HelpTopic, HitTarget, KeyOutcome, MouseFrame};
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

/// Stage a content preview. A cache hit enters Preview immediately; a miss returns fetch data.
pub(crate) fn stage_preview_content(
    state: &mut AppState,
    mut file: crate::domain::GistFileRef,
) -> Option<(crate::domain::GistFileRef, String)> {
    let key = file.cache_key();
    if let Some(content) = state.gist_content_cache.get(&key).cloned() {
        let title = state.preview_title(&file.gist_id, &file.filename);
        state.enter_preview(title, content, Some(key));
        return None;
    }
    if file.raw_url.is_none() {
        file.raw_url = state.gist_file_raw_url(&file.gist_id, &file.filename);
    }
    let title = state.preview_title(&file.gist_id, &file.filename);
    Some((file, title))
}

/// Invalidate cached content and build the fetch payload for refreshing a preview.
pub(crate) fn stage_refresh_preview(
    state: &mut AppState,
    mut file: crate::domain::GistFileRef,
) -> (crate::domain::GistFileRef, String) {
    state.gist_content_cache.remove(&file.cache_key());
    if file.raw_url.is_none() {
        file.raw_url = state.gist_file_raw_url(&file.gist_id, &file.filename);
    }
    let title = state.preview_title(&file.gist_id, &file.filename);
    (file, title)
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
                    entry: self
                        .defer_replacement()
                        .unwrap_or_else(|| self.defer_entry()),
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
        body: p.body.text,
        footer,
        footer_colored,
        wrap: state.preview_wrap,
        scroll: p.body.scroll,
        hscroll: p.body.hscroll,
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
    layout: &mut MouseFrame,
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
        crate::tui::keymap::for_screen(&state.screen),
        &state.theme,
    );
    if chrome.mouse_enabled {
        let close = crate::tui::render_close_button(frame, area, &state.theme);
        layout.register(HitTarget::Close, close);
    }
}

/// `PreviewContent` outcome: cache the fetched content and open the read-only preview.
pub(crate) fn on_preview_content(
    state: &mut AppState,
    entry: crate::tui::DeferredEntry,
    result: std::result::Result<String, String>,
    file: crate::domain::GistFileRef,
    preview_title: String,
) -> LoopFlow {
    match result {
        Ok(content) => {
            let key = file.cache_key();
            state
                .gist_content_cache
                .insert(key.clone(), content.clone());
            state.open_deferred(
                entry,
                crate::tui::Screen::Preview(Box::new(crate::tui::PreviewState {
                    title: preview_title,
                    body: crate::tui::ScrollBody {
                        text: content,
                        ..crate::tui::ScrollBody::default()
                    },
                    gist_key: Some(key),
                })),
            );
        }
        Err(error) => state.set_status(format!("fetch failed: {error}")),
    }

    LoopFlow::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{gist_file_ref, state_with_gists};
    use crate::tui::*;
    use crossterm::event::KeyCode;

    #[test]
    fn stage_preview_content_cache_hit_enters_preview_without_spawn_payload() {
        let mut state = state_with_gists();
        let file = gist_file_ref("g1", "a.txt");
        state
            .gist_content_cache
            .insert(file.cache_key(), "cached".into());

        assert!(stage_preview_content(&mut state, file).is_none());

        assert_eq!(state.preview().expect("preview").body.text, "cached");
    }

    #[test]
    fn stage_preview_content_miss_falls_back_to_raw_url_and_returns_title() {
        let mut state = state_with_gists();
        state.gist_catalog.owned[0].raw_url = Some("https://example.test/a.txt".into());

        let (file, title) =
            stage_preview_content(&mut state, gist_file_ref("g1", "a.txt")).expect("spawn");

        assert_eq!(file.raw_url.as_deref(), Some("https://example.test/a.txt"));
        assert!(title.contains("a.txt"));
    }

    #[test]
    fn stage_refresh_preview_drops_cache_and_returns_fetch_payload() {
        let mut state = state_with_gists();
        state.gist_catalog.owned[0].raw_url = Some("https://example.test/a.txt".into());
        let file = gist_file_ref("g1", "a.txt");
        state
            .gist_content_cache
            .insert(file.cache_key(), "cached".into());
        state.enter_preview("a.txt".into(), "cached".into(), Some(file.cache_key()));

        let entry = state.defer_replacement().unwrap();
        let (file, title) = stage_refresh_preview(&mut state, file);

        assert!(entry.return_to.is_gists());
        assert!(state.gist_content_cache.get(&file.cache_key()).is_none());
        assert_eq!(file.raw_url.as_deref(), Some("https://example.test/a.txt"));
        assert!(title.contains("a.txt"));
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
        assert_eq!(state.scroll_body().expect("Preview ScrollBody").scroll, 1);
        state.handle_key(KeyCode::Up);
        assert_eq!(state.scroll_body().expect("Preview ScrollBody").scroll, 0);
    }

    #[test]
    fn page_keys_jump_by_ten() {
        let mut state = initial_state();
        let text = (0..12)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        state.enter_preview("t".into(), text, None);
        state.handle_key(KeyCode::PageDown);
        assert_eq!(state.scroll_body().expect("Preview ScrollBody").scroll, 10);
        state.handle_key(KeyCode::PageUp);
        assert_eq!(state.scroll_body().expect("Preview ScrollBody").scroll, 0);
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
        let mut layout = MouseFrame::default();
        layout.register(HitTarget::TopGists, Rect::new(10, 0, 7, 1));
        let out = state.handle_mouse(MouseInput::Click { col: 12, row: 0 }, &layout);
        assert!(state.screen.is_gists());
        assert_eq!(out, KeyOutcome::None);
    }

    #[test]
    fn top_bar_config_click_opens_settings_from_any_screen() {
        let mut state = state_with_gists();
        state.screen = Screen::Preview(Box::default());
        let mut layout = MouseFrame::default();
        layout.register(HitTarget::TopConfig, Rect::new(28, 0, 8, 1));
        let out = state.handle_mouse(MouseInput::Click { col: 30, row: 0 }, &layout);
        assert!(state.screen.is_config());
        assert!(state.nav_stack.last().is_some_and(Screen::is_preview));
        assert_eq!(out, KeyOutcome::None);
    }

    #[test]
    fn top_bar_config_click_while_already_on_config_does_not_trap_keyboard_exit() {
        let mut state = state_with_gists();
        state.screen = Screen::Preview(Box::default());
        let mut layout = MouseFrame::default();
        layout.register(HitTarget::TopConfig, Rect::new(28, 0, 8, 1));
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
        let mut layout = MouseFrame::default();
        layout.register(HitTarget::TopHelp, Rect::new(30, 0, 7, 1));
        let out = state.handle_mouse(MouseInput::Click { col: 32, row: 0 }, &layout);
        assert!(state.screen.is_help());
        assert!(state.nav_stack.last().is_some_and(Screen::is_preview));
        assert_eq!(out, KeyOutcome::None);
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
    fn on_preview_content_ok_caches_and_enters_preview() {
        let mut state = initial_state();
        let file = gist_file_ref("g1", "a.txt");

        on_preview_content(
            &mut state,
            initial_state().defer_entry(),
            Ok("body".into()),
            file.clone(),
            "a.txt".into(),
        );

        assert_eq!(
            state.gist_content_cache.get(&file.cache_key()),
            Some(&"body".to_string())
        );
        let preview = state.preview().expect("expected Screen::Preview");
        assert_eq!(preview.body.text, "body");
    }

    #[test]
    fn on_preview_content_err_sets_status() {
        let mut state = initial_state();

        on_preview_content(
            &mut state,
            initial_state().defer_entry(),
            Err("boom".into()),
            gist_file_ref("g1", "a.txt"),
            "a.txt".into(),
        );

        assert_eq!(state.status.as_deref(), Some("fetch failed: boom"));
    }
}
