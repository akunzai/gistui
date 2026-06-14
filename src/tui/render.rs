use super::*;
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, List, ListItem, ListState, Padding, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
    Frame,
};
use similar::{ChangeTag, TextDiff};

pub(super) fn render(frame: &mut Frame, state: &AppState) {
    match state.screen {
        Screen::List => render_list(frame, state),
        Screen::Diff => render_diff(frame, state),
        Screen::Confirm => render_confirm(frame, state),
        Screen::Preview => render_preview(frame, state),
        Screen::Help => render_help(frame, state),
        Screen::Pins => render_pins(frame, state),
        Screen::Gists => render_gists(frame, state),
        Screen::GistDetail => render_gist_detail(frame, state),
    }
    if let Some(ref msg) = state.bg_task_msg {
        render_loading_overlay(frame, msg, state.spinner_frame);
    }
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

pub(super) fn render_help(frame: &mut Frame, state: &AppState) {
    // The repo URL and version live in the footer on every screen, so help is keys only.
    let body = "\
Navigation
  Tab        switch pane (Local / Gists)
  1 / 2      jump to the Local / Gist pane
  Up/Down    move the selection
  Left/Right scroll a long row horizontally

List screen
  r          toggle recursive file discovery (skips hidden + configured dirs)
  /          filter by filename or description
  v          cycle gist visibility: all / public / secret
  s          cycle the focused pane's sort: match / name / recent
  t          toggle row view: description / id
  a          flip which pane drives match ranking (anchor); the other pane
             re-ranks against the anchor's selection (focus stays put)
             (📌 = pinned pair · bold = same filename)

Actions (on the selected local file + gist)
  Enter      diff the local file against the gist; direction follows the focused
             pane — Gist pane = download view, Local pane = upload view
             (--- old / +++ new; local label = yellow, gist label = blue)
  Space      preview the gist file's content (R in preview to force-refresh)
  d          download the gist into the cwd
  u          upload the local file into the gist
  n          create a new gist from the local file (type a description, then s/p)
  p          pin / unpin the local <-> gist pair
  P          view / manage all pinned mappings (sync status + s/u/d/x)
  S          smart-sync the selected pinned pair (push/pull by modified time)
  X          remove the selected file from its gist (y/n confirm)
  g          open the gist manager (edit description, delete gist)
  e          edit the local file in $EDITOR
  y          copy the selected gist's URL to the system clipboard

Pinned Mappings screen (P)
  Up/Down    move between pins
  Left/Right scroll a long local path horizontally (~ = home)
  Enter      diff the selected pair (then d pull / u push from the diff)
  s          smart-sync (newer side wins; skips if already identical)
  u          force push  (upload local → gist)
  d          force pull  (download gist → local, diff + y/n confirm)
  x          unpin the selected pair
  status     ✓ synced · ↑ local newer · ↓ remote newer · ? unknown
  Each row shows (local <age> · gist <age>) relative modification times.

Diff view (Enter / d / u)
  Up/Down/Left/Right  scroll the diff
  PageUp/Dn  scroll the diff by 10 lines
  c          toggle context: configured radius <-> full file (remembered)
  d / u      download / upload from the diff
  syntax     unchanged context lines are syntax-highlighted by file type
  Esc / q    back

Full-screen preview (Space, or 1-9 in the detail view)
  Up/Down/Left/Right  scroll (Left/Right only when wrap is off)
  PageUp/Dn  scroll by 10 lines
  w          toggle soft line wrapping (remembered for the session)
  y          copy the gist URL · Y copy the file content to the clipboard
  syntax     known file types are syntax-highlighted
  R          re-fetch the content
  Esc / q    back

Upload Confirmation screen (u)
  y          confirm and execute the upload
  n / Esc    cancel the upload
  e          edit / redact the upload content in $EDITOR before upload
  p          (JSON only) toggle pretty-print formatting
  s          (JSON only) toggle recursive key sorting

Gist manager (g)
  Up/Down    move between gists
  Left/Right scroll a long description horizontally
  /          filter gists by description or id
  s          cycle sort: updated / created
  v          cycle visibility: all / public / secret
  e          edit the gist description (Enter apply, Esc cancel)
  Enter      open the gist detail view (info, file list, comments)
  o          open the gist in your web browser
  y          copy the gist's URL to the system clipboard
  c          compact revisions: squash history to one commit (force-push, y/n confirm)
  X          delete the entire gist and all its files (y/n confirm)
  q / Esc    back to the list

Gist detail (Enter from gist manager)
  Tab        switch tab: Files / Comments (one shows at a time; opens on Files)
  Up/Down    move the file cursor (Files tab) or scroll comments (Comments tab)
  PageUp/Dn  page comments / file cursor by 10
  Enter      preview the cursor-selected file (file list focused)
  1-9        preview the content of the Nth file (full-screen; R refresh, q back)
  c          compact revisions (y/n confirm; gist info shown as context)
  o          open the gist in your web browser
  y          copy the gist's URL to the system clipboard
  X          delete the entire gist and all its files (y/n confirm)
  q / Esc    back to the gist manager

General
  Esc / q    close an overlay; from the list, press twice to quit the app
  ?          show this help
  Up/Down    scroll this help text
  NO_COLOR   set this env var to disable syntax highlighting (preview + diff)";

    frame.render_widget(
        Paragraph::new(body).scroll((state.help_scroll, 0)).block(
            Block::default()
                .title("Help (Up/Down scroll) — press any other key to close")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .padding(Padding::horizontal(1)),
        ),
        frame.area(),
    );
}

/// Lowercase file extension of a filename or path string, if any.
fn file_ext(name: &str) -> Option<String> {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
}

/// Language extension for the previewed file, taken from its gist key's filename.
fn preview_ext(state: &AppState) -> Option<String> {
    state
        .preview_gist_key
        .as_ref()
        .and_then(|(_, filename)| file_ext(filename))
}

/// Language extension for the diff's file — the local/target filename both sides share.
fn diff_ext(state: &AppState) -> Option<String> {
    state
        .download_target
        .file_name()
        .or_else(|| state.preview_local.file_name())
        .and_then(|n| n.to_str())
        .and_then(file_ext)
}

/// The preview body as per-line span vectors: syntax-highlighted when the feature is enabled and
/// the file type is known, otherwise one plain span per line.
fn preview_line_spans(state: &AppState) -> Vec<Vec<Span<'static>>> {
    let lines: Vec<String> = state.diff_text.lines().map(str::to_string).collect();
    match (state.syntax_highlight, preview_ext(state)) {
        (true, Some(ext)) => super::highlight::highlight_buffer(&ext, &lines),
        _ => lines.into_iter().map(|l| vec![Span::raw(l)]).collect(),
    }
}

