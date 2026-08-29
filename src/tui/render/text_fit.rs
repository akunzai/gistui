//! Width-aware text fitting helpers.

use super::*;

/// Separator between the segments of a pane title.
pub(crate) const TITLE_SEP: &str = " · ";

/// Shortest elided context worth painting: `…` plus at least three cells.
pub(crate) const MIN_ELIDED_WIDTH: usize = 4;

/// Width of `text` in terminal cells — the same measure ratatui truncates a title by, so a
/// double-width glyph (a CJK path component, say) is not silently clipped by the block after
/// we let it through. The UI's own marks are all single-width by design (`docs/design.md`).
pub(crate) fn cell_width(text: &str) -> usize {
    ratatui::text::Span::raw(text).width()
}

/// Join a [`PaneTitleVm`] into at most `width` cells (issue #338) — that type documents which
/// part gives way first.
///
/// A segment that does not fit is dropped whole, together with its separator and everything
/// after it, so a narrow title never ends in a dangling `·` and never half-shows a state value.
/// The context is appended last and is the only part that shrinks, keeping its tail behind a
/// leading `…` — for a path, the half naming the directory.
///
/// Below the width the full head needs, an available `short_head` is spent — but only when the
/// cells it frees actually buy back a state segment. Shortening the head to win nothing but a
/// longer cwd would trade a label the user reads for context they can already see in the shell.
pub(crate) fn fit_title(title: &PaneTitleVm, width: usize) -> String {
    let full = fit_segments(&title.segments, width);
    let fitted = match title.short_head.as_deref() {
        Some(short) if full.shown < title.segments.len() => {
            let mut segments = title.segments.clone();
            segments[0] = short.to_string();
            let abbreviated = fit_segments(&segments, width);
            // Ties go to the full head: same state, more of the pane's name.
            if abbreviated.shown > full.shown {
                abbreviated
            } else {
                full
            }
        }
        _ => full,
    };

    let mut text = fitted.text;
    // A head that had to be clipped has already taken the whole width; anything after it would
    // overflow. An absent or empty context must not leave its separator behind either.
    if fitted.shown == 0 {
        return text;
    }
    let Some(context) = title.context.as_deref().filter(|c| !c.is_empty()) else {
        return text;
    };
    let sep = if text.is_empty() { "" } else { TITLE_SEP };
    let room = width.saturating_sub(fitted.used + cell_width(sep));
    if cell_width(context) <= room {
        text.push_str(sep);
        text.push_str(context);
    } else if room >= MIN_ELIDED_WIDTH {
        text.push_str(sep);
        text.push_str(&elide_start(context, room));
    }
    text
}

/// How much of `segments` fits `width`, and what that cost.
pub(crate) struct FittedSegments {
    text: String,
    used: usize,
    /// Segments joined whole. Fewer than `segments.len()` means state was dropped.
    shown: usize,
}

/// Join `segments` greedily, stopping at the first one that does not fit.
pub(crate) fn fit_segments(segments: &[String], width: usize) -> FittedSegments {
    let mut text = String::new();
    let mut used = 0usize;
    for (i, segment) in segments.iter().enumerate() {
        let sep = if i == 0 { "" } else { TITLE_SEP };
        let cost = cell_width(sep) + cell_width(segment);
        if used + cost > width {
            // Too narrow even for the head: clip it (marked with `…`, #340) rather than
            // painting a bare border.
            if i == 0 {
                text.push_str(&truncate_end(segment, width));
                used = cell_width(&text);
            }
            return FittedSegments {
                text,
                used,
                shown: i,
            };
        }
        text.push_str(sep);
        text.push_str(segment);
        used += cost;
    }
    FittedSegments {
        text,
        used,
        shown: segments.len(),
    }
}

/// `text` shortened to `room` cells by dropping its head behind a leading `…`.
pub(crate) fn elide_start(text: &str, room: usize) -> String {
    let tail_budget = room.saturating_sub(1);
    // The leftmost byte offset whose suffix fits is the longest tail we can keep.
    let start = text
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| cell_width(&text[i..]) <= tail_budget)
        .unwrap_or(text.len());
    format!("{ELLIPSIS}{}", &text[start..])
}

