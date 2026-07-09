use super::{theme::Theme, *};
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

pub(super) fn render(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    *layout = MouseLayout::default();
    // Paint the full canvas so every unfilled cell uses the theme background (no-op for dark
    // theme where bg=Reset, effective for light theme which sets a grey canvas).
    frame.render_widget(
        Block::default().style(state.theme.base_style()),
        frame.area(),
    );
    match state.screen {
        Screen::Palette => render_palette(frame, state, layout),
        Screen::List => render_list(frame, state, layout),
        Screen::Diff => render_diff(frame, state, layout),
        Screen::Confirm => render_confirm(frame, state, layout),
        Screen::Preview => render_preview(frame, state, layout),
        Screen::Help => render_help(frame, state, layout),
        Screen::Pins => render_pins(frame, state, layout),
        Screen::Gists => render_gists(frame, state, layout),
        Screen::GistDetail => render_gist_detail(frame, state, layout),
        Screen::Revisions => render_revisions(frame, state, layout),
    }
    if let Some(ref msg) = state.bg_task_msg {
        render_loading_overlay(frame, msg, state.spinner_frame, &state.theme);
    }
}

fn render_close_button(frame: &mut Frame, outer: Rect, theme: &Theme) -> Rect {
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

/// The three cross-screen top-bar shortcuts: bracketed hotkey letter + label. Kept in one
/// place so the click hit-rect math and the rendered text can never drift apart.
const TOP_BAR_ITEMS: [(&str, &str); 3] = [("G", "ists"), ("P", "ins"), ("?", "Help")];

/// Height of the persistent top bar rendered on every screen except the transient `Confirm`
/// y/n modal (which keeps its full-bleed diff/gist-info background — see `render_confirm`).
const TOP_BAR_HEIGHT: u16 = 1;

/// Renders the cross-screen top bar — ` gistui` on the left, `(G)ists (P)ins (?)Help`
/// right-aligned — into the top row of `area`, then returns the remaining rect below it for
/// the caller's existing content/footer layout (otherwise unchanged). The icons render as
/// plain text even with the mouse disabled, so the shortcuts stay visible; their hit-rects are
/// only recorded in `layout` when `mouse_enabled`, matching every other clickable region.
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

fn help_topic_body(topic: HelpTopic) -> &'static str {
    match topic {
        HelpTopic::List => {
            "\
Navigation
  Tab        switch pane (Local / Gists)
  1 / 2      jump to the Local / Gist pane
  Up/Down    move the selection (also j / k)
  Left/Right scroll a long row horizontally (also h / l)
  Ctrl+b/f   page up / down by 10 (also PageUp / PageDown)

List screen
  r          toggle recursive file discovery (skips hidden + configured dirs)
  /          filter the focused pane (Local = path/filename, Gist = description/id)
             while filtering: type to match · ↑↓ move · PgUp/PgDn page · Tab apply + switch pane
             · Enter apply · Esc clear · ←/→/Home/End move · Del
  v          cycle gist visibility: all / public / secret / starred / forked
  *          star / unstar the selected gist
             (others' gists are read-only: preview, diff, download, browser — not pin/upload/delete;
              open gist detail to fork with F)
  s          cycle the focused pane's sort: match / name / recent
  t          toggle row view: description / id
  T          toggle light/dark colour theme (global; saved to config)
  a          flip which pane drives match ranking (anchor); the other pane
             re-ranks against the anchor's selection (focus stays put)
             (📌 = pinned pair · bold = same filename)

Actions (on the selected local file + gist)
  Enter      diff the local file against the gist; direction follows the focused
             pane — Gist pane = download view, Local pane = upload view
             (--- old / +++ new; local label = yellow, gist label = blue)
  Space      preview the gist file's content (R in preview to force-refresh;
             blocked for images/binary — use d to download instead)
  H          open revision history for the selected gist file
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

Mouse (on by default; disable with mouse = false in config or --no-mouse)
  Wheel      scroll the focused list or content pane
  Click      select the clicked row (List panes also switch focus)
  Dbl-click  open the clicked row (diff / detail / pin diff / preview)
  Tab click  switch Files / Comments on the Gist details screen
  [✕] btn    close / go back on any pop-up screen
  Top bar    click (G)ists / (P)ins / (?)Help (top-right, every screen) to jump there
  Right-click  open the context menu at the click (same as ;)
  ; / Ctrl+p   open the menu / command palette from the keyboard (see General)"
        }
        HelpTopic::Pins => {
            "\
  Up/Down    move between pins (also j / k)
  PageUp/Dn  page by 10 (also Ctrl+b / Ctrl+f)
  Left/Right scroll a long local path horizontally (also h / l; ~ = home)
  /          filter pins by path or filename (↑↓ move · PgUp/PgDn page · Enter apply · Esc clear)
             ←/→/Home/End move the text cursor · Del deletes ahead
  o          cycle sort: default / local path / gist filename
  Enter      diff the selected pair (then d pull / u push from the diff)
  s          smart-sync (newer side wins; skips if already identical)
  u          force push  (upload local → gist)
  d          force pull  (download gist → local, diff + y/n confirm)
  x          unpin the selected pair
  status     ✓ synced · ↑ local newer · ↓ remote newer · ✕ missing · ? unknown
  Each row shows (local <age> · gist <age>) relative modification times."
        }
        HelpTopic::GistManager => {
            "\
  Up/Down    move between gists (also j / k)
  PageUp/Dn  page by 10 (also Ctrl+b / Ctrl+f)
  Left/Right scroll a long description horizontally (also h / l)
  /          filter gists by description or id (↑↓ move · PgUp/PgDn page · Enter apply · Esc clear)
  s          cycle sort: updated / created
  v          cycle visibility: all / public / secret / starred / forked
  *          star / unstar the selected gist
  Enter      open the gist detail view (info, file list, comments)
  o          open the gist in your web browser
  y          copy the gist's URL to the system clipboard
  H          open revision history (browse, diff, restore)
  q / Esc    back to the list
             (edit description, compact, delete: gist detail only, owned gists)
  Rows show ☆ N (stargazers), ⑂ N (forks), 💬 N (comments) when non-zero;
  ★ prefix = you starred it; ⑂ prefix = this gist is a fork."
        }
        HelpTopic::GistDetail => {
            "\
  Tab        switch tab: Files / Comments (one shows at a time; opens on Files)
  Up/Down    move the file cursor (Files tab) or scroll comments (also j / k)
  PageUp/Dn  page comments / file cursor by 10 (also Ctrl+b / Ctrl+f)
  m          load 30 older comments (Comments tab; also click the top line)
  Enter      preview the cursor-selected file (file list focused; blocked for binary)
  1-9        preview the content of the Nth file (full-screen; R refresh, q back)
             non-text files are tagged (binary) in the list
  H          open revision history for this gist (target = cursor file)
  *          star / unstar this gist
  o          open the gist in your web browser
  y          copy the gist's URL to the system clipboard
  q / Esc    back to the gist manager
  Info line shows ☆ N (stargazers), ⑂ N (forks), 💬 N (comments) when non-zero
  Owned gists only:
  e          edit the gist description (Enter apply, Esc cancel)
             ←/→/Home/End move the text cursor · Del deletes ahead
  c          compact revisions (y/n confirm; gist info shown as context)
  X          delete the entire gist and all its files (y/n confirm)
  Others' gists:
  F          fork into your account"
        }
        HelpTopic::Revisions => {
            "\
  Up/Down    move between revisions (also j / k; newest first; row 0 = current)
  PageUp/Dn  page by 10 (also Ctrl+b / Ctrl+f)
  Left/Right scroll a long row horizontally (also h / l)
  Enter      diff this revision vs its parent (incremental; initial = all additions)
  F          cycle the target file (multi-file gists; wraps)
  D          diff the target file: selected revision vs current (read-only; no download/upload)
  r          restore the target file from the selected revision (y/n confirm)
  q / Esc    back"
        }
        HelpTopic::Diff => {
            "\
  Up/Down/Left/Right  scroll the diff (also j / k / h / l; Left/Right only when wrap is off)
  PageUp/Dn  scroll the diff by 10 lines (also Ctrl+b / Ctrl+f)
  w          toggle soft line wrapping (remembered for the session)
  c          toggle context: configured radius <-> full file (remembered)
  d / u      download / upload from the diff
  syntax     unchanged context lines are syntax-highlighted by file type
  newline    a file-final-newline-only difference counts as identical
             (set ignore_trailing_newline = false for byte-exact diffs)
  Esc / q    back"
        }
        HelpTopic::Preview => {
            "\
  Up/Down/Left/Right  scroll (also j / k / h / l; Left/Right only when wrap is off)
  PageUp/Dn  scroll by 10 lines (also Ctrl+b / Ctrl+f)
  w          toggle soft line wrapping (remembered for the session)
  y          copy the gist URL · Y copy the file content to the clipboard
  syntax     known file types are syntax-highlighted
  R          re-fetch the content
  Esc / q    back"
        }
        HelpTopic::Upload => {
            "\
  y          confirm and execute the upload
  n / Esc    cancel the upload
  e          edit / redact the upload content in $EDITOR before upload
             (GUI editors: the diff updates live while the editor stays open;
             y/e wait until you close it — n still cancels immediately)
  p          (JSON only) toggle pretty-print formatting
  s          (JSON only) toggle recursive key sorting"
        }
        HelpTopic::General => {
            "\
  Esc / q    close an overlay; from the list, press twice to quit the app
  ?          show this help
  ;          context menu (actions valid for the current screen + selection)
  Ctrl+p     command palette (all actions + cross-screen navigation; type to filter)
  T          toggle light/dark colour theme (saved to config)
  Up/Down    scroll this help text
  NO_COLOR   set this env var to disable syntax highlighting (preview + diff)"
        }
        HelpTopic::About => {
            unreachable!(
                "About has its own dynamic body in about_topic_lines, rendered before help_topic_body is ever called"
            )
        }
    }
}