pub(super) fn render_preview(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    // A `R`-refresh fetch error (set via state.status) must surface here, not be swallowed.
    let hints = if state.preview_wrap {
        "↑↓ PgUp/Dn scroll  ·  w wrap [on]  ·  y/Y copy url/content  ·  R refresh  ·  Esc/q back"
    } else {
        "↑↓←→ PgUp/Dn scroll  ·  w wrap [off]  ·  y/Y copy url/content  ·  R refresh  ·  Esc/q back"
    };
    let (footer, colored) = footer_with_status(state.status.as_deref(), hints);
    let footer_lines = wrap_line_count(&footer, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(footer_lines + 1)])
        .split(area);

    // When wrapping, horizontal scroll is meaningless — pin the x offset to 0 so long lines
    // wrap into view instead of being scrolled off-screen.
    let block = Block::default()
        .title(state.preview_title.clone())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .padding(Padding::horizontal(1));
    let line_spans = preview_line_spans(state);
    let total_lines = line_spans.len();
    let paragraph = if state.preview_wrap {
        // Wrapping needs the full line set; vertical scroll goes through Paragraph (no hscroll).
        let body = Text::from(line_spans.into_iter().map(Line::from).collect::<Vec<_>>());
        Paragraph::new(body)
            .scroll((state.diff_scroll, 0))
            .wrap(Wrap { trim: false })
            .block(block)
    } else {
        // Manual horizontal + vertical scroll mirrors diff_view, avoiding the styled-line
        // redraw artifacts that Paragraph::scroll leaves on coloured spans.
        let visible: Vec<Line> = line_spans
            .into_iter()
            .map(|spans| apply_hscroll_spans(spans, state.diff_hscroll as usize))
            .skip(state.diff_scroll as usize)
            .collect();
        Paragraph::new(Text::from(visible)).block(block)
    };
    frame.render_widget(paragraph, chunks[0]);
    // Only the non-wrap path keeps a 1:1 line↔row mapping for an accurate thumb; under soft
    // wrapping the logical line count diverges from rendered rows, so skip the scrollbar there.
    if !state.preview_wrap {
        render_text_scrollbar(frame, chunks[0], total_lines, state.diff_scroll as usize);
    }
    render_footer(frame, chunks[1], "", &footer, colored);
}

pub(super) fn render_pins(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    // Sync feedback (e.g. "already in sync", "can't tell which side is newer") is set via
    // state.status while staying on this screen, so the footer must surface it (see #72).
    let hints = if state.pinned.is_empty() {
        "Esc/q back"
    } else {
        "↑↓ move · ←→ scroll · Enter diff · s sync · u push · d pull · x unpin  ·  ✓ synced ↑ local-newer ↓ remote-newer ? n/a  ·  Esc/q back"
    };
    let (footer, colored) = footer_with_status(state.status.as_deref(), hints);
    let footer_lines = wrap_line_count(&footer, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(footer_lines + 1)])
        .split(area);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let items: Vec<ListItem> = if state.pinned.is_empty() {
        vec![
            ListItem::new("  📌 No pinned mappings found (use p to pin a pair)")
                .style(Style::default().fg(Color::DarkGray)),
        ]
    } else {
        state
            .pinned
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let (lts, rts) = state.pin_mtimes(i);
                let age = |ts: Option<u64>| {
                    ts.map(|t| crate::domain::humanize_age(now - t as i64))
                        .unwrap_or_else(|| "?".to_string())
                };
                ListItem::new(hscroll_str(
                    &pin_row_label(
                        state.pin_sync_status(i).icon(),
                        &m.local_path,
                        &m.gist_id,
                        &m.gist_filename,
                        &age(lts),
                        &age(rts),
                    ),
                    state.pins_hscroll,
                ))
            })
            .collect()
    };

    let selected = (!state.pinned.is_empty()).then_some(state.pins_index);
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    "Pinned Mappings {}",
                    count_label(state.pinned.len(), state.pinned.len())
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    render_footer(frame, chunks[1], "", &footer, colored);
}

pub(super) fn gist_group_row_label(g: &GistGroup, now: u64, sort: GistGroupSort) -> String {
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
    format!("{}  {}  📄 {}  🕒 {}", g.id, desc, g.file_count, age)
}

