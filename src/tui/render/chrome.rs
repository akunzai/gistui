//! Shared terminal chrome: top bar, footers, modals, and overlays.

use super::*;

pub(crate) fn render_close_button(frame: &mut Frame, outer: Rect, theme: &Theme) -> Rect {
    let text = "[✕]";
    let width = text.chars().count() as u16;
    if outer.width < width + 2 || outer.height == 0 {
        return Rect::new(outer.x, outer.y, 0, 0);
    }
    let x = outer.right().saturating_sub(width + 1);
    let rect = Rect::new(x, outer.y, width, 1);
    frame.render_widget(
        Paragraph::new(Span::styled(text, Style::default().fg(theme.accent))),
        rect,
    );
    rect
}

/// Renders the cross-screen top bar — ` gistui` on the left,
/// `(g)ists (P)ins (C)onfig (?)Help` right-aligned — into the top row of `area`, then returns
/// the remaining rect below it for the caller's existing content/footer layout (otherwise
/// unchanged). The icons render as plain text even with the mouse disabled, so the shortcuts
/// stay visible; their hit-rects are only recorded in `layout` when `mouse_enabled`, matching
/// every other clickable region.
pub(crate) fn render_top_bar(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    mouse_enabled: bool,
    layout: &mut MouseFrame,
) -> Rect {
    if area.height == 0 {
        return area;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(TOP_BAR_HEIGHT), Constraint::Min(0)])
        .split(area);
    let bar = chunks[0];

    let fit = fit_top_bar(bar.width);
    frame.render_widget(
        Paragraph::new(fit.name.unwrap_or("")).style(theme.base_style()),
        bar,
    );

    let key_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(theme.fg);

    for item in fit.items {
        let w = item.width.min(bar.width.saturating_sub(item.x));
        let rect = Rect::new(bar.x + item.x, bar.y, w, 1);
        let natural = top_bar_item_width(item.key, item.rest);
        let line = if w < natural {
            Line::from(Span::styled(
                truncate_end(&format!("({}){}", item.key, item.rest), w as usize),
                label_style,
            ))
        } else {
            Line::from(vec![
                Span::styled("(", label_style),
                Span::styled(item.key.to_string(), key_style),
                Span::styled(format!("){}", item.rest), label_style),
            ])
        };
        frame.render_widget(Paragraph::new(line).style(theme.base_style()), rect);
        if mouse_enabled {
            let target = match item.index {
                0 => HitTarget::TopGists,
                1 => HitTarget::TopPins,
                2 => HitTarget::TopConfig,
                _ => HitTarget::TopHelp,
            };
            layout.register(target, rect);
        }
    }

    chunks[1]
}