/// Fixed row (0-indexed, within the topic body) of the clickable repo-URL line — used to
/// place `MouseLayout::repo_link`'s hit-rect. Kept stable regardless of update-check state
/// (see `about_topic_lines`) so this constant never has to change.
const ABOUT_REPO_LINE: u16 = 2;

/// The `About` topic's body: version, the clickable repo link (relocated from the old
/// per-screen footer — see `render_footer`), and the update-check status if a newer release
/// is available. Unlike every other topic's `&'static str` (`help_topic_body`), this one
/// needs `state`, so it returns owned `Line`s instead.
fn about_topic_lines(state: &AppState) -> Vec<Line<'static>> {
    let repo = env!("CARGO_PKG_REPOSITORY")
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let mut lines = vec![
        Line::from(format!("gistui v{}", env!("CARGO_PKG_VERSION"))),
        Line::from(""),
        // The leading indent is a plain (unstyled) span so only the repo URL itself is
        // underlined — not the whitespace in front of it.
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                repo.to_string(),
                Style::default()
                    .fg(state.theme.fg)
                    .add_modifier(Modifier::UNDERLINED),
            ),
        ]),
        Line::from(""),
    ];
    if let Some(latest) = &state.update_available {
        lines.push(Line::from(crate::update_check::update_hint(
            latest,
            &state.install_method,
        )));
    }
    lines
}

pub(super) fn render_help(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, state.mouse_enabled, layout);
    if state.help.index_open {
        let items: Vec<ListItem> = HelpTopic::all()
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let key = if i == 9 {
                    "0".to_string()
                } else {
                    (i + 1).to_string()
                };
                ListItem::new(format!("  {}  {}", key, t.title()))
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .title("Help — pick a topic (1-9,0 / ↑↓ Enter · Esc back)")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .style(state.theme.base_style())
                    .padding(Padding::horizontal(1)),
            )
            .style(state.theme.base_style())
            .highlight_style(
                Style::default()
                    .bg(state.theme.accent)
                    .fg(state.theme.fg_on_accent)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        let mut list_state = ListState::default();
        list_state.select(Some(state.help.index_sel));
        frame.render_stateful_widget(list, area, &mut list_state);
        if state.mouse_enabled {
            layout.list = Some(PaneHit {
                rect: area,
                offset: list_state.offset(),
            });
        }
    } else {
        let title = format!(
            "Help · {} — Tab topics · ↑↓ scroll · Esc back",
            state.help.topic.title()
        );
        let body: Text = if state.help.topic == HelpTopic::About {
            Text::from(about_topic_lines(state))
        } else {
            Text::from(help_topic_body(state.help.topic))
        };
        frame.render_widget(
            Paragraph::new(body)
                .style(state.theme.base_style())
                .scroll((state.help.scroll, 0))
                .block(
                    Block::default()
                        .title(title)
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .style(state.theme.base_style())
                        .padding(Padding::horizontal(1)),
                ),
            area,
        );
        // The repo-link line only gets a click target while it's actually the About topic and
        // currently scrolled into view — if the user has scrolled it off-screen, the hit-rect
        // is simply omitted this frame rather than tracked at a stale position.
        if state.help.topic == HelpTopic::About && state.mouse_enabled {
            let visible_row = ABOUT_REPO_LINE as i32 - state.help.scroll as i32;
            let inner_height = area.height.saturating_sub(2);
            if visible_row >= 0 && (visible_row as u16) < inner_height {
                layout.repo_link = Some(Rect::new(
                    area.x + 1,
                    area.y + 1 + visible_row as u16,
                    area.width.saturating_sub(2),
                    1,
                ));
            }
        }
    }
    if state.mouse_enabled {
        layout.close_button = Some(render_close_button(frame, area, &state.theme));
    }
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
        (true, Some(ext)) => super::highlight::highlight_buffer(&ext, &lines, &state.theme),
        _ => lines.into_iter().map(|l| vec![Span::raw(l)]).collect(),
    }
}