pub(super) fn render_gists(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    // Footer: filter input while filtering, else a one-shot status message (e.g. the compaction
    // result) when present, else the command hints. Only the hints get key colouring.
    let (ftitle, footer, colored) = if state.gists_filtering {
        (
            "Filter (Enter keep · Esc clear)".to_string(),
            format!("/{}_", state.gists_filter_query),
            false,
        )
    } else if let Some(message) = &state.status {
        (String::new(), message.clone(), false)
    } else {
        (
            String::new(),
            "↑↓ move · ←→ scroll · Enter detail · / filter · s sort · v type · e desc · o browser · c compact · X delete · q back"
                .to_string(),
            true,
        )
    };
    let footer_lines = wrap_line_count(&footer, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(footer_lines + 1)])
        .split(area);

    let groups = state.visible_gist_groups();
    let now = unix_now();
    let items: Vec<ListItem> = if groups.is_empty() {
        let msg = if state.gist_groups().is_empty() {
            ListItem::new("  📭 No gists found").style(Style::default().fg(Color::DarkGray))
        } else {
            ListItem::new("  🔍 No gists match the filter")
                .style(Style::default().fg(Color::DarkGray))
        };
        vec![msg]
    } else {
        groups
            .iter()
            .map(|g| {
                ListItem::new(hscroll_str(
                    &gist_group_row_label(g, now, state.gists_sort),
                    state.gists_hscroll,
                ))
            })
            .collect()
    };

    let selected = (!groups.is_empty()).then_some(state.gists_index);
    let mut title = format!(
        "Gists {}  ·  sort:{}  ·  type:{}",
        count_label(groups.len(), state.gist_groups().len()),
        state.gists_sort.label(),
        state.gists_type_filter.label()
    );
    if !state.gists_filter_query.is_empty() {
        title.push_str(&format!("  ·  /{}", state.gists_filter_query));
    }
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    render_footer(frame, chunks[1], &ftitle, &footer, colored);

    if state.editing_description {
        render_centered_modal(
            frame,
            "Edit description (Enter apply · Esc cancel)",
            &format!("{}_", state.description_input),
            Color::Cyan,
        );
    }
}

