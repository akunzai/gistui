//! Highlighted unified-diff painting.

use super::*;

/// Word-level inline highlight for a unified-diff `-`/`+` line.
/// `bold_tag` is the change side that gets BOLD (`Delete` for del, `Insert` for ins).
fn inline_change_line(
    del_line: &str,
    ins_line: &str,
    hscroll: usize,
    color: Color,
    prefix: char,
    bold_tag: ChangeTag,
) -> Line<'static> {
    let del_content = del_line.get(1..).unwrap_or("");
    let ins_content = ins_line.get(1..).unwrap_or("");
    let mut spans = vec![Span::styled(prefix.to_string(), Style::default().fg(color))];
    for change in TextDiff::from_words(del_content, ins_content).iter_all_changes() {
        let tag = change.tag();
        if tag == bold_tag {
            spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
        } else if tag == ChangeTag::Equal {
            spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(color),
            ));
        }
    }
    apply_hscroll_spans(spans, hscroll)
}

/// Del line with word-level highlighting: changed words bold-red, unchanged words plain red.
fn inline_del_line(
    del_line: &str,
    ins_line: &str,
    hscroll: usize,
    del_color: Color,
) -> Line<'static> {
    inline_change_line(
        del_line,
        ins_line,
        hscroll,
        del_color,
        '-',
        ChangeTag::Delete,
    )
}

/// Ins line with word-level highlighting: changed words bold-green, unchanged words plain green.
fn inline_ins_line(
    del_line: &str,
    ins_line: &str,
    hscroll: usize,
    ins_color: Color,
) -> Line<'static> {
    inline_change_line(
        del_line,
        ins_line,
        hscroll,
        ins_color,
        '+',
        ChangeTag::Insert,
    )
}

/// Renders a `--- /+++` header line, tinting the leading `local`/`gist` keyword (yellow/blue)
/// so each side's identity is readable regardless of which way the diff is oriented — the
/// `Enter` preview flips direction with focus (see `preview_diff_text`). The side is classified
/// from the un-scrolled line (anchored right after the marker), then the keyword is coloured in
/// the horizontally-scrolled slice; the rest stays bold.
fn header_line(line: &str, hscroll: usize, theme: &Theme) -> Line<'static> {
    let visible: String = line.chars().skip(hscroll).collect();
    let bold = Style::default().add_modifier(Modifier::BOLD);

    let body = line
        .strip_prefix("--- ")
        .or_else(|| line.strip_prefix("+++ "))
        .unwrap_or(line);
    let (keyword, color) = if body.starts_with("local") {
        ("local", theme.notice_color)
    } else if body.starts_with("gist") {
        ("gist", theme.gist_label_color)
    } else {
        return Line::styled(visible, bold);
    };

    // The marker is dashes/pluses with no letters, so the first hit of the keyword in the
    // visible slice is the real label keyword (not a substring of a filename).
    match visible.find(keyword) {
        Some(idx) => Line::from(vec![
            Span::styled(visible[..idx].to_string(), bold),
            Span::styled(
                visible[idx..idx + keyword.len()].to_string(),
                bold.fg(color),
            ),
            Span::styled(visible[idx + keyword.len()..].to_string(), bold),
        ]),
        None => Line::styled(visible, bold),
    }
}