pub(crate) fn render_compact_gist_bg_vm(
    frame: &mut Frame,
    area: Rect,
    bg: &crate::tui::screens::confirm::CompactGistBgVm,
    theme: &Theme,
) {
    let mut lines: Vec<Line> = vec![
        Line::from(bg.info_line.clone()),
        Line::from(""),
        Line::from(Span::styled(
            format!("Files ({})", bg.files.len()),
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];
    let cursor = bg.file_cursor.min(bg.files.len().saturating_sub(1));
    // Visible file rows = area height minus borders(2), info line, blank, "Files (n)" header (3).
    let visible_rows = (area.height as usize).saturating_sub(5);
    let offset = file_list_scroll(cursor, visible_rows, bg.files.len());
    lines.extend(file_rows(
        &bg.files,
        cursor,
        offset,
        visible_rows,
        false,
        theme,
    ));
    frame.render_widget(
        Paragraph::new(lines).style(theme.base_style()).block(
            Block::default()
                .title(fit_block_title(&bg.block_title, area.width))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.accent))
                .style(theme.base_style())
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

/// The default (idle) footer hint on screens that used to show a long per-screen key dump.
pub(crate) const MINIMAL_HINT: &str = "; Menu · Ctrl+p Palette";

/// Footer text + whether to colourise it: a one-shot `state.status` message (shown plain) when
/// present, else the colourised key `hints`. Shared by every screen so action results/errors
/// surface consistently and are never swallowed by a hard-coded footer (see #72, #66).
pub(crate) fn footer_with_status(status: Option<&str>, hints: &str) -> (String, bool) {
    match status {
        Some(message) => (message.to_string(), false),
        None => (hints.to_string(), true),
    }
}

/// Greedy word-wrap line count, matching how `Paragraph` with `Wrap { trim: true }` breaks
/// space-separated words at `width`. Used to size the footer block to its content.
pub(crate) fn wrap_line_count(text: &str, width: u16) -> u16 {
    if width == 0 {
        return 1;
    }
    let width = width as usize;
    let mut lines: u16 = 1;
    let mut col = 0usize;
    // A word wider than the line is broken across lines by ratatui's wrapper rather than
    // clipped, so it has to cost the rows it really takes — a confirm's question can be one
    // unbroken deep path with nowhere to break (issue: the modal was sized one row short).
    let place = |lines: &mut u16, w: usize| {
        if w > width {
            *lines = lines.saturating_add(((w - 1) / width) as u16);
            let rem = w % width;
            if rem == 0 {
                width
            } else {
                rem
            }
        } else {
            w
        }
    };
    for word in text.split_whitespace() {
        let w = word.chars().count();
        if col == 0 {
            col = place(&mut lines, w);
        } else if col + 1 + w <= width {
            col += 1 + w;
        } else {
            lines = lines.saturating_add(1);
            col = place(&mut lines, w);
        }
    }
    lines
}

/// Height to reserve for a screen's footer `Layout` row: `0` when both `text` and `title` are
/// empty (the footer fully collapses), else the wrapped line count for `text` plus one row when
/// `title` is non-empty (ratatui's [`Block::title`] always consumes a row, even without borders).
pub(crate) fn footer_height(text: &str, width: u16, title: &str, colored: bool) -> u16 {
    if text.is_empty() && title.is_empty() {
        return 0;
    }
    let content = if text.is_empty() {
        0
    } else if colored {
        // Coloured hint lines are trimmed to one row by [`fit_hints`] (#342).
        1
    } else {
        wrap_line_count(text, width.saturating_sub(2)).max(1)
    };
    content + u16::from(!title.is_empty())
}

/// Colour a command key by what its action does, so destructive and mutating keys stand apart
/// from plain navigation at a glance: destructive (delete/remove/unpin) → Red, write/sync
/// (download/upload/create/sync/…) → Green, everything else (navigation/view) → Cyan. Matched on
/// whole label words so e.g. `pins` does not read as the `pin` action.
/// The accent a key gets from what its action risks. The category is read off the keymap table,
/// which is why this no longer has to guess it from the label's wording (issue #369).
pub(crate) fn category_color(category: Category, theme: &Theme) -> Color {
    match category {
        Category::Destructive => theme.del_color,
        Category::Write => theme.write_color,
        Category::Nav | Category::Read => theme.accent,
    }
}

/// Style a footer command string: the leading key token of each `·`-separated item is accented by
/// its action category, looked up in `bindings` (see [`category_color`]); the descriptive label
/// keeps the terminal's default brightness so it stays legible, and only the separators are
/// dimmed. Every input character is preserved verbatim so `wrap_line_count` sizing stays exact.
pub(crate) fn hint_line(text: &str, bindings: &[Binding], theme: &Theme) -> Line<'static> {
    let dim = Style::default().fg(theme.dim);
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, seg) in text.split('·').enumerate() {
        if i > 0 {
            spans.push(Span::styled("·", dim));
        }
        let lead = seg.len() - seg.trim_start().len();
        let (indent, rest) = seg.split_at(lead);
        if !indent.is_empty() {
            spans.push(Span::styled(indent.to_string(), dim));
        }
        if rest.is_empty() {
            continue;
        }
        match rest.find(char::is_whitespace) {
            Some(pos) => {
                let (k, label) = rest.split_at(pos);
                let category = category_for_footer_key(bindings, k);
                let key = Style::default().fg(category_color(category, theme));
                spans.push(Span::styled(k.to_string(), key));
                spans.push(Span::raw(label.to_string()));
            }
            None => spans.push(Span::styled(
                rest.to_string(),
                Style::default().fg(category_color(
                    category_for_footer_key(bindings, rest),
                    theme,
                )),
            )),
        }
    }
    Line::from(spans)
}

/// The shared footer block: plain text with horizontal padding, no border (the old dim top
/// divider was removed to reclaim a row and keep the chrome minimal). The repo URL, app
/// version, and update-check status used to live in the footer but have moved to Help → About
/// (see `about_topic_lines`).
pub(crate) fn footer_block(title: &str, theme: &Theme) -> Block<'static> {
    let mut block = Block::default()
        .borders(Borders::NONE)
        .style(theme.base_style())
        .padding(Padding::horizontal(1));
    // ratatui treats even an empty `.title("")` as a top title row, which would leave zero
    // inner height when the footer chunk is only one row tall — see `Block::inner`.
    if !title.is_empty() {
        block = block.title(title.to_string());
    }
    block
}