/// One-line info summary for the detail header.
pub(super) fn gist_info_line(group: &GistGroup, now: u64) -> String {
    let vis = if group.public { "public" } else { "secret" };
    let created = crate::domain::parse_rfc3339_to_unix(&group.created_at)
        .map(|t| crate::domain::humanize_age(now as i64 - t as i64))
        .unwrap_or_else(|| "?".into());
    let updated = crate::domain::parse_rfc3339_to_unix(&group.updated_at)
        .map(|t| crate::domain::humanize_age(now as i64 - t as i64))
        .unwrap_or_else(|| "?".into());
    // The file count lives in the "Files (N)" section header below, so it's omitted here.
    // The detail view has room, so show the full gist id (not a truncated prefix).
    format!(
        "{vis} · created {created} · updated {updated} · {}",
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
fn file_rows(
    files: &[String],
    cursor: usize,
    offset: usize,
    visible_rows: usize,
    highlight_cursor: bool,
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
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else {
            rows.push(Line::from(format!("  {marker} {f}")));
        }
    }
    rows
}

/// The gist's title, derived from its description (falling back to the id).
fn gist_block_title(group: &GistGroup) -> String {
    if group.description.trim().is_empty() {
        format!("Gist {}", group.id)
    } else {
        format!("Gist: {}", group.description)
    }
}

/// Info + file-list block for a gist, used as the compaction-confirm background. (The gist
/// detail screen renders the info header and the file list as separate blocks so it can tab
/// between the file list and the comments.)
pub(super) fn render_gist_info_and_files(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    gist_id: &str,
) {
    let Some(group) = state.group_by_id(gist_id) else {
        return;
    };
    let files = state.gist_filenames(gist_id);
    let mut lines: Vec<Line> = vec![
        Line::from(gist_info_line(&group, unix_now())),
        Line::from(""),
        Line::from(Span::styled(
            format!("Files ({})", files.len()),
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];
    let cursor = state.detail_file_cursor.min(files.len().saturating_sub(1));
    // Visible file rows = area height minus borders(2), info line, blank, "Files (n)" header (3).
    let visible_rows = (area.height as usize).saturating_sub(5);
    let offset = file_list_scroll(cursor, visible_rows, files.len());
    lines.extend(file_rows(&files, cursor, offset, visible_rows, false));
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(gist_block_title(&group))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

/// The gist detail header: a block holding the basic-info line and the `Files │ Comments`
/// focus tabs. The active tab's content is rendered below it.
fn render_detail_header(frame: &mut Frame, area: Rect, state: &AppState, gist_id: &str) {
    let Some(group) = state.group_by_id(gist_id) else {
        return;
    };
    let lines = vec![
        Line::from(gist_info_line(&group, unix_now())),
        detail_focus_tabs_line(state.detail_focus),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(gist_block_title(&group))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

/// The gist detail's Files tab: the numbered, cursor-highlighted, scrollable file list,
/// titled with the file count.
fn render_gist_file_list(frame: &mut Frame, area: Rect, state: &AppState, gist_id: &str) {
    let files = state.gist_filenames(gist_id);
    let cursor = state.detail_file_cursor.min(files.len().saturating_sub(1));
    let visible_rows = (area.height as usize).saturating_sub(2);
    let offset = file_list_scroll(cursor, visible_rows, files.len());
    let lines = file_rows(&files, cursor, offset, visible_rows, true);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(format!("Files ({})", files.len()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

/// Comments pane: loading / error / empty / list (plain text wrapped to width, scrollable).
pub(super) fn render_gist_comments(frame: &mut Frame, area: Rect, state: &AppState) {
    let now = unix_now();
    let body: Vec<Line> = match (&state.detail_comments, &state.detail_comments_error) {
        (None, _) => vec![Line::from(Span::styled(
            "Loading comments…",
            Style::default().fg(Color::DarkGray),
        ))],
        (Some(_), Some(err)) => vec![Line::from(Span::styled(
            format!("comments error: {err}"),
            Style::default().fg(Color::Red),
        ))],
        (Some(comments), None) if comments.is_empty() => vec![Line::from(Span::styled(
            "No comments",
            Style::default().fg(Color::DarkGray),
        ))],
        (Some(comments), None) => {
            let mut lines = Vec::new();
            for c in comments {
                let age = crate::domain::parse_rfc3339_to_unix(&c.created_at)
                    .map(|t| crate::domain::humanize_age(now as i64 - t as i64))
                    .unwrap_or_else(|| "?".into());
                lines.push(Line::from(Span::styled(
                    format!("{} · {age}", c.author),
                    Style::default().fg(Color::Cyan),
                )));
                for raw in c.body.lines() {
                    lines.push(Line::from(format!("  {raw}")));
                }
                lines.push(Line::from(""));
            }
            lines
        }
    };
    let title = match &state.detail_comments {
        Some(c) if state.detail_comments_error.is_none() => format!("Comments ({})", c.len()),
        _ => "Comments".to_string(),
    };
    frame.render_widget(
        Paragraph::new(body)
            .scroll((state.detail_scroll, 0))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .padding(Padding::horizontal(1)),
            ),
        area,
    );
}

/// Footer text + whether to colourise it: a one-shot `state.status` message (shown plain) when
/// present, else the colourised key `hints`. Shared by every screen so action results/errors
/// surface consistently and are never swallowed by a hard-coded footer (see #72, #66).
pub(super) fn footer_with_status(status: Option<&str>, hints: &str) -> (String, bool) {
    match status {
        Some(message) => (message.to_string(), false),
        None => (hints.to_string(), true),
    }
}

/// The detail-view footer: a one-shot `state.status` message (e.g. the compaction result,
/// including "nothing to compact") when present, else the focus-aware key hints.
pub(super) fn detail_footer(status: Option<&str>, focus: DetailFocus) -> (String, bool) {
    let hints = match focus {
        DetailFocus::Comments => {
            "Tab files · ↑↓ scroll · 1-9 preview · c compact · o browser · X delete · q back"
        }
        DetailFocus::Files => {
            "Tab comments · ↑↓ select · ⏎ preview · 1-9 preview · c compact · o browser · X delete · q back"
        }
    };
    footer_with_status(status, hints)
}

/// The Files|Comments tab index, mirroring `detail_focus`. Pure so the tab selection is
/// trivially testable and stays in sync with the navigation handler. Files is the default
/// tab, so it comes first.
pub(super) fn detail_focus_tab(focus: DetailFocus) -> usize {
    match focus {
        DetailFocus::Files => 0,
        DetailFocus::Comments => 1,
    }
}

/// A `Files │ Comments` focus indicator line, with the pane Tab currently drives highlighted.
/// Rendered just under the gist's basic info (inside the info box) rather than as a floating
/// strip, so the active focus is visible without a disconnected top row.
pub(super) fn detail_focus_tabs_line(focus: DetailFocus) -> Line<'static> {
    let active = detail_focus_tab(focus);
    let active_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let idle_style = Style::default().fg(Color::DarkGray);
    let mut spans = Vec::new();
    for (i, label) in ["Files", "Comments"].iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", idle_style));
        }
        let style = if i == active {
            active_style
        } else {
            idle_style
        };
        spans.push(Span::styled(format!(" {label} "), style));
    }
    Line::from(spans)
}

pub(super) fn render_gist_detail(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    let (footer, colored) = detail_footer(state.status.as_deref(), state.detail_focus);
    let footer_lines = wrap_line_count(&footer, area.width.saturating_sub(2)).max(1);
    // Fixed 4-row header (borders + basic-info line + focus tabs); the active tab — the file
    // list or the comments, never both — fills the rest above the footer.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(3),
            Constraint::Length(footer_lines + 1),
        ])
        .split(area);
    if let Some(id) = state.detail_gist_id.as_deref() {
        render_detail_header(frame, chunks[0], state, id);
        match state.detail_focus {
            DetailFocus::Files => render_gist_file_list(frame, chunks[1], state, id),
            DetailFocus::Comments => render_gist_comments(frame, chunks[1], state),
        }
    }
    render_footer(frame, chunks[2], "", &footer, colored);
}

pub(super) fn local_row_label(path: &std::path::Path, cwd: &std::path::Path) -> String {
    path.strip_prefix(cwd).unwrap_or(path).display().to_string()
}

pub(super) fn hscroll_str(text: &str, offset: u16) -> String {
    text.chars().skip(offset as usize).collect()
}

/// Builds a single Pins-screen row. The local path is rendered with `display_path`
/// (home → `~`) so it stays readable; the full row is horizontally scrollable. Pure so
/// the path-shortening is unit-testable without a frame.
pub(super) fn pin_row_label(
    icon: &str,
    local_path: &std::path::Path,
    gist_id: &str,
    gist_filename: &str,
    local_age: &str,
    gist_age: &str,
) -> String {
    format!(
        "{}  {}  ↔  {} / {}   (local {} · gist {})",
        icon,
        crate::config::display_path(local_path),
        gist_id,
        gist_filename,
        local_age,
        gist_age,
    )
}

/// How a file-list row should be flagged: 📌 = an existing pinned pair; same-name = bold; else none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RowMark {
    Pinned,
    SameName,
    None,
}

pub(super) fn row_mark(reasons: &[MatchReason]) -> RowMark {
    if reasons.contains(&MatchReason::Pinned) {
        RowMark::Pinned
    } else if reasons.contains(&MatchReason::ExactFilename) {
        RowMark::SameName
    } else {
        RowMark::None
    }
}

/// Build a file-list row from its base text and match mark: 📌 prefix for a pinned pair,
/// bold for a same-name match, plain otherwise. Shared by both panes in `render_list`.
pub(super) fn marked_item(base: String, mark: RowMark, hscroll: u16) -> ListItem<'static> {
    match mark {
        RowMark::Pinned => ListItem::new(hscroll_str(&format!("📌 {base}"), hscroll)),
        RowMark::SameName => ListItem::new(hscroll_str(&base, hscroll))
            .style(Style::default().add_modifier(Modifier::BOLD)),
        RowMark::None => ListItem::new(hscroll_str(&base, hscroll)),
    }
}

pub(super) fn gist_row_label(g: &RankedGistFile, view: GistView) -> String {
    match view {
        GistView::Description => {
            if g.file.description.trim().is_empty() {
                g.file.filename.clone()
            } else {
                format!("{} — {}", g.file.filename, g.file.description)
            }
        }
        GistView::Id => format!("{} / {}", g.file.gist_id, g.file.filename),
    }
}

/// Command hint tailored to the focused pane: local-file actions on the left, gist actions
/// on the right, plus the always-available navigation/help/quit keys. The footer word-wraps
/// it to the terminal width.
pub(super) fn commands_hint(focus: FocusPane) -> String {
    // Focus-relevant common keys only; the full reference lives in the `?` help overlay.
    let mut items = vec!["Tab panes", "↑↓ move", "Enter diff", "a anchor"];
    match focus {
        FocusPane::Local => items.extend(["r recursive", "e edit", "n create", "P pins"]),
        FocusPane::Gist => items.extend([
            "Space preview",
            "d download",
            "u upload",
            "X remove file",
            "g gists",
        ]),
    }
    items.extend(["? help", "Esc/q quit"]);
    items.join("  ·  ")
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

/// Colour a command key by what its action does, so destructive and mutating keys stand apart
/// from plain navigation at a glance: destructive (delete/remove/unpin) → Red, write/sync
/// (download/upload/create/sync/…) → Green, everything else (navigation/view) → Cyan. Matched on
/// whole label words so e.g. `pins` does not read as the `pin` action.
pub(super) fn action_color(label: &str) -> Color {
    const DESTRUCTIVE: [&str; 3] = ["delete", "remove", "unpin"];
    const WRITE: [&str; 10] = [
        "download", "upload", "create", "new", "sync", "push", "pull", "pin", "edit", "desc",
    ];
    let mut color = Color::Cyan;
    for word in label.split_whitespace() {
        let word = word.to_ascii_lowercase();
        if DESTRUCTIVE.contains(&word.as_str()) {
            return Color::Red;
        }
        if WRITE.contains(&word.as_str()) {
            color = Color::Green;
        }
    }
    color
}

/// Style a footer command string: the leading key token of each `·`-separated item is accented by
/// its action category (see [`action_color`]); the descriptive label keeps the terminal's default
/// brightness so it stays legible, and only the separators are dimmed. Every input character is
/// preserved verbatim so `wrap_line_count` sizing stays exact.
pub(super) fn hint_line(text: &str) -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
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
                let key = Style::default().fg(action_color(label));
                spans.push(Span::styled(k.to_string(), key));
                spans.push(Span::raw(label.to_string()));
            }
            None => spans.push(Span::styled(
                rest.to_string(),
                Style::default().fg(action_color("")),
            )),
        }
    }
    Line::from(spans)
}