pub(super) fn render_preview(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, state.mouse_enabled, layout);
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
        .constraints([Constraint::Min(5), Constraint::Length(footer_lines)])
        .split(area);

    // When wrapping, horizontal scroll is meaningless — pin the x offset to 0 so long lines
    // wrap into view instead of being scrolled off-screen.
    let block = Block::default()
        .title(state.preview_title.clone())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(state.theme.base_style())
        .padding(Padding::horizontal(1));
    let line_spans = preview_line_spans(state);
    let total_lines = line_spans.len();
    let paragraph = if state.preview_wrap {
        // Wrapping needs the full line set; vertical scroll goes through Paragraph (no hscroll).
        let body = Text::from(line_spans.into_iter().map(Line::from).collect::<Vec<_>>());
        Paragraph::new(body)
            .style(state.theme.base_style())
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
        Paragraph::new(Text::from(visible))
            .style(state.theme.base_style())
            .block(block)
    };
    frame.render_widget(paragraph, chunks[0]);
    // Only the non-wrap path keeps a 1:1 line↔row mapping for an accurate thumb; under soft
    // wrapping the logical line count diverges from rendered rows, so skip the scrollbar there.
    if !state.preview_wrap {
        render_text_scrollbar(frame, chunks[0], total_lines, state.diff_scroll as usize);
    }
    render_footer(frame, chunks[1], "", &footer, colored, &state.theme, layout);
    if state.mouse_enabled {
        layout.close_button = Some(render_close_button(frame, area, &state.theme));
    }
}