/// Render a command footer into `area`. `colored` accents the command keys; pass `false` for
/// plain text (filter input, status messages) that is not a key/label list.
pub(crate) fn render_footer(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    text: &str,
    colored: bool,
    bindings: &[Binding],
    theme: &Theme,
) {
    let inner = area.width.saturating_sub(2) as usize;
    let text = if colored {
        fit_hints(text, inner)
    } else {
        text.to_string()
    };
    let para = if colored {
        Paragraph::new(hint_line(&text, bindings, theme))
    } else {
        Paragraph::new(text)
    };
    frame.render_widget(
        para.style(theme.base_style())
            .wrap(Wrap { trim: true })
            .block(footer_block(title, theme)),
        area,
    );
}

/// Like [`render_footer`] but draws a prebuilt styled `line`, used for active text inputs
/// so the cursor can be reverse-highlighted at its real position.
pub(crate) fn render_footer_line(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    line: Line,
    theme: &Theme,
    _layout: &mut MouseFrame,
) {
    frame.render_widget(
        Paragraph::new(line)
            .style(theme.base_style())
            .wrap(Wrap { trim: true })
            .block(footer_block(title, theme)),
        area,
    );
}

/// A styled line for an active inline text input: `prefix`, then the input text with a
/// reverse-video block cursor at its real position, then `suffix`. A cursor at the end
/// reverses a trailing space so the caret is always visible.
pub(crate) fn input_line(prefix: &str, input: &TextInput, suffix: &str) -> Line<'static> {
    let chars: Vec<char> = input.chars().collect();
    let cursor = input.cursor().min(chars.len());
    let rev = Style::default().add_modifier(Modifier::REVERSED);
    let mut spans: Vec<Span<'static>> = Vec::new();
    if !prefix.is_empty() {
        spans.push(Span::raw(prefix.to_string()));
    }
    let before: String = chars[..cursor].iter().collect();
    if !before.is_empty() {
        spans.push(Span::raw(before));
    }
    if cursor < chars.len() {
        spans.push(Span::styled(chars[cursor].to_string(), rev));
        let after: String = chars[cursor + 1..].iter().collect();
        if !after.is_empty() {
            spans.push(Span::raw(after));
        }
    } else {
        spans.push(Span::styled(" ".to_string(), rev));
    }
    if !suffix.is_empty() {
        spans.push(Span::raw(suffix.to_string()));
    }
    Line::from(spans)
}