/// Ellipsis marking that a value was clipped (issue #340). One cell in `cell_width`.
pub(crate) const ELLIPSIS: &str = "…";

/// Width-aware end truncation (issue #340): a value that fits is returned unchanged; a
/// value that does not is cut on a character boundary and marked with `…`. Wide glyphs
/// are never split — a leftover cell is left empty rather than showing half a character.
pub(crate) fn truncate_end(text: &str, width: usize) -> String {
    if cell_width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let ellipsis_width = cell_width(ELLIPSIS);
    if width < ellipsis_width {
        return String::new();
    }
    let budget = width - ellipsis_width;
    let mut clipped = String::new();
    let mut used = 0usize;
    let mut buf = [0u8; 4];
    for ch in text.chars() {
        let cells = cell_width(ch.encode_utf8(&mut buf));
        if used + cells > budget {
            break;
        }
        clipped.push(ch);
        used += cells;
    }
    clipped.truncate(clipped.trim_end().len());
    clipped.push_str(ELLIPSIS);
    clipped
}

/// Soft-wrap `text` to `width` cells, continuing each overflow row at the source
/// line's leading whitespace (issue #342). A line that fits is returned unchanged.
pub(crate) fn wrap_hanging(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    if cell_width(text) <= width {
        return vec![text.to_string()];
    }
    let indent_bytes = text.len() - text.trim_start().len();
    let indent = &text[..indent_bytes];
    let rest = &text[indent_bytes..];
    let indent_w = cell_width(indent);
    let hang = if indent_w > 0 && indent_w < width {
        indent
    } else {
        ""
    };

    let mut lines = Vec::new();
    let mut remaining = rest;
    let mut first = true;
    while !remaining.is_empty() {
        let prefix = if first {
            if indent_w < width {
                indent
            } else {
                ""
            }
        } else {
            hang
        };
        first = false;
        let budget = width.saturating_sub(cell_width(prefix));
        let (take, rest_after) = take_fitting(remaining, budget);
        if take.is_empty() {
            // Nothing fits next to the prefix (wide glyph, leftover cell): take one
            // character so the loop always advances.
            let ch_end = remaining
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(remaining.len());
            lines.push(format!("{prefix}{}", &remaining[..ch_end]));
            remaining = &remaining[ch_end..];
            continue;
        }
        lines.push(format!("{prefix}{take}"));
        remaining = rest_after;
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Longest prefix of `text` that fits `budget` cells, preferring a trailing space as the
/// break. The returned remainder has leading spaces stripped (the break itself).
pub(crate) fn take_fitting(text: &str, budget: usize) -> (&str, &str) {
    if budget == 0 {
        return ("", text);
    }
    if cell_width(text) <= budget {
        return (text, "");
    }
    let mut used = 0usize;
    let mut last_break: Option<usize> = None;
    let mut end = 0usize;
    let mut buf = [0u8; 4];
    for (i, ch) in text.char_indices() {
        let cells = cell_width(ch.encode_utf8(&mut buf));
        if used + cells > budget {
            break;
        }
        used += cells;
        end = i + ch.len_utf8();
        if ch == ' ' {
            last_break = Some(i);
        }
    }
    let split_at = last_break.unwrap_or(end);
    if split_at == 0 {
        return ("", text);
    }
    let take = &text[..split_at];
    let rest = text[split_at..].trim_start();
    (take, rest)
}

/// Trim a ` · `-separated hint line to `width` cells by dropping whole items,
/// keeping the last (leave-key) item (issue #342). A line that fits is unchanged.
pub(crate) fn fit_hints(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if cell_width(text) <= width {
        return text.to_string();
    }
    let sep = if text.contains("  ·  ") {
        "  ·  "
    } else {
        " · "
    };
    let items: Vec<&str> = text.split(sep).filter(|item| !item.is_empty()).collect();
    if items.is_empty() {
        return truncate_end(text, width);
    }
    let last = items[items.len() - 1];
    if cell_width(last) > width {
        return truncate_end(last, width);
    }
    let sep_w = cell_width(sep);
    let mut prefix: Vec<&str> = Vec::new();
    let mut used = cell_width(last);
    for item in &items[..items.len() - 1] {
        let extra = cell_width(item) + sep_w;
        if used + extra > width {
            break;
        }
        prefix.push(item);
        used += extra;
    }
    let mut out = prefix.join(sep);
    if !out.is_empty() {
        out.push_str(sep);
    }
    out.push_str(last);
    out
}

/// Title sitting on a bordered block: the two corner glyphs are not available.
pub(crate) fn fit_block_title(title: &str, area_width: u16) -> String {
    truncate_end(title, area_width.saturating_sub(2) as usize)
}

/// Cross-screen top-bar shortcuts: bracketed hotkey letter + label. Kept in one place so the
/// click hit-rect math and the rendered text can never drift apart. Order matches the
/// right-aligned strip: Gists · Pins · Config · Help (Config sits immediately left of Help,
/// same as duodiff). Gists uses lowercase `g` because that is the actual key (unlike Pins/Config,
/// which require Shift) — see `handle_key_list` in `screens/list.rs`.
pub(crate) const TOP_BAR_ITEMS: [(&str, &str); 4] =
    [("g", "ists"), ("P", "ins"), ("C", "onfig"), ("?", "Help")];

/// Left-side brand. A leading space is the left margin on a wide terminal.
pub(crate) const TOP_BAR_NAME: &str = " gistui";

/// Height of the persistent top bar rendered on every screen except the transient `Confirm`
/// y/n modal (which keeps its full-bleed diff/gist-info background — see `render_confirm`).
pub(crate) const TOP_BAR_HEIGHT: u16 = 1;

/// One shortcut that survived [`fit_top_bar`].
pub(crate) struct FittedTopBarItem {
    pub(crate) index: usize,
    pub(crate) key: &'static str,
    pub(crate) rest: &'static str,
    pub(crate) x: u16,
    pub(crate) width: u16,
}

/// Width budget for the top bar (issue #371): shortcuts keep the row; the brand
/// is decoration and only appears when the full name plus a one-cell gap fits
/// to the left of them.
pub(crate) struct FittedTopBar {
    pub(crate) name: Option<&'static str>,
    pub(crate) items: Vec<FittedTopBarItem>,
}

pub(crate) fn top_bar_item_width(key: &str, rest: &str) -> u16 {
    // "(" + key + ")" + rest — labels are ASCII, so chars == cells.
    (key.chars().count() + rest.chars().count() + 2) as u16
}

/// Split `width` between the brand and the right-aligned shortcuts.
///
/// The shortcuts take the row first. Whole items drop from the left if they
/// cannot all fit (same "drop a whole item" rule as [`fit_hints`]). The brand
/// is shown only when `" gistui"` plus a one-cell gap still fits in the leftover
/// — never truncated to `gis…`.
pub(crate) fn fit_top_bar(width: u16) -> FittedTopBar {
    const ITEM_GAP: u16 = 2;
    const RIGHT_MARGIN: u16 = 1;
    const NAME_GAP: u16 = 1;

    if width == 0 {
        return FittedTopBar {
            name: None,
            items: Vec::new(),
        };
    }

    let n = TOP_BAR_ITEMS.len();
    for start in 0..n {
        let slice = &TOP_BAR_ITEMS[start..];
        let widths: Vec<u16> = slice
            .iter()
            .map(|(key, rest)| top_bar_item_width(key, rest))
            .collect();
        let items_w: u16 = widths.iter().sum::<u16>() + ITEM_GAP * (slice.len() as u16 - 1);
        if items_w > width {
            continue;
        }
        let margin = if items_w + RIGHT_MARGIN <= width {
            RIGHT_MARGIN
        } else {
            0
        };
        let items_x = width - items_w - margin;
        let name_w = cell_width(TOP_BAR_NAME) as u16;
        let name = if name_w + NAME_GAP <= items_x {
            Some(TOP_BAR_NAME)
        } else {
            None
        };

        let mut x = items_x;
        let mut items = Vec::with_capacity(slice.len());
        for (i, ((key, rest), w)) in slice.iter().zip(widths).enumerate() {
            items.push(FittedTopBarItem {
                index: start + i,
                key,
                rest,
                x,
                width: w,
            });
            x += w + ITEM_GAP;
        }
        return FittedTopBar { name, items };
    }

    // Narrower than the last remaining shortcut: park it at the left edge.
    // Paint marks the clip with `truncate_end`, same as `fit_hints` on a leave key.
    let (key, rest) = TOP_BAR_ITEMS[n - 1];
    FittedTopBar {
        name: None,
        items: vec![FittedTopBarItem {
            index: n - 1,
            key,
            rest,
            x: 0,
            width,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Local pane title from the bug report, anchored and filtered.
    fn local_title() -> PaneTitleVm {
        let mut title = PaneTitleVm::new("[1] Local (6/15) ⚑".into());
        title.short_head = Some("[1] (6/15) ⚑".into());
        title.push("sort:match");
        title.push_filter("md");
        title.context = Some("~/code/gistui".into());
        title
    }

    /// A sweep probe: the width at or above which `fit_title` still shows `needle`.
    fn narrowest_width_showing(title: &PaneTitleVm, needle: &str) -> usize {
        (0..=80)
            .find(|&w| fit_title(title, w).contains(needle))
            .expect("needle never appears")
    }

    #[test]
    fn truncate_end_leaves_a_fitting_value_untouched() {
        assert_eq!(truncate_end("", 0), "");
        assert_eq!(truncate_end("hello", 5), "hello");
        assert_eq!(truncate_end("hello", 10), "hello");
        assert_eq!(truncate_end("日本語", 6), "日本語");
    }

    #[test]
    fn truncate_end_marks_a_clipped_value_with_an_ellipsis() {
        assert_eq!(truncate_end("hello", 0), "");
        assert_eq!(truncate_end("hello", 1), "…");
        assert_eq!(truncate_end("hello", 4), "hel…");
        // Each CJK cell is two wide; a leftover cell is not filled by splitting the next glyph.
        assert_eq!(truncate_end("日本語テスト", 5), "日本…");
        assert_eq!(truncate_end("日本語", 2), "…");
        assert_eq!(truncate_end("日本語", 3), "日…");
        assert_eq!(cell_width(&truncate_end("日本語テスト", 5)), 5);
    }

    #[test]
    fn wrap_hanging_leaves_a_fitting_line_untouched() {
        assert_eq!(wrap_hanging("  hello", 20), vec!["  hello".to_string()]);
        assert_eq!(wrap_hanging("", 10), vec![String::new()]);
        assert!(wrap_hanging("hello", 0).is_empty());
    }

    #[test]
    fn wrap_hanging_continues_at_the_source_indent() {
        assert_eq!(
            wrap_hanging("  hello world", 10),
            vec!["  hello".to_string(), "  world".to_string()]
        );
        assert_eq!(
            wrap_hanging("    - long list item", 14),
            vec!["    - long".to_string(), "    list item".to_string()]
        );
    }

    #[test]
    fn wrap_hanging_breaks_a_word_that_exceeds_the_width() {
        assert_eq!(
            wrap_hanging("  abcdefghij", 6),
            vec![
                "  abcd".to_string(),
                "  efgh".to_string(),
                "  ij".to_string()
            ]
        );
        assert_eq!(
            wrap_hanging("日本語", 5),
            vec!["日本".to_string(), "語".to_string()]
        );
        // Indent leaves one leftover cell; a 2-cell glyph still advances.
        let parts = wrap_hanging("    日", 5);
        assert!(parts.iter().any(|p| p.contains('日')), "{parts:?}");
    }

    #[test]
    fn wrap_hanging_help_topics_fit_an_eighty_column_inner_width() {
        const INNER: usize = 76; // 80 − borders − padding
        for topic in HelpTopic::all() {
            if topic == HelpTopic::About {
                continue;
            }
            for (i, line) in super::super::screens::help::help_topic_body(topic)
                .lines()
                .enumerate()
            {
                for part in wrap_hanging(line, INNER) {
                    assert!(
                        cell_width(&part) <= INNER,
                        "{topic:?} line {i} overflowed: {part:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn fit_hints_leaves_a_fitting_line_untouched() {
        let text = "↑↓ scroll  ·  d download  ·  Esc/q back";
        assert_eq!(fit_hints(text, 80), text);
        assert_eq!(fit_hints("", 10), "");
        assert_eq!(fit_hints("Esc/q back", 0), "");
    }

    #[test]
    fn fit_hints_drops_middle_items_to_keep_the_leave_key() {
        let text = "↑↓ scroll  ·  d download  ·  Esc/q back";
        // 30 cells: "↑↓ scroll  ·  Esc/q back" is 24; adding download (15 + sep 5) does not fit.
        assert_eq!(fit_hints(text, 30), "↑↓ scroll  ·  Esc/q back");
        assert_eq!(fit_hints(text, 12), "Esc/q back");
    }

    #[test]
    fn fit_hints_marks_a_leave_key_that_itself_cannot_fit() {
        assert_eq!(fit_hints("Esc/q back", 6), "Esc/q…");
        assert_eq!(
            fit_hints("↑↓←→ PgUp/Dn scroll  ·  Esc/q back", 8),
            "Esc/q b…"
        );
    }

    #[test]
    fn fit_top_bar_drops_the_name_when_the_shortcuts_need_the_row() {
        let fit = fit_top_bar(40);
        assert!(fit.name.is_none());
        assert_eq!(fit.items.len(), 4);
        assert_eq!(fit.items[0].key, "g");
        assert_eq!(fit.items[0].x, 5);
    }

    #[test]
    fn fit_top_bar_keeps_the_name_only_when_both_halves_fit() {
        assert!(fit_top_bar(42).name.is_none());
        let fit = fit_top_bar(43);
        assert_eq!(fit.name, Some(" gistui"));
        assert_eq!(fit.items.len(), 4);
        assert_eq!(fit.items[0].x, 8); // 7-cell name + 1-cell gap
        let wide = fit_top_bar(60);
        assert_eq!(wide.name, Some(" gistui"));
        assert_eq!(wide.items.len(), 4);
    }

    #[test]
    fn fit_top_bar_drops_leading_shortcuts_when_they_cannot_all_fit() {
        let fit = fit_top_bar(33);
        assert!(fit.name.is_none());
        assert_eq!(fit.items.len(), 3);
        assert_eq!(fit.items[0].key, "P");
        assert_eq!(
            fit.items
                .iter()
                .map(|item| format!("({}){}", item.key, item.rest))
                .collect::<Vec<_>>(),
            vec!["(P)ins", "(C)onfig", "(?)Help"]
        );
    }

    #[test]
    fn fit_title_drops_the_pane_name_to_keep_a_state_segment() {
        // 25 cells: too narrow for `[1] Local (6/15) ⚑ · sort:match` (31), wide enough once
        // the name goes.
        assert_eq!(fit_title(&local_title(), 25), "[1] (6/15) ⚑ · sort:match");
        assert_eq!(narrowest_width_showing(&local_title(), "sort:match"), 25);
    }

    #[test]
    fn fit_title_keeps_the_pane_name_when_dropping_it_buys_no_state() {
        for width in 18..=24 {
            let fitted = fit_title(&local_title(), width);
            assert_eq!(fitted, "[1] Local (6/15) ⚑", "width {width}");
        }
        // Wide enough for everything: the name is back even though eliding it would leave
        // room for a longer path.
        assert!(fit_title(&local_title(), 38).starts_with("[1] Local (6/15) ⚑"));
    }

    #[test]
    fn fit_title_prefers_a_whole_short_head_over_a_clipped_full_one() {
        for width in 12..=17 {
            assert_eq!(
                fit_title(&local_title(), width),
                "[1] (6/15) ⚑",
                "width {width}"
            );
        }
    }

    #[test]
    fn fit_title_without_a_short_head_is_unchanged() {
        let mut title = local_title();
        title.short_head = None;
        assert_eq!(fit_title(&title, 26), "[1] Local (6/15) ⚑ · …stui");
    }

    #[test]
    fn fit_title_joins_every_segment_when_it_fits() {
        let full = "[1] Local (6/15) ⚑ · sort:match · /md · ~/code/gistui";
        assert_eq!(fit_title(&local_title(), 200), full);
        // Every mark the title draws is single-width, so cells and characters agree — the
        // exact fit is the character count, with nothing to spare.
        assert_eq!(cell_width(full), full.chars().count());
        assert_eq!(fit_title(&local_title(), cell_width(full)), full);
        assert_ne!(fit_title(&local_title(), full.chars().count() - 1), full);
    }

    #[test]
    fn fit_title_drops_the_cwd_before_the_state_segments() {
        assert_eq!(
            fit_title(&local_title(), 38),
            "[1] Local (6/15) ⚑ · sort:match · /md"
        );
    }

    #[test]
    fn fit_title_elides_the_context_when_part_of_it_fits() {
        let mut title = PaneTitleVm::new("[1] Local (6/15)".into());
        title.push("sort:match");
        title.context = Some("~/code/some-org/proj".into());
        let fitted = fit_title(&title, 40);
        assert_eq!(fitted, "[1] Local (6/15) · sort:match · …rg/proj");
        assert_eq!(cell_width(&fitted), 40);
    }

    #[test]
    fn fit_title_never_elides_a_state_segment() {
        let mut title = PaneTitleVm::new("[2] Gists (2)".into());
        title.push("all");
        title.push("match");
        title.push_filter("somequery");
        for width in 0..60 {
            let fitted = fit_title(&title, width);
            if fitted.contains('…') {
                assert!(
                    fitted.ends_with('…') && !fitted.contains(" · "),
                    "state segment elided at width {width}: {fitted:?}"
                );
            }
        }
        assert_eq!(fit_title(&title, 24), "[2] Gists (2) · all");
    }

    #[test]
    fn fit_title_never_ends_in_a_dangling_separator() {
        let mut blank_context = local_title();
        blank_context.context = Some(String::new());
        for title in [local_title(), blank_context] {
            for width in 0..80 {
                let fitted = fit_title(&title, width);
                assert!(cell_width(&fitted) <= width, "overflowed at width {width}");
                assert!(
                    !fitted.ends_with('·') && !fitted.ends_with(' '),
                    "dangling separator at width {width}: {fitted:?}"
                );
                assert!(
                    !fitted.starts_with('·') && !fitted.starts_with(' '),
                    "leading separator at width {width}: {fitted:?}"
                );
            }
        }
    }

    #[test]
    fn fit_title_clips_the_head_when_nothing_fits() {
        assert_eq!(fit_title(&local_title(), 0), "");
        // Narrower than the short head too, so there is nothing left but a clipped head.
        assert_eq!(fit_title(&local_title(), 9), "[1] Loca…");
        // At 17 cells the short head fits whole — keeping the marker costs the pane name
        // rather than the marker.
        assert_eq!(fit_title(&local_title(), 17), "[1] (6/15) ⚑");
    }

    #[test]
    fn fit_title_keeps_the_anchor_marker_at_every_width_the_head_fits() {
        let anchored = local_title();
        let mut plain = anchored.clone();
        // The real builder derives both heads from the same state, so a control that strips the
        // anchor has to strip it from the fallback too — otherwise the short head smuggles it
        // back in at the widths where it is used.
        plain.segments[0] = "[1] Local (6/15)".into();
        plain.short_head = Some("[1] (6/15)".into());
        for width in cell_width(&anchored.segments[0])..80 {
            let with = fit_title(&anchored, width);
            let without = fit_title(&plain, width);
            assert!(
                with.contains('⚑'),
                "anchor dropped at width {width}: {with}"
            );
            assert!(!without.contains('⚑'), "phantom anchor at width {width}");
        }
    }
}