/// Builds the visible, coloured slice of a unified diff. Adjacent `-`/`+` line pairs receive
/// word-level inline highlighting (changed words bold, unchanged words dim) so small edits are
/// easy to spot. Scrolling is applied by hand — skip `vscroll` lines and drop `hscroll` leading
/// chars per line — rather than via `Paragraph::scroll`, whose styled-line handling leaves
/// redraw artifacts in ratatui 0.26.
///
/// When `highlight` is on and `ext` names a known language, the unchanged context lines (those
/// prefixed by a space) are syntax coloured; `-`/`+` lines keep their red/green + word-level
/// highlighting untouched so the add/delete signal stays dominant. Tabbed context lines are left
/// plain so their indentation stays aligned with the raw-tab `-`/`+` lines.
fn diff_view_highlighted(
    text: &str,
    vscroll: u16,
    hscroll: u16,
    ext: Option<&str>,
    highlight: bool,
    theme: &Theme,
) -> Text<'static> {
    let raw: Vec<&str> = text.lines().collect();
    let hscroll = hscroll as usize;
    let mut result: Vec<Line<'static>> = Vec::with_capacity(raw.len());

    // Pre-highlight the unchanged context lines as one buffer, keyed back by raw line index.
    let ctx_highlight: std::collections::HashMap<usize, Vec<Span<'static>>> = match (highlight, ext)
    {
        (true, Some(ext)) => {
            let mut idxs = Vec::new();
            let mut contents = Vec::new();
            for (idx, l) in raw.iter().enumerate() {
                if l.starts_with(' ') && !l.contains('\t') {
                    idxs.push(idx);
                    contents.push(l[1..].to_string());
                }
            }
            crate::tui::highlight::highlight_buffer(ext, &contents, theme)
                .into_iter()
                .zip(idxs)
                .map(|(spans, idx)| (idx, spans))
                .collect()
        }
        _ => std::collections::HashMap::new(),
    };

    let mut i = 0;
    while i < raw.len() {
        let line = raw[i];
        let is_del = line.starts_with('-') && !line.starts_with("---");
        let is_ins = line.starts_with('+') && !line.starts_with("+++");

        if is_del || is_ins {
            // Collect the contiguous del run then ins run.
            let del_start = i;
            while i < raw.len() && raw[i].starts_with('-') && !raw[i].starts_with("---") {
                i += 1;
            }
            let del_lines = &raw[del_start..i];

            let ins_start = i;
            while i < raw.len() && raw[i].starts_with('+') && !raw[i].starts_with("+++") {
                i += 1;
            }
            let ins_lines = &raw[ins_start..i];

            let pair_count = del_lines.len().min(ins_lines.len());

            // Del lines: paired ones get inline highlighting, extras plain red.
            for (j, &dl) in del_lines.iter().enumerate() {
                if j < pair_count {
                    result.push(inline_del_line(dl, ins_lines[j], hscroll, theme.del_color));
                } else {
                    let visible: String = dl.chars().skip(hscroll).collect();
                    result.push(Line::styled(visible, Style::default().fg(theme.del_color)));
                }
            }
            // Ins lines: paired ones get inline highlighting, extras plain.
            for (j, &il) in ins_lines.iter().enumerate() {
                if j < pair_count {
                    result.push(inline_ins_line(del_lines[j], il, hscroll, theme.ins_color));
                } else {
                    let visible: String = il.chars().skip(hscroll).collect();
                    result.push(Line::styled(visible, Style::default().fg(theme.ins_color)));
                }
            }
        } else if line.starts_with("+++") || line.starts_with("---") {
            result.push(header_line(line, hscroll, theme));
            i += 1;
        } else if let Some(spans) = ctx_highlight.get(&i) {
            // Syntax-highlighted context line: re-prepend the space marker, then scroll.
            let mut line_spans = Vec::with_capacity(spans.len() + 1);
            line_spans.push(Span::raw(" ".to_string()));
            line_spans.extend(spans.iter().cloned());
            result.push(apply_hscroll_spans(line_spans, hscroll));
            i += 1;
        } else {
            let visible: String = line.chars().skip(hscroll).collect();
            result.push(Line::styled(visible, Style::default()));
            i += 1;
        }
    }

    Text::from(
        result
            .into_iter()
            .skip(vscroll as usize)
            .collect::<Vec<_>>(),
    )
}

/// Label and trailing hint around the create flow's description input. Shared so
/// `confirm_prompt` (in `view_model.rs`; plain text / tests) and `render_confirm` (the
/// cursor-aware modal, here) can't drift apart.
pub(crate) const CREATE_DESC_PREFIX: &str = "Description (optional): ";
pub(crate) const CREATE_DESC_SUFFIX: &str = "   ·  Enter next  ·  Esc cancel";

/// Render just the diff content pane (no footer) into `area` from a [`DiffVm`].
pub(crate) fn render_diff_pane_vm(
    frame: &mut Frame,
    area: Rect,
    diff: &crate::tui::view_model::DiffVm,
    theme: &Theme,
) {
    let block = Block::default()
        .title(fit_block_title(&diff.title, area.width))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(theme.base_style())
        .padding(Padding::horizontal(1));
    let paragraph = if diff.wrap {
        // Wrapping needs the full, un-h-scrolled line set; vertical scroll goes through
        // Paragraph. Mirrors render_preview's wrap branch.
        Paragraph::new(diff_view_highlighted(
            &diff.body,
            0,
            0,
            diff.ext.as_deref(),
            diff.syntax_highlight,
            theme,
        ))
        .style(theme.base_style())
        .scroll((diff.scroll, 0))
        .wrap(Wrap { trim: false })
        .block(block)
    } else {
        Paragraph::new(diff_view_highlighted(
            &diff.body,
            diff.scroll,
            diff.hscroll,
            diff.ext.as_deref(),
            diff.syntax_highlight,
            theme,
        ))
        .style(theme.base_style())
        .block(block)
    };
    frame.render_widget(paragraph, area);
    // The scrollbar's 1:1 line↔row mapping only holds without soft wrapping (see render_preview).
    if !diff.wrap {
        let total_lines = diff.body.lines().count();
        render_text_scrollbar(frame, area, total_lines, diff.scroll as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