/// The shared borderless footer block: a single dim top divider that carries the left `title` and
/// the app version pinned to the bottom-right corner of every screen.
pub(super) fn footer_block(title: &str) -> Block<'static> {
    // Repo URL (scheme stripped — the host/path already names the project) plus the version.
    let repo = env!("CARGO_PKG_REPOSITORY")
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let label = format!(" {} v{} ", repo, env!("CARGO_PKG_VERSION"));
    Block::default()
        .title(title.to_string())
        .title_top(
            // Reset (not the dim divider colour) so it reads at full brightness.
            Line::from(label)
                .right_aligned()
                .style(Style::default().fg(Color::Reset)),
        )
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray))
        .padding(Padding::horizontal(1))
}

/// Render a command footer into `area`. `colored` accents the command keys; pass `false` for
/// plain text (filter input, status messages) that is not a key/label list.
pub(super) fn render_footer(frame: &mut Frame, area: Rect, title: &str, text: &str, colored: bool) {
    let para = if colored {
        Paragraph::new(hint_line(text))
    } else {
        Paragraph::new(text.to_string())
    };
    frame.render_widget(
        para.wrap(Wrap { trim: true }).block(footer_block(title)),
        area,
    );
}

pub(super) fn render_list(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    let footer_body = if state.filtering {
        format!(
            "filter: {}_   (Enter apply · Esc clear)",
            state.filter_query
        )
    } else {
        match &state.status {
            Some(message) => message.clone(),
            None => commands_hint(state.focus),
        }
    };
    // Only the command-hint variant gets key colouring; filter input and status stay plain.
    let footer_is_command = !state.filtering && state.status.is_none();
    // Width inside the footer block: minus the 2 horizontal padding columns (no side borders).
    let footer_lines = wrap_line_count(&footer_body, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(footer_lines + 1)])
        .split(area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    // Show each candidate's path relative to cwd; in flat mode this is just the filename,
    // in recursive mode it includes the subdirectory (e.g. src/utils/helpers.rs).
    let local_items: Vec<ListItem> = if state.local_scanning && state.locals.is_empty() {
        vec![ListItem::new(format!(
            "  {} Scanning files…",
            spinner_glyph(state.spinner_frame)
        ))
        .style(Style::default().fg(Color::DarkGray))]
    } else if state.locals.is_empty() {
        vec![ListItem::new("  📭 No local files found").style(Style::default().fg(Color::DarkGray))]
    } else {
        state
            .visible_locals()
            .iter()
            .map(|r| {
                let base = local_row_label(&r.candidate.path, &state.cwd);
                marked_item(base, row_mark(&r.reasons), state.local_hscroll)
            })
            .collect()
    };
    let local_focused = state.focus == FocusPane::Local;
    let local_selected = (!state.locals.is_empty()).then_some(state.local_index);
    let recursive_marker = if state.local_recursive { " [↓]" } else { "" };
    let scanning_marker = if state.local_scanning { " …" } else { "" };
    let local_title = format!(
        "[1] Local {} · {}{}{} · sort:{}",
        count_label(state.locals.len(), state.locals.len()),
        crate::config::display_path(&state.cwd),
        recursive_marker,
        scanning_marker,
        state.local_sort.label()
    );
    // Mark the pane that currently drives the match ranking (the anchor).
    let local_title = if state.anchor == FocusPane::Local {
        format!("{local_title} · ⚓")
    } else {
        local_title
    };
    render_pane(
        frame,
        columns[0],
        &local_title,
        local_items,
        local_focused,
        local_selected,
    );

    let ranked = state.ranked_gists();
    let gist_items: Vec<ListItem> = if state.loading && ranked.is_empty() {
        vec![ListItem::new(format!(
            "  {} Loading gists…",
            spinner_glyph(state.spinner_frame)
        ))
        .style(Style::default().fg(Color::DarkGray))]
    } else if ranked.is_empty() {
        let message = if !state.filter_query.is_empty() {
            ListItem::new("  🔍 No gists match the filter")
                .style(Style::default().fg(Color::DarkGray))
        } else {
            ListItem::new("  📭 No gists found").style(Style::default().fg(Color::DarkGray))
        };
        vec![message]
    } else {
        ranked
            .iter()
            .map(|g| {
                let base = gist_row_label(g, state.gist_view);
                marked_item(base, row_mark(&g.reasons), state.gist_hscroll)
            })
            .collect()
    };
    let gist_focused = state.focus == FocusPane::Gist;
    let gist_selected = (!ranked.is_empty()).then_some(state.gist_index);
    let mut gist_title = format!(
        "[2] Gists {} · {} · {}",
        count_label(ranked.len(), state.gists.len()),
        state.gist_type_filter.label(),
        state.gist_sort.label()
    );
    if !state.filter_query.is_empty() {
        gist_title.push_str(&format!(" · /{}", state.filter_query));
    }
    let gist_title = if state.anchor == FocusPane::Gist {
        format!("{gist_title} · ⚓")
    } else {
        gist_title
    };
    render_pane(
        frame,
        columns[1],
        &gist_title,
        gist_items,
        gist_focused,
        gist_selected,
    );

    render_footer(frame, chunks[1], "", &footer_body, footer_is_command);
}