/// Centered modal rect sized to fit `body` (clamped to the frame).
pub(crate) fn centered_modal_rect(area: Rect, body: &str) -> Rect {
    let max_width = area.width.saturating_sub(2).max(1);
    let width = ((area.width as u32 * 60 / 100) as u16).clamp(max_width.min(20), max_width);
    // Inner text width = box width minus the two border columns and the horizontal padding.
    let inner_width = width.saturating_sub(4);
    let body_lines = wrap_line_count(body, inner_width).max(1);
    let max_height = area.height.saturating_sub(2).max(1);
    let height = (body_lines + 2).clamp(max_height.min(3), max_height);
    Rect {
        x: area.width.saturating_sub(width) / 2,
        y: area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn modal_block(
    title: &str,
    area_width: u16,
    border: Color,
    theme: &Theme,
) -> Block<'static> {
    // The title is spaced off the corner so it reads as a label rather than as part of the
    // border run; `fit_block_title` still clips it on a narrow frame.
    Block::default()
        .title(fit_block_title(&format!(" {title} "), area_width))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(theme.base_style())
        .padding(Padding::horizontal(1))
}

/// Horizontal and vertical padding inside a confirm modal: two columns each side so the text
/// clears the border, and one blank row above and below so a decision does not read as a
/// status sliver (`docs/agents/design.md`).
const CONFIRM_MODAL_PADDING: Padding = Padding {
    left: 2,
    right: 2,
    top: 1,
    bottom: 1,
};

/// Cells the confirm modal's padding and borders take from its width.
const CONFIRM_MODAL_CHROME: u16 = 6;

/// Lay the resolving keys out as aligned columns that fit `inner_width`.
///
/// Every cell is padded to the widest `key  label` in the modal, so a second row of toggles
/// lines up under the first row's actions. When the keys cannot all fit on one line they are
/// packed onto further lines rather than left to `Wrap` — word-wrapping a key row splits a
/// key from its own verb (`e` on one line, `edit first` on the next) and costs the sizing
/// pass a row it did not count.
pub(crate) fn confirm_key_rows(
    keys: &[crate::tui::screens::confirm::ConfirmKeyVm],
    options: &[crate::tui::screens::confirm::ConfirmKeyVm],
    inner_width: u16,
    border: Color,
) -> Vec<Line<'static>> {
    let cell = keys
        .iter()
        .chain(options)
        .map(|k| k.width())
        .max()
        .unwrap_or(0)
        + 4;
    // A run of `n` cells is `n * cell` wide less the trailing gutter the last one does not
    // need. At least one key per line, however narrow the modal is.
    let per_line = (1..)
        .take_while(|n| n * cell <= inner_width as usize + 4)
        .last()
        .unwrap_or(1);
    let line = |row_keys: &[crate::tui::screens::confirm::ConfirmKeyVm]| {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (i, k) in row_keys.iter().enumerate() {
            spans.push(Span::styled(
                k.key.to_string(),
                Style::default().fg(border).add_modifier(Modifier::BOLD),
            ));
            let pad = if i + 1 == row_keys.len() {
                String::new()
            } else {
                " ".repeat(cell.saturating_sub(k.width()))
            };
            spans.push(Span::raw(format!("  {}{}", k.label, pad)));
        }
        Line::from(spans)
    };
    let mut rows = Vec::new();
    for group in [keys, options] {
        rows.extend(group.chunks(per_line).map(line));
    }
    rows
}

/// Body lines and the width they were laid out for, shared by sizing and painting so the
/// modal can never be one row short of what it draws.
fn confirm_modal_body(
    prompt: &crate::tui::screens::confirm::ConfirmPromptVm,
    inner_width: u16,
    border: Color,
    theme: &Theme,
) -> (Vec<Line<'static>>, u16) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut rows = wrap_line_count(&prompt.question, inner_width);
    lines.push(Line::from(prompt.question.clone()));
    if let Some(detail) = &prompt.detail {
        rows = rows.saturating_add(wrap_line_count(detail, inner_width));
        lines.push(Line::from(Span::styled(
            detail.clone(),
            Style::default().fg(theme.dim),
        )));
    }
    let key_rows = confirm_key_rows(&prompt.keys, &prompt.options, inner_width, border);
    if !key_rows.is_empty() {
        // One blank row separates the question from the answer.
        lines.push(Line::from(String::new()));
        rows = rows.saturating_add(1 + key_rows.len() as u16);
        lines.extend(key_rows);
    }
    (lines, rows)
}

