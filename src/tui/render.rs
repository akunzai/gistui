use super::view_model::PaneTitleVm;
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

pub(super) fn render(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    *layout = MouseLayout::default();
    // Paint the full canvas so every unfilled cell uses the theme background (no-op for dark
    // theme where bg=Reset, effective for light theme which sets a grey canvas).
    frame.render_widget(
        Block::default().style(state.theme.base_style()),
        frame.area(),
    );
    // Pure presentation seam (issues #241 / #250): every screen paints from the view model.
    // Pin sync IO is never done here — only cache reads.
    let vm = super::build_view_model(state);
    render_screen_vm(frame, state, &vm.screen, &vm.chrome, layout);
    if let Some(ref msg) = vm.chrome.bg_task_msg {
        render_loading_overlay(frame, msg, vm.chrome.spinner_frame, &state.theme);
    }
}

/// Paints one `ScreenVm`. Shared by `render()` (the primary per-frame path) and
/// `render_palette_vm` (the palette's already-built background, issue #272) — one seam, two
/// real callers, so a new `Screen` variant only needs wiring here once.
pub(crate) fn render_screen_vm(
    frame: &mut Frame,
    state: &AppState,
    screen: &super::ScreenVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    match screen {
        ScreenVm::List(list) => render_list(frame, state, list, chrome, layout),
        ScreenVm::Gists(gists) => render_gists(frame, state, gists, chrome, layout),
        ScreenVm::GistDetail(detail) => render_detail(frame, state, detail, chrome, layout),
        ScreenVm::Revisions(revs) => render_revisions(frame, state, revs, chrome, layout),
        ScreenVm::Config(config) => render_config(frame, state, config, chrome, layout),
        ScreenVm::Diff(diff) => render_diff(frame, state, diff, chrome, layout),
        ScreenVm::Preview(preview) => render_preview(frame, state, preview, chrome, layout),
        ScreenVm::Pins(pins) => render_pins(frame, state, pins, chrome, layout),
        ScreenVm::Confirm(confirm) => render_confirm(frame, state, confirm, chrome, layout),
        ScreenVm::Help(help) => render_help(frame, state, help, chrome, layout),
        ScreenVm::Palette(palette) => render_palette(frame, state, palette, chrome, layout),
    }
}

pub(super) fn render_close_button(frame: &mut Frame, outer: Rect, theme: &Theme) -> Rect {
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

/// Cross-screen top-bar shortcuts: bracketed hotkey letter + label. Kept in one place so the
/// click hit-rect math and the rendered text can never drift apart. Order matches the
/// right-aligned strip: Gists · Pins · Config · Help (Config sits immediately left of Help,
/// same as duodiff).
const TOP_BAR_ITEMS: [(&str, &str); 4] =
    [("G", "ists"), ("P", "ins"), ("C", "onfig"), ("?", "Help")];

/// Height of the persistent top bar rendered on every screen except the transient `Confirm`
/// y/n modal (which keeps its full-bleed diff/gist-info background — see `render_confirm`).
const TOP_BAR_HEIGHT: u16 = 1;

/// Renders the cross-screen top bar — ` gistui` on the left,
/// `(G)ists (P)ins (C)onfig (?)Help` right-aligned — into the top row of `area`, then returns
/// the remaining rect below it for the caller's existing content/footer layout (otherwise
/// unchanged). The icons render as plain text even with the mouse disabled, so the shortcuts
/// stay visible; their hit-rects are only recorded in `layout` when `mouse_enabled`, matching
/// every other clickable region.
pub(super) fn render_top_bar(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    mouse_enabled: bool,
    layout: &mut MouseLayout,
) -> Rect {
    if area.height == 0 {
        return area;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(TOP_BAR_HEIGHT), Constraint::Min(0)])
        .split(area);
    let bar = chunks[0];

    frame.render_widget(Paragraph::new(" gistui").style(theme.base_style()), bar);

    let key_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(theme.fg);
    let widths: Vec<u16> = TOP_BAR_ITEMS
        .into_iter()
        .map(|(k, rest)| (k.chars().count() + rest.chars().count() + 2) as u16) // "(" + key + ")" + rest
        .collect();
    const GAP: u16 = 2;
    let total: u16 = widths.iter().sum::<u16>() + GAP * (TOP_BAR_ITEMS.len() as u16 - 1);
    let mut x = bar.right().saturating_sub(total + 1); // 1-column right margin

    for (i, (key, rest)) in TOP_BAR_ITEMS.into_iter().enumerate() {
        let w = widths[i].min(bar.right().saturating_sub(x));
        let rect = Rect::new(x, bar.y, w, 1);
        let spans = vec![
            Span::styled("(", label_style),
            Span::styled(key.to_string(), key_style),
            Span::styled(format!("){rest}"), label_style),
        ];
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(theme.base_style()),
            rect,
        );
        if mouse_enabled {
            match i {
                0 => layout.top_bar_gists = Some(rect),
                1 => layout.top_bar_pins = Some(rect),
                2 => layout.top_bar_config = Some(rect),
                _ => layout.top_bar_help = Some(rect),
            }
        }
        x += widths[i] + GAP;
    }

    chunks[1]
}