pub(super) fn render_pins(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, state.mouse_enabled, layout);
    // Sync feedback (e.g. "already in sync", "can't tell which side is newer") is set via
    // state.status while staying on this screen, so the footer must surface it (see #72).
    let (ftitle, footer, colored) = if state.pins.filtering {
        (
            "Filter (↑↓ move · Enter apply · Esc clear)".to_string(),
            format!("/{}_", state.pins.filter_query),
            false,
        )
    } else {
        let (footer, colored) = footer_with_status(state.status.as_deref(), MINIMAL_HINT);
        (String::new(), footer, colored)
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(footer_height(&footer, area.width, &ftitle)),
        ])
        .split(area);

    let visible = state.visible_pin_indices();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let items: Vec<ListItem> = if state.pinned.is_empty() {
        vec![
            ListItem::new("  📌 No pinned mappings found (use p to pin a pair)")
                .style(Style::default().fg(state.theme.dim)),
        ]
    } else if visible.is_empty() {
        vec![ListItem::new("  🔍 No pins match the filter")
            .style(Style::default().fg(state.theme.dim))]
    } else {
        visible
            .iter()
            .map(|&i| {
                let m = &state.pinned[i];
                let (lts, rts) = state.pin_mtimes(i);
                let age = |ts: Option<u64>| {
                    ts.map(|t| crate::domain::humanize_age(now - t as i64))
                        .unwrap_or_else(|| "?".to_string())
                };
                let status = state.pin_sync_status(i);
                let local_age = if status == crate::domain::SyncStatus::Missing {
                    "missing".to_string()
                } else {
                    age(lts)
                };
                let item = ListItem::new(hscroll_str(
                    &pin_row_label(
                        status.icon(),
                        &m.local_path,
                        &m.gist_id,
                        &m.gist_filename,
                        &local_age,
                        &age(rts),
                    ),
                    state.pins.hscroll,
                ));
                if status == crate::domain::SyncStatus::Missing {
                    item.style(Style::default().fg(state.theme.del_color))
                } else {
                    item
                }
            })
            .collect()
    };

    let selected = (!visible.is_empty()).then_some(state.pins.index);
    let mut title = format!(
        "Pinned Mappings {}",
        count_label(visible.len(), state.pinned.len())
    );
    if !state.pins.filter_query.is_empty() {
        title.push_str(&format!(" · /{}", state.pins.filter_query));
    }
    if state.pins.sort != crate::tui::PinSort::Default {
        title.push_str(&format!(" · sort:{}", state.pins.sort.label()));
    }
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.theme.accent))
                .style(state.theme.base_style())
                .padding(Padding::horizontal(1)),
        )
        .style(state.theme.base_style())
        .highlight_style(
            Style::default()
                .bg(state.theme.accent)
                .fg(state.theme.fg_on_accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if state.mouse_enabled {
        layout.list = Some(PaneHit {
            rect: chunks[0],
            offset: list_state.offset(),
        });
    }

    if state.pins.filtering {
        render_footer_line(
            frame,
            chunks[1],
            &ftitle,
            input_line("/", &state.pins.filter_query, ""),
            &state.theme,
            layout,
        );
    } else {
        render_footer(
            frame,
            chunks[1],
            &ftitle,
            &footer,
            colored,
            &state.theme,
            layout,
        );
    }
    if state.mouse_enabled {
        layout.close_button = Some(render_close_button(frame, area, &state.theme));
    }
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

pub(super) fn render_gists(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, state.mouse_enabled, layout);
    // Footer: filter input while filtering, else a one-shot status message (e.g. the compaction
    // result) when present, else the command hints. Only the hints get key colouring.
    let (ftitle, footer, colored) = if state.gist_manager.filtering {
        (
            "Filter (↑↓ move · Enter apply · Esc clear)".to_string(),
            format!("/{}_", state.gist_manager.filter_query),
            false,
        )
    } else {
        let (footer, colored) = footer_with_status(state.status.as_deref(), MINIMAL_HINT);
        (String::new(), footer, colored)
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(footer_height(&footer, area.width, &ftitle)),
        ])
        .split(area);

    let groups = state.visible_gist_groups();
    let now = unix_now();
    let items: Vec<ListItem> = if groups.is_empty() {
        let msg = if state.gist_groups().is_empty() {
            ListItem::new("  📭 No gists found").style(Style::default().fg(state.theme.dim))
        } else {
            ListItem::new("  🔍 No gists match the filter")
                .style(Style::default().fg(state.theme.dim))
        };
        vec![msg]
    } else {
        groups
            .iter()
            .map(|g| {
                ListItem::new(hscroll_str(
                    &gist_group_row_label(
                        g,
                        now,
                        state.gist_manager.sort,
                        (
                            state.gist_comment_counts.get(&g.id).copied().unwrap_or(0),
                            state.gist_star_counts.get(&g.id).copied().unwrap_or(0),
                            state.gist_fork_counts.get(&g.id).copied().unwrap_or(0),
                        ),
                        state.gist_is_starred(&g.id),
                        state.current_user_login.as_deref(),
                    ),
                    state.gist_manager.hscroll,
                ))
            })
            .collect()
    };

    let selected = (!groups.is_empty()).then_some(state.gist_manager.index);
    let mut title = format!(
        "Gists {}  ·  sort:{}  ·  type:{}  ·  ★ {}  ·  ⑂ {}",
        count_label(groups.len(), state.gist_groups().len()),
        state.gist_manager.sort.label(),
        state.gist_manager.type_filter.label(),
        state.starred_gist_count(),
        state.owned_fork_gist_count()
    );
    if !state.gist_manager.filter_query.is_empty() {
        title.push_str(&format!("  ·  /{}", state.gist_manager.filter_query));
    }
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.theme.accent))
                .style(state.theme.base_style())
                .padding(Padding::horizontal(1)),
        )
        .style(state.theme.base_style())
        .highlight_style(
            Style::default()
                .bg(state.theme.accent)
                .fg(state.theme.fg_on_accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if state.mouse_enabled {
        layout.list = Some(PaneHit {
            rect: chunks[0],
            offset: list_state.offset(),
        });
    }

    if state.gist_manager.filtering {
        render_footer_line(
            frame,
            chunks[1],
            &ftitle,
            input_line("/", &state.gist_manager.filter_query, ""),
            &state.theme,
            layout,
        );
    } else {
        render_footer(
            frame,
            chunks[1],
            &ftitle,
            &footer,
            colored,
            &state.theme,
            layout,
        );
    }
    if state.mouse_enabled {
        layout.close_button = Some(render_close_button(frame, area, &state.theme));
    }
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

pub(super) fn revision_row_label(
    rev: &crate::domain::GistRevision,
    index: usize,
    now: u64,
) -> String {
    let age = crate::domain::parse_rfc3339_to_unix(&rev.committed_at)
        .map(|t| crate::domain::humanize_age(now as i64 - t as i64))
        .unwrap_or_else(|| "?".into());
    let delta = format!(
        "+{}/-{}",
        rev.change_status.additions, rev.change_status.deletions
    );
    let sha = crate::domain::short_sha(&rev.version);
    let current = if index == 0 { " (current)" } else { "" };
    format!(
        "#{}  {} ago  {}  {}  {}{}",
        index + 1,
        age,
        delta,
        rev.user,
        sha,
        current
    )
}

pub(super) fn render_revisions(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, state.mouse_enabled, layout);
    let (ftitle, footer, colored) = if let Some(message) = &state.status {
        (String::new(), message.clone(), false)
    } else if state.revision.entries.is_none() {
        (String::new(), "Loading revisions…".to_string(), false)
    } else if let Some(err) = &state.revision.fetch_error {
        (String::new(), err.clone(), false)
    } else {
        let file = state.revision_target_file_label();
        (String::new(), format!("file={file}"), false)
    };
    let footer_lines = wrap_line_count(&footer, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(footer_lines)])
        .split(area);

    let gist_id = state.revision.gist_id.as_deref().unwrap_or("");
    let label = state
        .group_by_id(gist_id)
        .map(|g| {
            if g.description.trim().is_empty() {
                g.id.clone()
            } else {
                g.description.clone()
            }
        })
        .unwrap_or_else(|| gist_id.to_string());
    let count = state
        .revision
        .entries
        .as_ref()
        .map(|e| e.len())
        .unwrap_or(0);
    let now = unix_now();

    let items: Vec<ListItem> =
        match &state.revision.entries {
            None => vec![ListItem::new("  ⏳ Loading revisions…")
                .style(Style::default().fg(state.theme.dim))],
            Some(entries) if entries.is_empty() => {
                vec![ListItem::new("  📭 No revisions found")
                    .style(Style::default().fg(state.theme.dim))]
            }
            Some(entries) => entries
                .iter()
                .enumerate()
                .map(|(i, rev)| {
                    ListItem::new(hscroll_str(
                        &revision_row_label(rev, i, now),
                        state.revision.hscroll,
                    ))
                })
                .collect(),
        };

    let selected = (count > 0).then_some(state.revision.index);
    let title = format!("Revisions: {label} {}", count_label(count, count));
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.theme.accent))
                .style(state.theme.base_style())
                .padding(Padding::horizontal(1)),
        )
        .style(state.theme.base_style())
        .highlight_style(
            Style::default()
                .bg(state.theme.accent)
                .fg(state.theme.fg_on_accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if state.mouse_enabled {
        layout.list = Some(PaneHit {
            rect: chunks[0],
            offset: list_state.offset(),
        });
    }
    render_footer(
        frame,
        chunks[1],
        &ftitle,
        &footer,
        colored,
        &state.theme,
        layout,
    );
    if state.mouse_enabled {
        layout.close_button = Some(render_close_button(frame, area, &state.theme));
    }
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
    let files = state.gist_file_display_names(gist_id);
    let mut lines: Vec<Line> = vec![
        Line::from(gist_info_line(
            &group,
            unix_now(),
            state.current_user_login.as_deref(),
            state.gist_is_starred(gist_id),
            state.gist_counts(gist_id),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("Files ({})", files.len()),
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];
    let cursor = state.detail.file_cursor.min(files.len().saturating_sub(1));
    // Visible file rows = area height minus borders(2), info line, blank, "Files (n)" header (3).
    let visible_rows = (area.height as usize).saturating_sub(5);
    let offset = file_list_scroll(cursor, visible_rows, files.len());
    lines.extend(file_rows(
        &files,
        cursor,
        offset,
        visible_rows,
        false,
        &state.theme,
    ));
    frame.render_widget(
        Paragraph::new(lines).style(state.theme.base_style()).block(
            Block::default()
                .title(gist_block_title(&group))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.theme.accent))
                .style(state.theme.base_style())
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

/// The gist detail header: a block holding the basic-info line and the `Files │ Comments`
/// focus tabs. The active tab's content is rendered below it.
fn render_detail_header(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    gist_id: &str,
    layout: &mut MouseLayout,
) {
    let Some(group) = state.group_by_id(gist_id) else {
        return;
    };
    let lines = vec![
        Line::from(gist_info_line(
            &group,
            unix_now(),
            state.current_user_login.as_deref(),
            state.gist_is_starred(gist_id),
            state.gist_counts(gist_id),
        )),
        detail_focus_tabs_line(state.detail.focus, &state.theme),
    ];
    frame.render_widget(
        Paragraph::new(lines).style(state.theme.base_style()).block(
            Block::default()
                .title(gist_block_title(&group))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.theme.accent))
                .style(state.theme.base_style())
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
    if state.mouse_enabled {
        // Tab line is the 2nd content row (border + gist-info line above it); content starts
        // after the left border (1) + horizontal padding (1). Labels: " Files " (7), " │ " (3),
        // " Comments " (10) — see detail_focus_tabs_line.
        let content_x = area.x + 2;
        let tabs_y = area.y + 2;
        layout.detail_tab_files = Some(Rect::new(content_x, tabs_y, 7, 1));
        layout.detail_tab_comments = Some(Rect::new(content_x + 10, tabs_y, 10, 1));
    }
}

/// The gist detail's Files tab: the numbered, cursor-highlighted, scrollable file list,
/// titled with the file count.
fn render_gist_file_list(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    gist_id: &str,
    layout: &mut MouseLayout,
) {
    let files = state.gist_file_display_names(gist_id);
    let cursor = state.detail.file_cursor.min(files.len().saturating_sub(1));
    let visible_rows = (area.height as usize).saturating_sub(2);
    let offset = file_list_scroll(cursor, visible_rows, files.len());
    if state.mouse_enabled {
        layout.detail_files = Some(PaneHit { rect: area, offset });
    }
    let lines = file_rows(&files, cursor, offset, visible_rows, true, &state.theme);
    frame.render_widget(
        Paragraph::new(lines).style(state.theme.base_style()).block(
            Block::default()
                .title(format!("Files ({})", files.len()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.theme.accent))
                .style(state.theme.base_style())
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

/// Build the styled lines for a run of comments (author·age header, 2-space-indented body,
/// trailing blank). Pure so the count below can mirror it exactly for scroll compensation.
pub(super) fn comment_lines<'a>(
    comments: &'a [GistComment],
    theme: &Theme,
    now: i64,
) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    for c in comments {
        let age = crate::domain::parse_rfc3339_to_unix(&c.created_at)
            .map(|t| crate::domain::humanize_age(now - t as i64))
            .unwrap_or_else(|| "?".into());
        lines.push(Line::from(Span::styled(
            format!("{} · {age}", c.author),
            Style::default().fg(theme.accent),
        )));
        for raw in c.body.lines() {
            lines.push(Line::from(format!("  {raw}")));
        }
        lines.push(Line::from(""));
    }
    lines
}

/// Dim helper line (matches the existing dim placeholder styling).
fn dim_line<'a>(text: &'a str, state: &AppState) -> Line<'a> {
    Line::from(Span::styled(text, Style::default().fg(state.theme.dim)))
}

/// Title for the comments block: shows the loaded range out of the total when known.
fn comments_title(state: &AppState) -> String {
    match (&state.detail.comments, state.detail.comments_total) {
        (Some(c), _) if state.detail.comments_error.is_some() => format!("Comments ({})", c.len()),
        (Some(c), Some(total)) if !c.is_empty() => {
            let loaded = c.len() as u32;
            let first = total.saturating_sub(loaded) + 1;
            format!("Comments ({first}–{total} / {total})")
        }
        (Some(c), None) if !c.is_empty() => format!("Comments (newest {})", c.len()),
        _ => "Comments".to_string(),
    }
}

/// Comments pane: loading / error / empty / a paged window (newest page first; older pages
/// prepended). Renders a clickable "load older" affordance at the top, styles each comment,
/// and publishes the affordance rect + max scroll into `layout`.
pub(super) fn render_gist_comments(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    layout: &mut MouseLayout,
) {
    let now = unix_now();
    let mut body: Vec<Line> = Vec::new();
    let mut affordance_present = false;

    match (
        &state.detail.comments,
        state.detail.comments_loading,
        &state.detail.comments_error,
    ) {
        (None, true, _) => body.push(dim_line("Loading comments…", state)),
        (None, false, _) => body.push(dim_line("Tab here to load comments", state)),
        (Some(_), _, Some(err)) => body.push(Line::from(Span::styled(
            format!("comments error: {err}"),
            Style::default().fg(state.theme.del_color),
        ))),
        (Some(comments), _, None) if comments.is_empty() => {
            body.push(dim_line("No comments", state))
        }
        (Some(comments), _, None) => {
            // Top affordance line: load-older / loading / start-of-thread.
            let label = if state.detail.comments_loading_more {
                "Loading…"
            } else if state.detail.comments_loaded_oldest_page > 1 {
                affordance_present = true;
                "↑ Load 30 older comments"
            } else {
                "— Start of thread —"
            };
            body.push(dim_line(label, state));
            body.push(Line::from(""));
            body.extend(comment_lines(comments, &state.theme, now as i64));
        }
    }

    // Record the affordance hit region (line 0 inside the bordered area) for mouse clicks.
    layout.comments_load_older = if affordance_present {
        Some(Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            1,
        ))
    } else {
        None
    };

    // The scrollbar uses the logical line count, which shares units with `detail_scroll`, so the
    // thumb position is exact (its size is approximate when long comments soft-wrap).
    let total_lines = body.len();
    let inner_rows = area.height.saturating_sub(2);
    layout.comments_max_scroll = Some((total_lines as u16).saturating_sub(inner_rows));

    let title = comments_title(state);
    frame.render_widget(
        Paragraph::new(body)
            .style(state.theme.base_style())
            .scroll((state.detail.scroll, 0))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .style(state.theme.base_style()),
            ),
        area,
    );
    render_text_scrollbar(frame, area, total_lines, state.detail.scroll as usize);
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
pub(super) fn detail_focus_tabs_line(focus: DetailFocus, theme: &Theme) -> Line<'static> {
    let active = detail_focus_tab(focus);
    let active_style = Style::default()
        .fg(theme.fg_on_accent)
        .bg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let idle_style = Style::default().fg(theme.dim);
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

pub(super) fn render_gist_detail(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, state.mouse_enabled, layout);
    let (footer, colored) = footer_with_status(state.status.as_deref(), MINIMAL_HINT);
    // Fixed 4-row header (borders + basic-info line + focus tabs); the active tab — the file
    // list or the comments, never both — fills the rest above the footer.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(3),
            Constraint::Length(footer_height(&footer, area.width, "")),
        ])
        .split(area);
    if let Some(id) = state.detail.gist_id.as_deref() {
        render_detail_header(frame, chunks[0], state, id, layout);
        match state.detail.focus {
            DetailFocus::Files => render_gist_file_list(frame, chunks[1], state, id, layout),
            DetailFocus::Comments => render_gist_comments(frame, chunks[1], state, layout),
        }
    }
    render_footer(frame, chunks[2], "", &footer, colored, &state.theme, layout);

    let edit_modal = if state.editing_description {
        // The modal covers the file list and tabs; drop their hit regions so a click
        // behind the modal doesn't move the cursor or switch tabs.
        layout.detail_files = None;
        layout.detail_tab_files = None;
        layout.detail_tab_comments = None;
        layout.comments_load_older = None;
        Some(render_centered_modal_input(
            frame,
            "Edit description (Enter apply · Esc cancel)",
            "",
            &state.description_input,
            "",
            state.theme.accent,
            &state.theme,
        ))
    } else {
        None
    };
    if state.mouse_enabled {
        // When the edit-description modal is open, the close button belongs on it;
        // otherwise it sits on the full-screen detail view's top-right corner.
        layout.close_button = Some(render_close_button(
            frame,
            edit_modal.unwrap_or(area),
            &state.theme,
        ));
    }
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