/// Centered modal rect for a confirm body of `body_rows` laid-out rows. Width follows
/// [`centered_modal_rect`]'s 60% rule so every confirm is the same size regardless of how
/// short its question is.
fn centered_confirm_rect(area: Rect, body_rows: u16) -> Rect {
    let width = confirm_modal_width(area);
    let max_height = area.height.saturating_sub(2).max(1);
    // Two borders plus the one blank row of padding above and below.
    let height = body_rows
        .saturating_add(4)
        .clamp(max_height.min(3), max_height);
    Rect {
        x: area.width.saturating_sub(width) / 2,
        y: area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

/// Every confirm is the same width — 60% of the frame — regardless of how short its question
/// is, so the modal does not change size between the two steps of one action. Shared by the
/// sizing pass and the wrap-width probe that feeds it.
fn confirm_modal_width(area: Rect) -> u16 {
    let max_width = area.width.saturating_sub(2).max(1);
    ((area.width as u32 * 60 / 100) as u16).clamp(max_width.min(24), max_width)
}

/// The confirm modal: question, consequence, then the keys that resolve it.
pub(crate) fn render_confirm_modal(
    frame: &mut Frame,
    title: &str,
    prompt: &crate::tui::screens::confirm::ConfirmPromptVm,
    border: Color,
    theme: &Theme,
) -> Rect {
    let area = frame.area();
    let inner_width = confirm_modal_width(area).saturating_sub(CONFIRM_MODAL_CHROME);
    let (lines, rows) = confirm_modal_body(prompt, inner_width, border, theme);
    let rect = centered_confirm_rect(area, rows);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme.base_style())
            .wrap(Wrap { trim: true })
            .block(modal_block(title, rect.width, border, theme).padding(CONFIRM_MODAL_PADDING)),
        rect,
    );
    rect
}

/// The create flow's description editor, laid out like [`render_confirm_modal`] so the two
/// steps of one action do not change shape between keystrokes.
pub(crate) fn render_confirm_input_modal(
    frame: &mut Frame,
    title: &str,
    prefix: &str,
    input: &TextInput,
    keys: &[crate::tui::screens::confirm::ConfirmKeyVm],
    border: Color,
    theme: &Theme,
) -> Rect {
    let area = frame.area();
    let inner_width = confirm_modal_width(area).saturating_sub(CONFIRM_MODAL_CHROME);
    // The modal grows downwards as the description outgrows one line, so the caret stays on
    // screen; without wrapping, `Paragraph` would truncate and type blind past the border.
    let mut lines = vec![input_line(prefix, input, "")];
    let mut rows = wrap_line_count(&format!("{prefix}{input} "), inner_width);
    let key_rows = confirm_key_rows(keys, &[], inner_width, border);
    if !key_rows.is_empty() {
        lines.push(Line::from(String::new()));
        rows = rows.saturating_add(1 + key_rows.len() as u16);
        lines.extend(key_rows);
    }
    let rect = centered_confirm_rect(area, rows);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme.base_style())
            .wrap(Wrap { trim: false })
            .block(modal_block(title, rect.width, border, theme).padding(CONFIRM_MODAL_PADDING)),
        rect,
    );
    rect
}

pub(crate) fn render_centered_modal(
    frame: &mut Frame,
    title: &str,
    body: &str,
    border: Color,
    theme: &Theme,
) -> Rect {
    let rect = centered_modal_rect(frame.area(), body);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(body.to_string())
            .style(theme.base_style())
            .wrap(Wrap { trim: true })
            .block(modal_block(title, rect.width, border, theme)),
        rect,
    );
    rect
}

/// Centered modal whose body is an active text input (`prefix` + text-with-cursor +
/// `suffix`), so the description editor shows the caret at its real position.
pub(crate) fn render_centered_modal_input(
    frame: &mut Frame,
    title: &str,
    prefix: &str,
    input: &TextInput,
    suffix: &str,
    border: Color,
    theme: &Theme,
) -> Rect {
    // Size from the plain text plus one column for the (possibly trailing) cursor cell.
    let plain = format!("{prefix}{input} {suffix}");
    let rect = centered_modal_rect(frame.area(), &plain);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(input_line(prefix, input, suffix))
            .style(theme.base_style())
            .wrap(Wrap { trim: true })
            .block(modal_block(title, rect.width, border, theme)),
        rect,
    );
    rect
}