pub(super) fn render_pane(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: Vec<ListItem>,
    focused: bool,
    selected: Option<usize>,
) {
    let item_count = items.len();
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    // The border colour alone signals which pane is active; row text stays at full
    // brightness in both panes so it is always legible.
    // Focused selection is a solid bar (whole row); unfocused just bolds the row.
    let highlight_style = if focused {
        Style::default()
            .bg(Color::Cyan)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                // The title sits on the top border, so it would otherwise inherit the dimmed
                // border colour when unfocused; pin it to the terminal default so only the
                // border line reflects focus, never the title text.
                .title_style(Style::default().fg(Color::Reset))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style)
                .padding(Padding::horizontal(1)),
        )
        .highlight_style(highlight_style)
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, area, &mut list_state);

    // Show a scrollbar when the list overflows its viewport.
    let viewport = area.height.saturating_sub(2) as usize;
    if viewport > 0 && item_count > viewport {
        let mut scrollbar_state = ScrollbarState::new(item_count).position(selected.unwrap_or(0));
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
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

/// Del line with word-level highlighting: changed words bold-red, unchanged words plain red.
pub(super) fn inline_del_line(del_line: &str, ins_line: &str, hscroll: usize) -> Line<'static> {
    let del_content = del_line.get(1..).unwrap_or("");
    let ins_content = ins_line.get(1..).unwrap_or("");
    let mut spans = vec![Span::styled("-", Style::default().fg(Color::Red))];
    for change in TextDiff::from_words(del_content, ins_content).iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            ChangeTag::Equal => spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(Color::Red),
            )),
            ChangeTag::Insert => {}
        }
    }
    apply_hscroll_spans(spans, hscroll)
}