pub(super) fn gist_row_display(g: &RankedGistFile, view: GistView, state: &AppState) -> String {
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
    format!(
        "{}{}",
        gist_badge_prefix(state.gist_is_starred(&g.file.gist_id), g.file.is_fork()),
        base
    )
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

pub(super) fn render_list(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, state.mouse_enabled, layout);
    let footer_body = if state.filtering {
        let (pane, query) = match state.focus {
            FocusPane::Local => ("local", &state.local_filter_query),
            FocusPane::Gist => ("gist", &state.filter_query),
        };
        format!("filter {pane}: {query}_   (Tab next pane · Enter apply · Esc clear)")
    } else {
        match &state.status {
            Some(message) => message.clone(),
            None => MINIMAL_HINT.to_string(),
        }
    };
    // Only the command-hint variant gets key colouring; filter input and status stay plain.
    let footer_is_command = !state.filtering && state.status.is_none();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(footer_height(&footer_body, area.width, "")),
        ])
        .split(area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    // Show each candidate's path relative to cwd; in flat mode this is just the filename,
    // in recursive mode it includes the subdirectory (e.g. src/utils/helpers.rs).
    // Dual snapshot once for both panes (issue #224) — avoids a second full recompute
    // when building the gist pane from `ranked_gists()` below.
    let (visible_locals, ranked) = state.list_pane_snapshots();
    let local_items: Vec<ListItem> = if state.local_scanning && state.locals.is_empty() {
        vec![ListItem::new(format!(
            "  {} Scanning files…",
            spinner_glyph(state.spinner_frame)
        ))
        .style(Style::default().fg(state.theme.dim))]
    } else if state.locals.is_empty() {
        vec![ListItem::new("  📭 No local files found").style(Style::default().fg(state.theme.dim))]
    } else if visible_locals.is_empty() {
        vec![ListItem::new("  🔍 No files match the filter")
            .style(Style::default().fg(state.theme.dim))]
    } else {
        visible_locals
            .iter()
            .map(|r| {
                let base = super::text::local_row_label(&r.candidate.path, &state.cwd);
                marked_item(base, row_mark(&r.reasons), state.local_hscroll)
            })
            .collect()
    };
    let local_focused = state.focus == FocusPane::Local;
    let local_selected = (!visible_locals.is_empty()).then_some(state.local_index);
    let recursive_marker = if state.local_recursive { " [↓]" } else { "" };
    let scanning_marker = if state.local_scanning { " …" } else { "" };
    let mut local_title = format!(
        "[1] Local {} · {}{}{} · sort:{}",
        count_label(visible_locals.len(), state.locals.len()),
        crate::config::display_path(&state.cwd),
        recursive_marker,
        scanning_marker,
        state.local_sort.label()
    );
    if !state.local_filter_query.is_empty() {
        local_title.push_str(&format!(" · /{}", state.local_filter_query));
    }
    // Mark the pane that currently drives the match ranking (the anchor).
    let local_title = if state.anchor == FocusPane::Local {
        format!("{local_title} · ⚓")
    } else {
        local_title
    };
    let local_offset = render_pane(
        frame,
        columns[0],
        &local_title,
        local_items,
        local_focused,
        local_selected,
        &state.theme,
    );
    if state.mouse_enabled {
        layout.local = Some(PaneHit {
            rect: columns[0],
            offset: local_offset,
        });
    }

    let gist_items: Vec<ListItem> = if state.loading && ranked.is_empty() {
        vec![ListItem::new(format!(
            "  {} Loading gists…",
            spinner_glyph(state.spinner_frame)
        ))
        .style(Style::default().fg(state.theme.dim))]
    } else if ranked.is_empty() {
        let message = if !state.filter_query.is_empty() {
            ListItem::new("  🔍 No gists match the filter")
                .style(Style::default().fg(state.theme.dim))
        } else {
            ListItem::new("  📭 No gists found").style(Style::default().fg(state.theme.dim))
        };
        vec![message]
    } else {
        ranked
            .iter()
            .map(|g| {
                let base = gist_row_display(g, state.gist_view, state);
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
    let gist_offset = render_pane(
        frame,
        columns[1],
        &gist_title,
        gist_items,
        gist_focused,
        gist_selected,
        &state.theme,
    );
    if state.mouse_enabled {
        layout.gist = Some(PaneHit {
            rect: columns[1],
            offset: gist_offset,
        });
    }

    if state.filtering {
        let (pane, query) = match state.focus {
            FocusPane::Local => ("local", &state.local_filter_query),
            FocusPane::Gist => ("gist", &state.filter_query),
        };
        let line = input_line(
            &format!("filter {pane}: "),
            query,
            "   (Tab next pane · Enter apply · Esc clear)",
        );
        render_footer_line(frame, chunks[1], "", line, &state.theme, layout);
    } else {
        render_footer(
            frame,
            chunks[1],
            "",
            &footer_body,
            footer_is_command,
            &state.theme,
            layout,
        );
    }
}

pub(super) fn render_pane(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: Vec<ListItem>,
    focused: bool,
    selected: Option<usize>,
    theme: &Theme,
) -> usize {
    let item_count = items.len();
    let border_style = if focused {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.dim)
    };
    // The border colour alone signals which pane is active; row text stays at full
    // brightness in both panes so it is always legible.
    // Focused selection is a solid bar (whole row); unfocused just bolds the row.
    let highlight_style = if focused {
        Style::default()
            .bg(theme.accent)
            .fg(theme.fg_on_accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                // Pin title to theme fg so it stays legible in both dark and light modes.
                .title_style(Style::default().fg(theme.fg))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style)
                .style(theme.base_style())
                .padding(Padding::horizontal(1)),
        )
        .style(theme.base_style())
        .highlight_style(highlight_style)
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(selected);
    frame.render_stateful_widget(list, area, &mut list_state);
    let offset = list_state.offset();

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
    offset
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
pub(super) fn inline_del_line(
    del_line: &str,
    ins_line: &str,
    hscroll: usize,
    del_color: Color,
) -> Line<'static> {
    let del_content = del_line.get(1..).unwrap_or("");
    let ins_content = ins_line.get(1..).unwrap_or("");
    let mut spans = vec![Span::styled("-", Style::default().fg(del_color))];
    for change in TextDiff::from_words(del_content, ins_content).iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(del_color).add_modifier(Modifier::BOLD),
            )),
            ChangeTag::Equal => spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(del_color),
            )),
            ChangeTag::Insert => {}
        }
    }
    apply_hscroll_spans(spans, hscroll)
}