/// Frames for the in-progress spinner, advanced by `AppState::spinner_frame` (one step per
/// event-loop tick, ~150ms). Braille dots are single-width and widely supported, so no ASCII
/// fallback is added here.
pub(crate) const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The spinner glyph for the given tick. `frame` may be any value; it is reduced modulo the
/// frame count.
pub(crate) fn spinner_glyph(frame: usize) -> &'static str {
    SPINNER_FRAMES[frame % SPINNER_FRAMES.len()]
}

/// Column width for palette key hints: at least one char, wide enough for the longest
/// visible key (`Enter`, `Ctrl+p`, …) so labels never run into the hint.
#[cfg(test)]
pub(crate) fn palette_key_width(items: &[&PaletteItem]) -> usize {
    items
        .iter()
        .map(|item| item.key_hint.chars().count())
        .max()
        .unwrap_or(1)
        .max(1)
}

/// One palette row from a full [`PaletteItem`] (unit tests).
#[cfg(test)]
pub(crate) fn palette_row_line(
    item: &PaletteItem,
    key_width: usize,
    theme: &Theme,
    row_style: Style,
) -> Line<'static> {
    palette_row_spans(
        &item.key_hint,
        &item.label,
        item.category,
        key_width,
        theme,
        row_style,
    )
}

/// Shared by palette paint (`PaletteVm` rows) and test helpers.
pub(crate) fn palette_row_spans(
    key_hint: &str,
    label: &str,
    category: Category,
    key_width: usize,
    theme: &Theme,
    row_style: Style,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<key_width$}  ", key_hint, key_width = key_width),
            Style::default().fg(category_color(category, theme)),
        ),
        Span::styled(label.to_string(), row_style),
    ])
}

/// A centered "Working…" box shown while a blocking `gh` action runs.
pub(crate) fn render_loading_overlay(
    frame: &mut Frame,
    msg: &str,
    spinner_frame: usize,
    theme: &Theme,
) {
    let body = format!("{} {msg}", spinner_glyph(spinner_frame));
    render_centered_modal(frame, "Working…", &body, theme.accent, theme);
}

// Shared styled-span horizontal scrolling.
pub(crate) fn apply_hscroll_spans(spans: Vec<Span<'static>>, hscroll: usize) -> Line<'static> {
    let mut skip = hscroll;
    let visible: Vec<Span<'static>> = spans
        .into_iter()
        .filter_map(|span| {
            let len = span.content.chars().count();
            if skip >= len {
                skip -= len;
                None
            } else {
                let content: String = span.content.chars().skip(skip).collect();
                skip = 0;
                if content.is_empty() {
                    None
                } else {
                    Some(Span::styled(content, span.style))
                }
            }
        })
        .collect();
    Line::from(visible)
}

/// Overlay a vertical scrollbar on the right edge of a bordered, scrollable text pane when
/// its `total` lines overflow the inner viewport. `offset` is the index of the topmost
/// visible line, so the thumb reflects the real scroll position (not a selection index).
pub(crate) fn render_text_scrollbar(frame: &mut Frame, area: Rect, total: usize, offset: usize) {
    let viewport = area.height.saturating_sub(2) as usize;
    if viewport == 0 || total <= viewport {
        return;
    }
    let mut sb_state = ScrollbarState::new(total).position(offset);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut sb_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use crossterm::event::{KeyCode, KeyModifiers};

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
    fn wrap_line_count_is_responsive_to_width() {
        let text = "aaa bbb ccc";
        assert_eq!(wrap_line_count(text, 100), 1);
        assert_eq!(wrap_line_count(text, 7), 2);
        assert_eq!(wrap_line_count(text, 3), 3);
        assert_eq!(wrap_line_count(text, 0), 1);
    }

    #[test]
    fn wrap_line_count_breaks_a_word_wider_than_the_line() {
        // ratatui hard-breaks an overlong word instead of clipping it, so the count has to
        // follow — a confirm's question can be one unbroken deep path.
        assert_eq!(wrap_line_count("aaaaaaaaaa", 5), 2);
        assert_eq!(wrap_line_count("aaaaaaaaaaa", 5), 3);
        assert_eq!(wrap_line_count("ab aaaaaaaaaa", 5), 3);
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
}