/// Ins line with word-level highlighting: changed words bold-green, unchanged words plain green.
pub(super) fn inline_ins_line(del_line: &str, ins_line: &str, hscroll: usize) -> Line<'static> {
    let del_content = del_line.get(1..).unwrap_or("");
    let ins_content = ins_line.get(1..).unwrap_or("");
    let mut spans = vec![Span::styled("+", Style::default().fg(Color::Green))];
    for change in TextDiff::from_words(del_content, ins_content).iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => spans.push(Span::styled(
                change.value().to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
            ChangeTag::Equal => spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(Color::Green),
            )),
            ChangeTag::Delete => {}
        }
    }
    apply_hscroll_spans(spans, hscroll)
}

/// Renders a `--- /+++` header line, tinting the leading `local`/`gist` keyword (yellow/blue)
/// so each side's identity is readable regardless of which way the diff is oriented — the
/// `Enter` preview flips direction with focus (see `preview_diff_text`). The side is classified
/// from the un-scrolled line (anchored right after the marker), then the keyword is coloured in
/// the horizontally-scrolled slice; the rest stays bold.
pub(super) fn header_line(line: &str, hscroll: usize) -> Line<'static> {
    let visible: String = line.chars().skip(hscroll).collect();
    let bold = Style::default().add_modifier(Modifier::BOLD);

    let body = line
        .strip_prefix("--- ")
        .or_else(|| line.strip_prefix("+++ "))
        .unwrap_or(line);
    let (keyword, color) = if body.starts_with("local") {
        ("local", Color::Yellow)
    } else if body.starts_with("gist") {
        ("gist", Color::Blue)
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
            super::highlight::highlight_buffer(ext, &contents)
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
                    result.push(inline_del_line(dl, ins_lines[j], hscroll));
                } else {
                    let visible: String = dl.chars().skip(hscroll).collect();
                    result.push(Line::styled(visible, Style::default().fg(Color::Red)));
                }
            }
            // Ins lines: paired ones get inline highlighting, extras plain green.
            for (j, &il) in ins_lines.iter().enumerate() {
                if j < pair_count {
                    result.push(inline_ins_line(del_lines[j], il, hscroll));
                } else {
                    let visible: String = il.chars().skip(hscroll).collect();
                    result.push(Line::styled(visible, Style::default().fg(Color::Green)));
                }
            }
        } else if line.starts_with("+++") || line.starts_with("---") {
            result.push(header_line(line, hscroll));
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

/// The diff pane title. The gist id, filenames, and both sides' mtimes live in the diff's
/// `--- / +++` header lines (see `diff_labels`); the title stays concise and avoids
/// repeating a path.
pub(super) fn diff_title(state: &AppState) -> String {
    match &state.pending_action {
        Some(PendingAction::Upload {
            gist_id, filename, ..
        }) => format!("Upload → gist {gist_id} / {filename}"),
        Some(PendingAction::Create { local_path }) => {
            format!(
                "Create gist from {}",
                crate::config::display_path(local_path)
            )
        }
        Some(PendingAction::Delete { gist_id, .. }) => {
            format!("Delete gist {gist_id}")
        }
        Some(PendingAction::RemoveFile {
            gist_id, filename, ..
        }) => {
            format!("Remove {filename} from gist {gist_id}")
        }
        _ => {
            let label = if state.diff_identical {
                "Diff (identical)"
            } else {
                "Diff"
            };
            if state.preview_local.as_os_str().is_empty()
                || state.preview_local == state.download_target
            {
                format!(
                    "{label} → {}",
                    crate::config::display_path(&state.download_target)
                )
            } else {
                format!(
                    "{label}: {} → {}",
                    crate::config::display_path(&state.preview_local),
                    crate::config::display_path(&state.download_target)
                )
            }
        }
    }
}

/// The prompt shown inside the centered confirm modal — one line per pending action,
/// listing the keys that resolve it. Pure so it can be unit-tested.
pub(super) fn confirm_prompt(state: &AppState) -> String {
    match &state.pending_action {
        Some(PendingAction::Create { .. }) if state.editing_description => {
            format!(
                "Description (optional): {}_   ·  Enter next  ·  Esc cancel",
                state.description_input
            )
        }
        Some(PendingAction::Create { local_path }) => {
            let desc = if state.description_input.is_empty() {
                "no description".to_string()
            } else {
                format!("desc: {}", state.description_input)
            };
            format!(
                "Create gist from {} ({desc})?  s secret  p public  Esc cancel",
                crate::config::display_path(local_path)
            )
        }
        Some(PendingAction::Upload {
            gist_id,
            filename,
            local_path,
        }) => {
            let edited_status = if state.upload_edited_content.is_some() {
                " [edited]"
            } else {
                ""
            };
            let mut opts = format!("y yes  n/Esc cancel  e edit{edited_status}");
            if is_json_file(local_path) {
                let pretty_status = if state.upload_json_pretty {
                    " [on]"
                } else {
                    " [off]"
                };
                let sort_status = if state.upload_json_sort {
                    " [on]"
                } else {
                    " [off]"
                };
                opts.push_str(&format!("  p pretty{pretty_status}  s sort{sort_status}"));
            }
            format!("Upload {filename} to gist {gist_id}?  ·  {opts}")
        }
        Some(PendingAction::Delete { gist_id, label }) => {
            format!("Permanently delete \"{label}\" ({gist_id})? (y/n)")
        }
        Some(PendingAction::RemoveFile {
            gist_id, filename, ..
        }) => {
            format!("Remove {filename} from gist {gist_id}? (y/n)")
        }
        Some(PendingAction::CompactGist { label, count, .. }) => {
            format!(
                "Compact {count} revisions of \"{label}\" into one? This force-pushes and cannot be undone. (y/n)"
            )
        }
        _ => format!("Overwrite {}? (y/n)", state.download_target.display()),
    }
}

/// Title and border colour for the confirm modal. Destructive actions are tinted red so the
/// stakes read at a glance; everything else is a neutral yellow prompt.
pub(super) fn confirm_modal_style(state: &AppState) -> (&'static str, Color) {
    match &state.pending_action {
        Some(PendingAction::Create { .. }) if state.editing_description => {
            ("Description", Color::Cyan)
        }
        Some(PendingAction::Create { .. }) => ("Create gist", Color::Yellow),
        Some(PendingAction::Upload { .. }) => ("Upload", Color::Yellow),
        Some(PendingAction::Delete { .. }) => ("Delete", Color::Red),
        Some(PendingAction::RemoveFile { .. }) => ("Remove file", Color::Red),
        Some(PendingAction::CompactGist { .. }) => ("Compact revisions", Color::Red),
        _ => ("Overwrite", Color::Red),
    }
}

/// Overlay a vertical scrollbar on the right edge of a bordered, scrollable text pane when
/// its `total` lines overflow the inner viewport. `offset` is the index of the topmost
/// visible line, so the thumb reflects the real scroll position (not a selection index).
fn render_text_scrollbar(frame: &mut Frame, area: Rect, total: usize, offset: usize) {
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

/// Render just the diff content pane (no footer) into `area`.
pub(super) fn render_diff_pane(frame: &mut Frame, area: Rect, state: &AppState) {
    // Collapse unchanged context to the configured radius unless the user toggled full view.
    let diff_body = match state.effective_diff_context() {
        Some(radius) => crate::diff::collapse_context(&state.diff_text, radius),
        None => state.diff_text.clone(),
    };
    let ext = diff_ext(state);
    frame.render_widget(
        Paragraph::new(diff_view_highlighted(
            &diff_body,
            state.diff_scroll,
            state.diff_hscroll,
            ext.as_deref(),
            state.syntax_highlight,
        ))
        .block(
            Block::default()
                .title(diff_title(state))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
    let total_lines = diff_body.lines().count();
    render_text_scrollbar(frame, area, total_lines, state.diff_scroll as usize);
}

/// The `Screen::Diff` preview: the diff pane plus a scroll/commands footer.
///
/// #72 audit: this footer intentionally does not surface `state.status`. Diff actions (`d`/`u`)
/// transition to `Screen::Confirm` or to the IO that lands back on `List`; their results surface
/// on those destination screens (which read `state.status`), so no status is set while on Diff.
pub(super) fn render_diff(frame: &mut Frame, state: &AppState) {
    let context = if state.diff_show_full {
        "c context [full]".to_string()
    } else {
        format!("c context [{}]", state.diff_context)
    };
    let footer = if state.diff_identical {
        format!(
            "Files are identical — nothing to sync  ·  ↑↓←→ PgUp/Dn scroll  ·  {context}  ·  Esc/q back"
        )
    } else {
        format!("↑↓←→ PgUp/Dn scroll  ·  d download  ·  u upload  ·  {context}  ·  Esc/q back")
    };

    let area = frame.area();
    let footer_lines = wrap_line_count(&footer, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(footer_lines + 1)])
        .split(area);

    render_diff_pane(frame, chunks[0], state);

    render_footer(frame, chunks[1], "", &footer, true);
}

/// `Screen::Confirm`: the diff fills the screen as context behind a centered prompt modal,
/// keeping the overwrite gate's diff visible while the question is asked front-and-centre.
/// #72 audit: this modal intentionally does not surface `state.status`. It is a transient y/n
/// gate — confirming executes the action and transitions to `List`/`Gists`, where the result
/// status is shown; cancelling returns to the launching screen without setting a status here.
pub(super) fn render_confirm(frame: &mut Frame, state: &AppState) {
    match &state.pending_action {
        Some(PendingAction::CompactGist { gist_id, .. }) => {
            render_gist_info_and_files(frame, frame.area(), state, gist_id);
        }
        _ => render_diff_pane(frame, frame.area(), state),
    }
    let (title, border) = confirm_modal_style(state);
    render_centered_modal(frame, title, &confirm_prompt(state), border);
}

pub(super) fn is_json_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

pub(super) fn render_centered_modal(frame: &mut Frame, title: &str, body: &str, border: Color) {
    let area = frame.area();
    let max_width = area.width.saturating_sub(2).max(1);
    let width = ((area.width as u32 * 60 / 100) as u16).clamp(max_width.min(20), max_width);
    // Inner text width = box width minus the two border columns and the horizontal padding.
    let inner_width = width.saturating_sub(4);
    let body_lines = wrap_line_count(body, inner_width).max(1);
    let max_height = area.height.saturating_sub(2).max(1);
    let height = (body_lines + 2).clamp(max_height.min(3), max_height);
    let rect = Rect {
        x: area.width.saturating_sub(width) / 2,
        y: area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(body.to_string())
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .title(title.to_string())
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(border))
                    .padding(Padding::horizontal(1)),
            ),
        rect,
    );
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

/// A centered "Working…" box shown while a blocking `gh` action runs.
pub(super) fn render_loading_overlay(frame: &mut Frame, msg: &str, spinner_frame: usize) {
    let body = format!("{} {msg}", spinner_glyph(spinner_frame));
    render_centered_modal(frame, "Working…", &body, Color::Cyan);
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
) -> String {
    if upload_orientation {
        crate::diff::unified_diff(gist_label, remote, local_label, local_content)
    } else {
        crate::diff::unified_diff(local_label, local_content, gist_label, remote)
    }
}
