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
    let footer_lines =
        crate::tui::wrap_line_count(&preview.footer, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(footer_lines)])
        .split(area);

    // When wrapping, horizontal scroll is meaningless — pin the x offset to 0 so long lines
    // wrap into view instead of being scrolled off-screen.
    let block = Block::default()
        .title(preview.title.clone())
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