/// Ins line with word-level highlighting: changed words bold-green, unchanged words plain green.
pub(super) fn inline_ins_line(
    del_line: &str,
    ins_line: &str,
    hscroll: usize,
    ins_color: Color,
) -> Line<'static> {
    let del_content = del_line.get(1..).unwrap_or("");
    let ins_content = ins_line.get(1..).unwrap_or("");
    let mut spans = vec![Span::styled("+", Style::default().fg(ins_color))];
    for change in TextDiff::from_words(del_content, ins_content).iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(ins_color).add_modifier(Modifier::BOLD),
            )),
            ChangeTag::Equal => spans.push(Span::styled(
                change.value().to_string(),
                Style::default().fg(ins_color),
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

/// Label and trailing hint around the create flow's description input. Shared so
/// `confirm_prompt` (plain text / tests) and `render_confirm` (the cursor-aware modal)
/// can't drift apart.
pub(super) const CREATE_DESC_PREFIX: &str = "Description (optional): ";
pub(super) const CREATE_DESC_SUFFIX: &str = "   ·  Enter next  ·  Esc cancel";

/// The prompt shown inside the centered confirm modal — one line per pending action,
/// listing the keys that resolve it. Pure so it can be unit-tested.
pub(super) fn confirm_prompt(state: &AppState) -> String {
    match &state.pending_action {
        Some(PendingAction::Create { .. }) if state.editing_description => {
            // `_` is the plain-text caret; the rendered modal draws a reverse-video
            // cursor at its real position instead (see render_confirm).
            format!(
                "{CREATE_DESC_PREFIX}{}_{CREATE_DESC_SUFFIX}",
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
            gist_id: _,
            filename: _,
            local_path: _,
        }) if state.upload.watching => {
            format!(
                "{} watching for edits — close the editor to continue  ·  n cancel",
                spinner_glyph(state.spinner_frame)
            )
        }
        Some(PendingAction::Upload {
            gist_id,
            filename,
            local_path,
        }) => {
            let edited_status = if state.upload.edited_content.is_some() {
                " [edited]"
            } else {
                ""
            };
            let mut opts = format!("y yes  n/Esc cancel  e edit{edited_status}");
            if is_json_file(local_path) {
                let pretty_status = if state.upload.json_pretty {
                    " [on]"
                } else {
                    " [off]"
                };
                let sort_status = if state.upload.json_sort {
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
        Some(PendingAction::RestoreRevision {
            filename,
            version_label,
            ..
        }) => {
            format!(
                "Restore {filename} to revision {version_label}? This uploads old content as a new revision. (y/n)"
            )
        }
        _ => format!(
            "Overwrite {}? (y/n)",
            crate::config::display_path(&state.download_target)
        ),
    }
}

/// Title and border colour for the confirm modal. Destructive actions are tinted with the
/// theme's `del_color` so the stakes read at a glance; non-destructive writes use the neutral
/// `notice_color` prompt.
pub(super) fn confirm_modal_style(state: &AppState) -> (&'static str, Color) {
    let theme = &state.theme;
    match &state.pending_action {
        Some(PendingAction::Create { .. }) if state.editing_description => {
            ("Description", theme.accent)
        }
        Some(PendingAction::Create { .. }) => ("Create gist", theme.notice_color),
        Some(PendingAction::Upload { .. }) => ("Upload", theme.notice_color),
        Some(PendingAction::Delete { .. }) => ("Delete", theme.del_color),
        Some(PendingAction::RemoveFile { .. }) => ("Remove file", theme.del_color),
        Some(PendingAction::CompactGist { .. }) => ("Compact revisions", theme.del_color),
        Some(PendingAction::RestoreRevision { .. }) => ("Restore revision", theme.notice_color),
        _ => ("Overwrite", theme.del_color),
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
    let block = Block::default()
        .title(diff_title(state))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(state.theme.base_style())
        .padding(Padding::horizontal(1));
    let paragraph = if state.diff_wrap {
        // Wrapping needs the full, un-h-scrolled line set; vertical scroll goes through
        // Paragraph. Mirrors render_preview's wrap branch.
        Paragraph::new(diff_view_highlighted(
            &diff_body,
            0,
            0,
            ext.as_deref(),
            state.syntax_highlight,
            &state.theme,
        ))
        .style(state.theme.base_style())
        .scroll((state.diff_scroll, 0))
        .wrap(Wrap { trim: false })
        .block(block)
    } else {
        Paragraph::new(diff_view_highlighted(
            &diff_body,
            state.diff_scroll,
            state.diff_hscroll,
            ext.as_deref(),
            state.syntax_highlight,
            &state.theme,
        ))
        .style(state.theme.base_style())
        .block(block)
    };
    frame.render_widget(paragraph, area);
    // The scrollbar's 1:1 line↔row mapping only holds without soft wrapping (see render_preview).
    if !state.diff_wrap {
        let total_lines = diff_body.lines().count();
        render_text_scrollbar(frame, area, total_lines, state.diff_scroll as usize);
    }
}

/// The `Screen::Diff` preview: the diff pane plus a scroll/commands footer.
///
/// #72 audit: this footer intentionally does not surface `state.status`. Diff actions (`d`/`u`)
/// transition to `Screen::Confirm` or to the IO that lands back on `List`; their results surface
/// on those destination screens (which read `state.status`), so no status is set while on Diff.
/// Footer hints for `Screen::Diff` (pure for tests).
pub(super) fn diff_footer(state: &AppState) -> String {
    let context = if state.diff_show_full {
        "c context [full]".to_string()
    } else {
        format!("c context [{}]", state.diff_context)
    };
    // When wrapping, horizontal scroll (←→) is meaningless — drop it from the hint.
    let scroll = if state.diff_wrap {
        "↑↓ PgUp/Dn scroll"
    } else {
        "↑↓←→ PgUp/Dn scroll"
    };
    let wrap = if state.diff_wrap {
        "w wrap [on]"
    } else {
        "w wrap [off]"
    };
    let back = "Esc/q back";
    if !state.diff_allows_sync() {
        if state.diff_identical {
            format!("Files are identical  ·  {scroll}  ·  {wrap}  ·  {context}  ·  {back}")
        } else {
            format!("{scroll}  ·  {wrap}  ·  {context}  ·  {back}")
        }
    } else if state.diff_identical {
        format!("Files are identical — nothing to sync  ·  {scroll}  ·  {wrap}  ·  {context}  ·  {back}")
    } else {
        format!("{scroll}  ·  d download  ·  u upload  ·  {wrap}  ·  {context}  ·  {back}")
    }
}

pub(super) fn render_diff(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    let footer = diff_footer(state);

    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, state.mouse_enabled, layout);
    let footer_lines = wrap_line_count(&footer, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(footer_lines)])
        .split(area);

    render_diff_pane(frame, chunks[0], state);

    render_footer(frame, chunks[1], "", &footer, true, &state.theme, layout);
    if state.mouse_enabled {
        layout.close_button = Some(render_close_button(frame, area, &state.theme));
    }
}

/// `Screen::Confirm`: the diff fills the screen as context behind a centered prompt modal,
/// keeping the overwrite gate's diff visible while the question is asked front-and-centre.
/// #72 audit: this modal intentionally does not surface `state.status`. It is a transient y/n
/// gate — confirming executes the action and transitions to `List`/`Gists`, where the result
/// status is shown; cancelling returns to the launching screen without setting a status here.
pub(super) fn render_confirm(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    match &state.pending_action {
        Some(PendingAction::CompactGist { gist_id, .. }) => {
            render_gist_info_and_files(frame, frame.area(), state, gist_id);
        }
        _ => render_diff_pane(frame, frame.area(), state),
    }
    let (title, border) = confirm_modal_style(state);
    // The create flow's description step is an active text input, so it needs the
    // reverse-video cursor; every other confirm prompt is static text.
    let modal = if matches!(state.pending_action, Some(PendingAction::Create { .. }))
        && state.editing_description
    {
        render_centered_modal_input(
            frame,
            title,
            CREATE_DESC_PREFIX,
            &state.description_input,
            CREATE_DESC_SUFFIX,
            border,
            &state.theme,
        )
    } else {
        render_centered_modal(frame, title, &confirm_prompt(state), border, &state.theme)
    };
    if state.mouse_enabled {
        // Put the close button on the modal box itself, not the full-screen corner.
        layout.close_button = Some(render_close_button(frame, modal, &state.theme));
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

fn modal_block(title: &str, border: Color, theme: &Theme) -> Block<'static> {
    Block::default()
        .title(title.to_string())
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
            .block(modal_block(title, border, theme)),
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
            .block(modal_block(title, border, theme)),
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
pub(super) fn palette_key_width(items: &[&PaletteItem]) -> usize {
    items
        .iter()
        .map(|item| item.key_hint.chars().count())
        .max()
        .unwrap_or(1)
}

/// One palette row: indented key column + gap + label. Pure for unit tests.
pub(super) fn palette_row_line(
    item: &PaletteItem,
    key_width: usize,
    theme: &Theme,
    row_style: Style,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<key_width$}  ", item.key_hint, key_width = key_width),
            Style::default().fg(action_color(&item.label, theme)),
        ),
        Span::styled(item.label.clone(), row_style),
    ])
}

/// The unified context menu / command palette drawn over the screen it was opened from.
fn render_palette(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    let mut bg_layout = MouseLayout::default();
    match state.palette.origin_screen {
        Screen::List => render_list(frame, state, &mut bg_layout),
        Screen::Diff => render_diff(frame, state, &mut bg_layout),
        Screen::Preview => render_preview(frame, state, &mut bg_layout),
        Screen::Help => render_help(frame, state, &mut bg_layout),
        Screen::Pins => render_pins(frame, state, &mut bg_layout),
        Screen::Gists => render_gists(frame, state, &mut bg_layout),
        Screen::GistDetail => render_gist_detail(frame, state, &mut bg_layout),
        Screen::Revisions => render_revisions(frame, state, &mut bg_layout),
        Screen::Confirm | Screen::Palette => {}
    }

    let area = frame.area();
    let visible = state.palette_visible_items();
    let has_query = state.palette.mode == PaletteMode::Command;
    let title = match state.palette.mode {
        PaletteMode::Menu => "Menu",
        PaletteMode::Command => "Command palette",
    };
    let key_width = palette_key_width(&visible);
    let body_lines = visible.len() + usize::from(has_query);
    let longest_row = visible
        .iter()
        .map(|item| 2 + key_width + 2 + item.label.chars().count());
    let content_width = longest_row.max().unwrap_or(20) as u16;
    let width = if has_query {
        (area.width * 70 / 100).clamp(
            content_width.saturating_add(4),
            area.width.saturating_sub(2).max(1),
        )
    } else {
        (area.width * 45 / 100).clamp(
            content_width.saturating_add(4),
            area.width.saturating_sub(2).max(1),
        )
    };
    let max_h = area.height.saturating_sub(2).max(1) as usize;
    let height = (body_lines + 2).clamp(3, max_h) as u16;
    let (x, y) = match (state.palette.mode, state.palette.anchor) {
        (PaletteMode::Menu, Some((col, row))) => (
            col.saturating_sub(width / 2)
                .min(area.width.saturating_sub(width)),
            row.saturating_sub(1)
                .min(area.height.saturating_sub(height)),
        ),
        _ => (
            area.width.saturating_sub(width) / 2,
            area.height.saturating_sub(height).saturating_sub(1),
        ),
    };
    let rect = Rect::new(x, y, width, height);

    frame.render_widget(Clear, rect);

    layout.palette_rows.clear();
    let dim = Style::default().fg(state.theme.dim);
    let active = Style::default()
        .fg(state.theme.fg_on_accent)
        .bg(state.theme.accent)
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line<'static>> = Vec::new();
    if has_query {
        lines.push(input_line("> ", &state.palette.query, ""));
    }
    if visible.is_empty() {
        lines.push(Line::from(Span::styled("  (no matches)", dim)));
    } else {
        for (i, item) in visible.iter().enumerate() {
            let row_style = if i == state.palette.selected {
                active
            } else if item.enabled {
                state.theme.base_style()
            } else {
                Style::default().fg(state.theme.dim)
            };
            lines.push(palette_row_line(item, key_width, &state.theme, row_style));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(state.theme.base_style()).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.theme.accent))
                .style(state.theme.base_style()),
        ),
        rect,
    );

    let inner = rect.inner(Margin::new(1, 1));
    let mut y = inner.y + u16::from(has_query);
    for item in visible.iter() {
        if y >= inner.bottom() {
            break;
        }
        if state.mouse_enabled && item.enabled {
            layout
                .palette_rows
                .push(Rect::new(inner.x, y, inner.width, 1));
        }
        y = y.saturating_add(1);
    }
    if state.mouse_enabled {
        layout.palette_close = Some(render_close_button(frame, rect, &state.theme));
    }
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