/// A count suffix for a list title: `(N)` normally, or `(shown/total)` when a filter has
/// narrowed the list (`shown < total`). Extends the existing `Files (N)` / `Comments (N)`
/// convention to the other panes consistently.
pub(super) fn count_label(shown: usize, total: usize) -> String {
    if shown < total {
        format!("({shown}/{total})")
    } else {
        format!("({total})")
    }
}

/// Separator between the segments of a pane title.
const TITLE_SEP: &str = " · ";

/// Shortest elided context worth painting: `…` plus at least three cells.
const MIN_ELIDED_WIDTH: usize = 4;

/// Width of `text` in terminal cells — the same measure ratatui truncates a title by, so a
/// double-width glyph (`⚓`) is not silently clipped by the block after we let it through.
fn cell_width(text: &str) -> usize {
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
pub(super) fn fit_title(title: &PaneTitleVm, width: usize) -> String {
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
struct FittedSegments {
    text: String,
    used: usize,
    /// Segments joined whole. Fewer than `segments.len()` means state was dropped.
    shown: usize,
}

/// Join `segments` greedily, stopping at the first one that does not fit.
fn fit_segments(segments: &[String], width: usize) -> FittedSegments {
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
fn elide_start(text: &str, room: usize) -> String {
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
const ELLIPSIS: &str = "…";

/// Highlight prefix every row list paints (`▶` plus a space). Kept in one place so the
/// truncation budget and the widget's `highlight_symbol` cannot drift.
pub(super) const LIST_HIGHLIGHT_SYMBOL: &str = "▶ ";

/// Borders (2) + `Padding::horizontal(1)` (2) around a list pane's inner rows.
const LIST_CHROME_CELLS: u16 = 4;

/// Width-aware end truncation (issue #340): a value that fits is returned unchanged; a
/// value that does not is cut on a character boundary and marked with `…`. Wide glyphs
/// are never split — a leftover cell is left empty rather than showing half a character.
pub(super) fn truncate_end(text: &str, width: usize) -> String {
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

/// Title sitting on a bordered block: the two corner glyphs are not available.
pub(super) fn fit_block_title(title: &str, area_width: u16) -> String {
    truncate_end(title, area_width.saturating_sub(2) as usize)
}

/// Horizontal offset applied to one list row: only the selected row moves (issue #341).
pub(super) fn row_hscroll(selected: Option<usize>, index: usize, hscroll: u16) -> u16 {
    if selected == Some(index) {
        hscroll
    } else {
        0
    }
}

/// Visible list-row text: horizontal scroll first, then truncate to the inner content
/// width (borders, padding, and the highlight symbol) so ratatui cannot silently clip.
/// A non-zero offset keeps a leading `…` so the skip is visible (issue #341).
pub(super) fn visible_list_row(label: &str, hscroll: u16, pane_width: u16) -> String {
    let inner = pane_width.saturating_sub(LIST_CHROME_CELLS) as usize;
    let room = inner.saturating_sub(cell_width(LIST_HIGHLIGHT_SYMBOL));
    let scrolled = hscroll_str(label, hscroll);
    if hscroll == 0 {
        return truncate_end(&scrolled, room);
    }
    let ellipsis_width = cell_width(ELLIPSIS);
    if room < ellipsis_width {
        return String::new();
    }
    format!(
        "{ELLIPSIS}{}",
        truncate_end(&scrolled, room - ellipsis_width)
    )
}

fn gist_badge_prefix(starred: bool, forked: bool) -> String {
    let mut prefix = String::new();
    if starred {
        prefix.push('★');
        prefix.push(' ');
    }
    if forked {
        prefix.push('⑂');
        prefix.push(' ');
    }
    prefix
}

fn gist_owner_prefix(group: &GistGroup, current_user: Option<&str>) -> String {
    if group.owner_login.is_empty() {
        return String::new();
    }
    if current_user == Some(group.owner_login.as_str()) {
        return String::new();
    }
    format!("@{}  ", group.owner_login)
}

pub(super) fn gist_group_row_label(
    g: &GistGroup,
    now: u64,
    sort: GistGroupSort,
    counts: (u32, u32, u32),
    starred: bool,
    current_user: Option<&str>,
) -> String {
    let (comments, stars, forks) = counts;
    let desc = if g.description.trim().is_empty() {
        "(no description)".to_string()
    } else {
        g.description.clone()
    };
    // Visibility is dropped from the row — it's surfaced by the `v` filter, the title's
    // `type:` label, and the detail view. 📄 / 🕒 distinguish file count from the age.
    // The 🕒 age tracks the active sort key (created vs updated) so the column the rows
    // are ordered by is the one shown; it's a relative age (single largest unit).
    let timestamp = match sort {
        GistGroupSort::Updated => &g.updated_at,
        GistGroupSort::Created => &g.created_at,
    };
    let age = crate::domain::parse_rfc3339_to_unix(timestamp)
        .map(|t| crate::domain::humanize_age(now as i64 - t as i64))
        .unwrap_or_else(|| "?".into());
    // Only surface markers when non-zero so the common quiet rows stay clean.
    let comments_seg = if comments > 0 {
        format!("  💬 {comments}")
    } else {
        String::new()
    };
    let stars_seg = if stars > 0 {
        format!("  ☆ {stars}")
    } else {
        String::new()
    };
    let forks_seg = if forks > 0 {
        format!("  ⑂ {forks}")
    } else {
        String::new()
    };
    format!(
        "{}{}{}  {}  📄 {}{}{}{}  🕒 {}",
        gist_badge_prefix(starred, g.fork_of_id.is_some()),
        gist_owner_prefix(g, current_user),
        g.id,
        desc,
        g.file_count,
        comments_seg,
        stars_seg,
        forks_seg,
        age
    )
}

fn gist_info_counts_seg(comments: u32, stars: u32, forks: u32) -> String {
    let mut parts = Vec::new();
    if stars > 0 {
        parts.push(format!("☆ {stars}"));
    }
    if forks > 0 {
        parts.push(format!("⑂ {forks}"));
    }
    if comments > 0 {
        parts.push(format!("💬 {comments}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{} · ", parts.join(" · "))
    }
}

/// One-line info summary for the detail header.
pub(super) fn gist_info_line(
    group: &GistGroup,
    now: u64,
    current_user: Option<&str>,
    starred: bool,
    counts: (u32, u32, u32),
) -> String {
    let (comments, stars, forks) = counts;
    let star_seg = if starred { "★ starred · " } else { "" };
    let vis = if group.public { "public" } else { "secret" };
    let owner_seg = gist_owner_prefix(group, current_user);
    let counts_seg = gist_info_counts_seg(comments, stars, forks);
    let created = crate::domain::parse_rfc3339_to_unix(&group.created_at)
        .map(|t| crate::domain::humanize_age(now as i64 - t as i64))
        .unwrap_or_else(|| "?".into());
    let updated = crate::domain::parse_rfc3339_to_unix(&group.updated_at)
        .map(|t| crate::domain::humanize_age(now as i64 - t as i64))
        .unwrap_or_else(|| "?".into());
    // The file count lives in the "Files (N)" section header below, so it's omitted here.
    // The detail view has room, so show the full gist id (not a truncated prefix).
    let fork_seg = group
        .fork_of_id
        .as_deref()
        .map(|id| format!("fork of {id} · "))
        .unwrap_or_default();
    format!(
        "{star_seg}{owner_seg}{vis} · {counts_seg}created {created} · updated {updated} · {fork_seg}{}",
        group.id
    )
}

/// Current Unix time in seconds (saturating to 0 before the epoch); used for relative-age labels.
pub(super) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Info + file-list block for a gist (reused as the compaction-confirm background).
/// First visible file index so `cursor` stays within a `visible_rows`-high window over
/// `count` files. Returns 0 when everything fits or `visible_rows == 0`.
pub(super) fn file_list_scroll(cursor: usize, visible_rows: usize, count: usize) -> usize {
    if visible_rows == 0 || count <= visible_rows || cursor < visible_rows {
        return 0;
    }
    (cursor + 1).saturating_sub(visible_rows)
}

/// Build the numbered file rows for the gist's file list (detail Files tab and the
/// compaction-confirm background). The first nine files are numbered to match the 1–9 preview
/// keys; the rest are bulleted. With `highlight_cursor`, the `cursor` row is reverse-styled.
/// Windows to `visible_rows` rows starting at `offset`.
pub(super) fn file_rows(
    files: &[String],
    cursor: usize,
    offset: usize,
    visible_rows: usize,
    highlight_cursor: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    for (i, f) in files
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_rows.max(1))
    {
        let marker = if i < 9 {
            format!("{}.", i + 1)
        } else {
            "·".to_string()
        };
        if highlight_cursor && i == cursor {
            rows.push(Line::from(Span::styled(
                format!("▸ {marker} {f}"),
                Style::default()
                    .fg(theme.fg_on_accent)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )));
        } else {
            rows.push(Line::from(format!("  {marker} {f}")));
        }
    }
    rows
}

/// Compaction-confirm background from a pure compact-gist view model.
pub(super) fn render_compact_gist_bg_vm(
    frame: &mut Frame,
    area: Rect,
    bg: &super::view_model::CompactGistBgVm,
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
pub(super) const MINIMAL_HINT: &str = "; Menu · Ctrl+p Palette";

/// Footer text + whether to colourise it: a one-shot `state.status` message (shown plain) when
/// present, else the colourised key `hints`. Shared by every screen so action results/errors
/// surface consistently and are never swallowed by a hard-coded footer (see #72, #66).
pub(super) fn footer_with_status(status: Option<&str>, hints: &str) -> (String, bool) {
    match status {
        Some(message) => (message.to_string(), false),
        None => (hints.to_string(), true),
    }
}

pub(super) use super::text::hscroll_str;

/// Compose the full list-row string (including pin mark) that paint and hscroll max must share.
pub(super) fn marked_row_text(base: String, mark: crate::ranking::MatchMark) -> String {
    match mark {
        crate::ranking::MatchMark::Pinned => format!("📌 {base}"),
        crate::ranking::MatchMark::ExactFilename | crate::ranking::MatchMark::None => base,
    }
}

/// Gist file-list row **without** the live star mark (fork badge still applied). Used as the
/// shared base for paint/hscroll; star is layered in [`gist_row_display`] so both stay aligned.
pub(super) fn gist_row_label(g: &RankedGistFile, view: GistView) -> String {
    let base = match view {
        GistView::Description => {
            if g.file.description.trim().is_empty() {
                g.file.filename.clone()
            } else {
                format!("{} — {}", g.file.filename, g.file.description)
            }
        }
        GistView::Id => format!("{} / {}", g.file.gist_id, g.file.filename),
    };
    format!("{}{}", gist_badge_prefix(false, g.file.is_fork()), base)
}

/// Greedy word-wrap line count, matching how `Paragraph` with `Wrap { trim: true }` breaks
/// space-separated words at `width`. Used to size the footer block to its content.
pub(super) fn wrap_line_count(text: &str, width: u16) -> u16 {
    if width == 0 {
        return 1;
    }
    let width = width as usize;
    let mut lines: u16 = 1;
    let mut col = 0usize;
    for word in text.split_whitespace() {
        let w = word.chars().count();
        if col == 0 {
            col = w.min(width);
        } else if col + 1 + w <= width {
            col += 1 + w;
        } else {
            lines = lines.saturating_add(1);
            col = w.min(width);
        }
    }
    lines
}

/// Height to reserve for a screen's footer `Layout` row: `0` when both `text` and `title` are
/// empty (the footer fully collapses), else the wrapped line count for `text` plus one row when
/// `title` is non-empty (ratatui's [`Block::title`] always consumes a row, even without borders).
pub(super) fn footer_height(text: &str, width: u16, title: &str) -> u16 {
    if text.is_empty() && title.is_empty() {
        return 0;
    }
    let content = if text.is_empty() {
        0
    } else {
        wrap_line_count(text, width.saturating_sub(2)).max(1)
    };
    content + u16::from(!title.is_empty())
}

/// Colour a command key by what its action does, so destructive and mutating keys stand apart
/// from plain navigation at a glance: destructive (delete/remove/unpin) → Red, write/sync
/// (download/upload/create/sync/…) → Green, everything else (navigation/view) → Cyan. Matched on
/// whole label words so e.g. `pins` does not read as the `pin` action.
pub(super) fn action_color(label: &str, theme: &Theme) -> Color {
    const DESTRUCTIVE: [&str; 3] = ["delete", "remove", "unpin"];
    const WRITE: [&str; 10] = [
        "download", "upload", "create", "new", "sync", "push", "pull", "pin", "edit", "desc",
    ];
    let mut color = theme.accent;
    for word in label.split_whitespace() {
        let word = word.to_ascii_lowercase();
        if DESTRUCTIVE.contains(&word.as_str()) {
            return theme.del_color;
        }
        if WRITE.contains(&word.as_str()) {
            color = theme.write_color;
        }
    }
    color
}

/// Style a footer command string: the leading key token of each `·`-separated item is accented by
/// its action category (see [`action_color`]); the descriptive label keeps the terminal's default
/// brightness so it stays legible, and only the separators are dimmed. Every input character is
/// preserved verbatim so `wrap_line_count` sizing stays exact.
pub(super) fn hint_line(text: &str, theme: &Theme) -> Line<'static> {
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
                let key = Style::default().fg(action_color(label, theme));
                spans.push(Span::styled(k.to_string(), key));
                spans.push(Span::raw(label.to_string()));
            }
            None => spans.push(Span::styled(
                rest.to_string(),
                Style::default().fg(action_color("", theme)),
            )),
        }
    }
    Line::from(spans)
}

/// The shared footer block: plain text with horizontal padding, no border (the old dim top
/// divider was removed to reclaim a row and keep the chrome minimal). The repo URL, app
/// version, and update-check status used to live in the footer but have moved to Help → About
/// (see `about_topic_lines`).
pub(super) fn footer_block(title: &str, theme: &Theme) -> Block<'static> {
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
pub(super) fn render_footer(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    text: &str,
    colored: bool,
    theme: &Theme,
    _layout: &mut MouseLayout,
) {
    let para = if colored {
        Paragraph::new(hint_line(text, theme))
    } else {
        Paragraph::new(text.to_string())
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
pub(super) fn render_footer_line(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    line: Line,
    theme: &Theme,
    _layout: &mut MouseLayout,
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
pub(super) fn input_line(prefix: &str, input: &TextInput, suffix: &str) -> Line<'static> {
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

/// Builds the visible, coloured slice of a unified diff (additions green, deletions red,
/// `---`/`+++` headers bold). Scrolling is applied here by hand — skip `vscroll` lines and
/// drop `hscroll` leading chars per line — rather than via `Paragraph::scroll`, whose
/// styled-line handling leaves redraw artifacts in ratatui 0.26.
/// Skips `hscroll` characters across an ordered list of spans, preserving styles.
pub(super) fn apply_hscroll_spans(spans: Vec<Span<'static>>, hscroll: usize) -> Line<'static> {
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
pub(super) fn inline_del_line(
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
pub(super) fn inline_ins_line(
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
pub(super) fn header_line(line: &str, hscroll: usize, theme: &Theme) -> Line<'static> {
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
pub(super) fn diff_view_highlighted(
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
            super::highlight::highlight_buffer(ext, &contents, theme)
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
pub(super) const CREATE_DESC_PREFIX: &str = "Description (optional): ";
pub(super) const CREATE_DESC_SUFFIX: &str = "   ·  Enter next  ·  Esc cancel";

/// Overlay a vertical scrollbar on the right edge of a bordered, scrollable text pane when
/// its `total` lines overflow the inner viewport. `offset` is the index of the topmost
/// visible line, so the thumb reflects the real scroll position (not a selection index).
pub(super) fn render_text_scrollbar(frame: &mut Frame, area: Rect, total: usize, offset: usize) {
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

/// Render just the diff content pane (no footer) into `area` from a [`DiffVm`].
pub(super) fn render_diff_pane_vm(
    frame: &mut Frame,
    area: Rect,
    diff: &super::view_model::DiffVm,
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

pub(super) fn is_json_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

/// Centered modal rect sized to fit `body` (clamped to the frame).
fn centered_modal_rect(area: Rect, body: &str) -> Rect {
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

fn modal_block(title: &str, area_width: u16, border: Color, theme: &Theme) -> Block<'static> {
    Block::default()
        .title(fit_block_title(title, area_width))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(theme.base_style())
        .padding(Padding::horizontal(1))
}

pub(super) fn render_centered_modal(
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
pub(super) fn render_centered_modal_input(
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
/// event-loop tick, ~150ms). Braille dots are as widely supported as the emoji already used
/// across the UI (📭/🔍/⏳), so no ASCII fallback is added here.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The spinner glyph for the given tick. `frame` may be any value; it is reduced modulo the
/// frame count.
pub(super) fn spinner_glyph(frame: usize) -> &'static str {
    SPINNER_FRAMES[frame % SPINNER_FRAMES.len()]
}

/// Column width for palette key hints: at least one char, wide enough for the longest
/// visible key (`Enter`, `Ctrl+p`, …) so labels never run into the hint.
#[cfg(test)]
pub(super) fn palette_key_width(items: &[&PaletteItem]) -> usize {
    items
        .iter()
        .map(|item| item.key_hint.chars().count())
        .max()
        .unwrap_or(1)
        .max(1)
}

/// One palette row from a full [`PaletteItem`] (unit tests).
#[cfg(test)]
pub(super) fn palette_row_line(
    item: &PaletteItem,
    key_width: usize,
    theme: &Theme,
    row_style: Style,
) -> Line<'static> {
    palette_row_spans(&item.key_hint, &item.label, key_width, theme, row_style)
}

/// Shared by palette paint (`PaletteVm` rows) and test helpers.
pub(super) fn palette_row_spans(
    key_hint: &str,
    label: &str,
    key_width: usize,
    theme: &Theme,
    row_style: Style,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<key_width$}  ", key_hint, key_width = key_width),
            Style::default().fg(action_color(label, theme)),
        ),
        Span::styled(label.to_string(), row_style),
    ])
}

/// A centered "Working…" box shown while a blocking `gh` action runs.
pub(super) fn render_loading_overlay(
    frame: &mut Frame,
    msg: &str,
    spinner_frame: usize,
    theme: &Theme,
) {
    let body = format!("{} {msg}", spinner_glyph(spinner_frame));
    render_centered_modal(frame, "Working…", &body, theme.accent, theme);
}

/// Civil date (year, month, day) from a day count since the Unix epoch — Howard Hinnant's
/// algorithm. UTC, leap-second agnostic (fine for display).
pub(super) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub(super) fn format_unix_utc(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86400));
    let rem = secs.rem_euclid(86400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

pub(super) fn file_mtime_label(path: &std::path::Path) -> String {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| format_unix_utc(d.as_secs() as i64))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Normalises the gist API's RFC3339 `updated_at` (e.g. `2026-06-08T11:06:18Z`) to
/// `2026-06-08 11:06 UTC` for display alongside the local file's mtime.
pub(super) fn gist_time_label(updated_at: &str) -> String {
    if updated_at.is_empty() {
        "unknown".to_string()
    } else if updated_at.len() >= 16 {
        format!("{} UTC", updated_at[..16].replace('T', " "))
    } else {
        updated_at.to_string()
    }
}

// ---------------------------------------------------------------------------
// Pinned-sync helpers (Task 9 + Task 10)
// ---------------------------------------------------------------------------

pub(super) fn diff_labels(
    local_path: Option<&std::path::Path>,
    gist: &GistFile,
) -> (String, String) {
    let local_name = local_path
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("(none)");
    let local_time = local_path
        .map(file_mtime_label)
        .unwrap_or_else(|| "—".to_string());
    let local_label = format!("local: {local_name} ({local_time})");
    let gist_label = format!(
        "gist {} / {} ({})",
        gist.gist_id,
        gist.filename,
        gist_time_label(&gist.updated_at)
    );
    (local_label, gist_label)
}

/// Orientation for the `Enter` diff preview, driven by the focused pane: focusing the gist
/// pane frames it as a *download* (old = local, new = gist), focusing the local pane frames
/// it as an *upload* (old = gist, new = local). The dedicated `d`/`u` actions keep their own
/// fixed orientation; this only affects the read-only preview.
pub(super) fn preview_diff_text(
    upload_orientation: bool,
    local_label: &str,
    local_content: &str,
    gist_label: &str,
    remote: &str,
    ignore_trailing_newline: bool,
) -> String {
    if upload_orientation {
        crate::diff::unified_diff(
            gist_label,
            remote,
            local_label,
            local_content,
            ignore_trailing_newline,
        )
    } else {
        crate::diff::unified_diff(
            local_label,
            local_content,
            gist_label,
            remote,
            ignore_trailing_newline,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    /// Concatenates every cell's symbol in row-major order (no separators) — enough to assert
    /// a known label/title landed somewhere in the frame without pinning exact coordinates.
    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer.content().iter().map(|c| c.symbol()).collect()
    }

    /// Issue #340: a value that fits, including exactly, is left untouched.
    #[test]
    fn truncate_end_leaves_a_fitting_value_untouched() {
        assert_eq!(truncate_end("", 0), "");
        assert_eq!(truncate_end("hello", 5), "hello");
        assert_eq!(truncate_end("hello", 10), "hello");
        assert_eq!(truncate_end("日本語", 6), "日本語");
    }

    fn list_row_pane_width(content_cells: u16) -> u16 {
        LIST_CHROME_CELLS + cell_width(LIST_HIGHLIGHT_SYMBOL) as u16 + content_cells
    }

    /// Issue #341: horizontal scroll is applied before end-truncation; an unscrolled
    /// row that fits is returned unchanged.
    #[test]
    fn visible_list_row_leaves_an_unscrolled_value_intact() {
        let width = list_row_pane_width(20);
        assert_eq!(visible_list_row("AGENTS.md", 0, width), "AGENTS.md");
    }

    /// Issue #341: a horizontally offset row keeps a leading `…` so the skip is visible.
    #[test]
    fn visible_list_row_marks_a_horizontal_offset_with_a_leading_ellipsis() {
        let width = list_row_pane_width(20);
        assert_eq!(visible_list_row("AGENTS.md", 6, width), "….md");
    }

    /// Issue #341: scrolling the selected long path must not eat the start of its siblings.
    #[test]
    fn list_hscroll_leaves_unselected_rows_readable_from_their_start() {
        let mut state = crate::tui::tests::state_with_local_paths(&[
            "/cwd/AGENTS.md",
            "/cwd/docs/adr/0001-appstate-field-visibility.md",
            "/cwd/CHANGELOG.md",
        ]);
        state.focus = FocusPane::Local;
        state.local_index = 1;
        state.local_hscroll = 6;
        let text = render_state(&state);
        assert!(
            text.contains("AGENTS.md"),
            "unselected short row lost its start: {text}"
        );
        assert!(
            text.contains("CHANGELOG.md"),
            "unselected short row lost its start: {text}"
        );
        assert!(
            text.contains("…dr/0001"),
            "selected row must show a leading ellipsis at this offset: {text}"
        );
        assert!(
            !text.contains("docs/adr/0001-appstate-field-visibility.md"),
            "selected row should have scrolled past its head: {text}"
        );
    }

    /// Issue #340: clipping appends `…` and never splits a wide glyph.
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

    /// Builds a real `ScreenVm` from `state` (same seam `render()` uses) and paints it through
    /// `render_screen_vm` — the dispatch under test. Panics (e.g. an unreachable match arm, an
    /// out-of-bounds slice on empty data) fail the test; the returned buffer text lets callers
    /// additionally assert on painted content.
    fn render_state(state: &AppState) -> String {
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let vm = super::super::build_view_model(state);
        let mut layout = MouseLayout::default();
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
        assert!(text.contains("(G)ists"));
        assert!(text.contains("(P)ins"));
        assert!(text.contains("(C)onfig"));
        assert!(text.contains("(?)Help"));
    }

    /// The Local pane title from the bug report, anchored and filtered.
    fn local_title() -> PaneTitleVm {
        let mut title = PaneTitleVm::new("[1] Local (6/15) ⚓".into());
        title.short_head = Some("[1] (6/15) ⚓".into());
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

    /// #338 follow-up: the pane *name* is the head's cheapest part to lose — `[1]` and the
    /// layout still say which pane this is — so a narrow title spends it on `sort:`.
    #[test]
    fn fit_title_drops_the_pane_name_to_keep_a_state_segment() {
        // 26 cells: too narrow for `[1] Local (6/15) ⚓ · sort:match` (32), wide enough once
        // the name goes.
        assert_eq!(fit_title(&local_title(), 26), "[1] (6/15) ⚓ · sort:match");
        assert_eq!(narrowest_width_showing(&local_title(), "sort:match"), 26);
    }

    /// The name is spent on state, never on more cwd — dropping it to widen re-derivable
    /// context would trade something the reader needs for something they already know.
    #[test]
    fn fit_title_keeps_the_pane_name_when_dropping_it_buys_no_state() {
        for width in 19..=25 {
            let fitted = fit_title(&local_title(), width);
            assert_eq!(fitted, "[1] Local (6/15) ⚓", "width {width}");
        }
        // Wide enough for everything: the name is back even though eliding it would leave
        // room for a longer path.
        assert!(fit_title(&local_title(), 38).starts_with("[1] Local (6/15) ⚓"));
    }

    /// Below the full head's own width the short head is a better answer than a clipped one:
    /// a whole abbreviated head beats `[1] Local (6`.
    #[test]
    fn fit_title_prefers_a_whole_short_head_over_a_clipped_full_one() {
        for width in 13..=18 {
            assert_eq!(
                fit_title(&local_title(), width),
                "[1] (6/15) ⚓",
                "width {width}"
            );
        }
    }

    /// A title with no short head keeps the original single-head behaviour.
    #[test]
    fn fit_title_without_a_short_head_is_unchanged() {
        let mut title = local_title();
        title.short_head = None;
        assert_eq!(fit_title(&title, 26), "[1] Local (6/15) ⚓ · …tui");
    }

    #[test]
    fn fit_title_joins_every_segment_when_it_fits() {
        let full = "[1] Local (6/15) ⚓ · sort:match · /md · ~/code/gistui";
        assert_eq!(fit_title(&local_title(), 200), full);
        // `⚓` is two cells wide, so the exact fit is one wider than the character count.
        assert_eq!(fit_title(&local_title(), cell_width(full)), full);
        assert_ne!(fit_title(&local_title(), full.chars().count()), full);
    }

    /// #338: the state a keypress just changed outlives the cwd when the title is too narrow.
    #[test]
    fn fit_title_drops_the_cwd_before_the_state_segments() {
        assert_eq!(
            fit_title(&local_title(), 38),
            "[1] Local (6/15) ⚓ · sort:match · /md"
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

    /// #338: only the context shrinks. A pane with none (the Gists pane) drops whole segments,
    /// so a sort mode or filter is never half-shown. #340: a head that itself cannot fit is
    /// marked with a trailing `…` — that is the only ellipsis this pane can produce.
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

    /// #338: too narrow even for the head clips it, rather than painting an empty title.
    /// #340: that clipped head is marked with `…` so it does not read as the real value.
    #[test]
    fn fit_title_clips_the_head_when_nothing_fits() {
        assert_eq!(fit_title(&local_title(), 0), "");
        // Narrower than the short head too, so there is nothing left but a clipped head.
        assert_eq!(fit_title(&local_title(), 9), "[1] Loca…");
        // The anchor is two cells wide and will not split, but at 17 cells the short head
        // fits whole — keeping the marker costs the pane name rather than the marker.
        assert_eq!(fit_title(&local_title(), 17), "[1] (6/15) ⚓");
    }

    /// #338: the anchor lives in the head, so flipping it changes the title at every width the
    /// head itself survives.
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
                with.contains('⚓'),
                "anchor dropped at width {width}: {with}"
            );
            assert!(!without.contains('⚓'), "phantom anchor at width {width}");
        }
    }

    /// #338: in the terminal width from the bug report the Local title keeps sort / filter /
    /// anchor and lets the cwd take the truncation, so flipping the anchor is visible on screen.
    #[test]
    fn render_screen_vm_list_title_keeps_state_over_cwd() {
        let mut state = initial_state();
        state.cwd = std::path::PathBuf::from("/cwd/some-org/some-project");
        state.locals = vec![
            crate::domain::LocalCandidate {
                path: state.cwd.join("notes.md"),
                pinned: false,
                modified: None,
            },
            crate::domain::LocalCandidate {
                path: state.cwd.join("a.txt"),
                pinned: false,
                modified: None,
            },
        ];
        state.local_filter_query = "md".into();
        state.anchor = FocusPane::Local;

        // 137 columns as reported; the Local pane gets 40% of that.
        let backend = TestBackend::new(137, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let vm = super::super::build_view_model(&state);
        let mut layout = MouseLayout::default();
        terminal
            .draw(|frame| render_screen_vm(frame, &state, &vm.screen, &vm.chrome, &mut layout))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());

        // A wide glyph owns two cells, so the anchor is followed by a filler cell in the dump.
        assert!(
            text.contains("[1] Local (1/2) ⚓"),
            "local title missing the anchor: {text}"
        );
        assert!(
            text.contains("· sort:match · /md ·"),
            "local title missing state segments: {text}"
        );
        // The cwd is what shortened: only its tail survived, behind an ellipsis.
        assert!(text.contains("…some-project"), "cwd not elided: {text}");
    }

    /// #338 follow-up: in a terminal too narrow for the full head, the pane *name* is what the
    /// title gives up — `sort:` and the anchor survive, and `[1]` still says which pane it is.
    #[test]
    fn render_screen_vm_list_title_drops_the_pane_name_when_narrow() {
        let mut state = initial_state();
        state.cwd = std::path::PathBuf::from("/cwd/some-org/some-project");
        state.locals = vec![crate::domain::LocalCandidate {
            path: state.cwd.join("notes.md"),
            pinned: false,
            modified: None,
        }];
        state.anchor = FocusPane::Local;

        // 70 columns: the Local pane's 40% share cannot hold `[1] Local (1) ⚓ · sort:match`.
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let vm = super::super::build_view_model(&state);
        let mut layout = MouseLayout::default();
        terminal
            .draw(|frame| render_screen_vm(frame, &state, &vm.screen, &vm.chrome, &mut layout))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());

        // A wide glyph owns two cells, so the anchor is followed by a filler cell in the dump.
        assert!(
            text.contains("[1] (1) ⚓"),
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
            gist_id: "abc123def".into(),
            description: description.into(),
            filename: filename.into(),
            public: false,
            updated_at: "2024-01-01T00:00:00Z".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
            owner_login: String::new(),
            fork_of_id: None,
            raw_url: None,
            content_type: None,
            node_id: None,
        }
    }

    fn render_state_size(state: &AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let vm = super::super::build_view_model(state);
        let mut layout = MouseLayout::default();
        terminal
            .draw(|frame| render_screen_vm(frame, state, &vm.screen, &vm.chrome, &mut layout))
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    /// Issue #340: a long gist-pane description is marked with `…` instead of reading as
    /// the real value. Reproduces on the 137-column terminal from the report.
    #[test]
    fn render_screen_vm_list_row_marks_clipped_description() {
        let mut state = initial_state();
        state.gists = vec![gist_file(
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

    /// Issue #340: a wide-character filename is clipped on a character boundary, not mid-glyph.
    #[test]
    fn render_screen_vm_list_row_does_not_split_a_wide_character() {
        let mut state = initial_state();
        state.gists = vec![gist_file(
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

    /// Issue #340: a value that fits the pane is left untouched — no ellipsis.
    #[test]
    fn render_screen_vm_list_row_leaves_a_fitting_label_untouched() {
        let mut state = initial_state();
        state.gists = vec![gist_file("a.txt", "short")];
        let text = render_state_size(&state, 137, 20);
        assert!(
            text.contains("a.txt — short"),
            "fitting label missing: {text}"
        );
        assert!(!text.contains('…'), "fitting label was clipped: {text}");
    }

    /// Issue #340: Gist manager rows use the same truncation as the List pane.
    #[test]
    fn render_screen_vm_gists_row_marks_clipped_description() {
        let mut state = initial_state();
        state.screen = Screen::Gists(Box::default());
        state.gists = vec![gist_file(
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

    /// Issue #340: Pinned Mappings rows use the same truncation as the List pane.
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
        assert!(text.contains("No pinned mappings found"));
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
