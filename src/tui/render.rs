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
    // Pure presentation seam (issues #241 / #250): every screen paints from the view model.
    // Pin sync IO is never done here — only cache reads.
    let vm = super::build_view_model(state);
    match &vm.screen {
        super::ScreenVm::List(list) => render_list_vm(frame, state, list, &vm.chrome, layout),
        super::ScreenVm::Gists(gists) => render_gists_vm(frame, state, gists, &vm.chrome, layout),
        super::ScreenVm::GistDetail(detail) => {
            render_gist_detail_vm(frame, state, detail, &vm.chrome, layout)
        }
        super::ScreenVm::Revisions(revs) => {
            render_revisions_vm(frame, state, revs, &vm.chrome, layout)
        }
        super::ScreenVm::Config(config) => {
            render_config_vm(frame, state, config, &vm.chrome, layout)
        }
        super::ScreenVm::Diff(diff) => render_diff_vm(frame, state, diff, &vm.chrome, layout),
        super::ScreenVm::Preview(preview) => {
            render_preview_vm(frame, state, preview, &vm.chrome, layout)
        }
        super::ScreenVm::Pins(pins) => render_pins_vm(frame, state, pins, &vm.chrome, layout),
        super::ScreenVm::Confirm(confirm) => {
            render_confirm_vm(frame, state, confirm, &vm.chrome, layout)
        }
        super::ScreenVm::Help(help) => render_help_vm(frame, state, help, &vm.chrome, layout),
        super::ScreenVm::Palette(palette) => {
            render_palette_vm(frame, state, palette, &vm.chrome, layout)
        }
    }
    if let Some(ref msg) = vm.chrome.bg_task_msg {
        render_loading_overlay(frame, msg, vm.chrome.spinner_frame, &state.theme);
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

pub(super) fn help_topic_body(topic: HelpTopic) -> &'static str {
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
  Top bar    click (G)ists / (P)ins / (C)onfig / (?)Help (top-right, every screen)
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
        HelpTopic::Config => {
            "\
Settings (C, top-bar (C)onfig, or Ctrl+p → Open settings)
  Up/Down    move between fields (also j / k)
  Enter/Space  toggle a boolean, or increase a number
  h / l      decrease / increase (also ← / →)
  Esc / q    close (opening Settings never writes config by itself —
             values are saved only after you change a field)

Fields
  Theme                  dark / light (also global T)
  Mouse support          on / off (session still respects --no-mouse)
  Check for updates      on / off (session still respects --no-update-check)
  Ignore trailing newline  on / off (diff + overwrite confirm)
  Recursive scan depth   0–20 (r recursive discovery)
  Diff context lines     0–50 (c in Diff still toggles full vs this radius)"
        }
        HelpTopic::General => {
            "\
  Esc / q    close an overlay; from the list, press twice to quit the app
  ?          show this help
  C          open Settings (flat list of preferences; also Ctrl+p)
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
pub(super) const ABOUT_REPO_LINE: usize = 2;

/// Plain-text About topic lines for the pure view-model (issue #241). Paint re-applies the
/// underlined repo style from [`ABOUT_REPO_LINE`].
pub(super) fn about_topic_lines_plain(state: &AppState) -> Vec<String> {
    let repo = env!("CARGO_PKG_REPOSITORY")
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let mut lines = vec![
        format!("gistui v{}", env!("CARGO_PKG_VERSION")),
        String::new(),
        format!("  {repo}"),
        String::new(),
    ];
    if let Some(latest) = &state.update_available {
        lines.push(crate::update_check::update_hint(
            latest,
            &state.install_method,
        ));
    }
    lines
}

pub(super) fn render_config(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    let chrome = super::view_model::build_chrome(state);
    let config = super::view_model::build_config_vm(state);
    render_config_vm(frame, state, &config, &chrome, layout);
}

fn render_config_vm(
    frame: &mut Frame,
    state: &AppState,
    config: &super::view_model::ConfigVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);
    let items: Vec<ListItem> = config
        .rows
        .iter()
        .map(|row| ListItem::new(row.clone()))
        .collect();
    let mut list_state = ListState::default().with_selected(Some(config.selected));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Settings ")
        .title_bottom(Line::from(
            " Esc close · Enter/←/→ change · saved on change ",
        ))
        .style(state.theme.base_style())
        .border_style(Style::default().fg(state.theme.accent));
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(state.theme.accent)
                .fg(state.theme.fg_on_accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if chrome.mouse_enabled {
        layout.close_button = Some(render_close_button(frame, chunks[0], &state.theme));
        layout.list = Some(PaneHit {
            rect: chunks[0],
            offset: 0,
        });
    }
    if let Some(ref status) = config.status {
        frame.render_widget(
            Paragraph::new(status.as_str()).style(Style::default().fg(state.theme.accent)),
            chunks[1],
        );
    }
}

pub(super) fn render_help(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    // Build Help body directly so Palette-over-Help still paints (screen is Palette).
    let chrome = super::view_model::build_chrome(state);
    let help = super::view_model::build_help_vm(state);
    render_help_vm(frame, state, &help, &chrome, layout);
}

fn render_help_vm(
    frame: &mut Frame,
    state: &AppState,
    help: &super::view_model::HelpVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    match &help.mode {
        super::view_model::HelpModeVm::Index { items, selected } => {
            let list_items: Vec<ListItem> = items
                .iter()
                .map(|item| ListItem::new(format!("  {}  {}", item.key, item.title)))
                .collect();
            let list = List::new(list_items)
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
            list_state.select(Some(*selected));
            frame.render_stateful_widget(list, area, &mut list_state);
            if chrome.mouse_enabled {
                layout.list = Some(PaneHit {
                    rect: area,
                    offset: list_state.offset(),
                });
            }
        }
        super::view_model::HelpModeVm::Topic {
            title,
            lines,
            scroll,
            about_repo_line,
        } => {
            let body_lines: Vec<Line<'static>> = lines
                .iter()
                .enumerate()
                .map(|(i, text)| {
                    if about_repo_line == &Some(i) {
                        let repo = text.trim_start();
                        let indent_len = text.len() - repo.len();
                        let indent = text[..indent_len].to_string();
                        Line::from(vec![
                            Span::raw(indent),
                            Span::styled(
                                repo.to_string(),
                                Style::default()
                                    .fg(state.theme.fg)
                                    .add_modifier(Modifier::UNDERLINED),
                            ),
                        ])
                    } else {
                        Line::from(text.clone())
                    }
                })
                .collect();
            frame.render_widget(
                Paragraph::new(Text::from(body_lines))
                    .style(state.theme.base_style())
                    .scroll((*scroll, 0))
                    .block(
                        Block::default()
                            .title(title.clone())
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .style(state.theme.base_style())
                            .padding(Padding::horizontal(1)),
                    ),
                area,
            );
            if let (Some(repo_line), true) = (*about_repo_line, chrome.mouse_enabled) {
                let visible_row = repo_line as i32 - *scroll as i32;
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
    }
    if chrome.mouse_enabled {
        layout.close_button = Some(render_close_button(frame, area, &state.theme));
    }
}

/// The preview body as per-line span vectors: syntax-highlighted when the feature is enabled and
/// the file type is known, otherwise one plain span per line.
fn preview_line_spans(
    body: &str,
    syntax_highlight: bool,
    ext: Option<&str>,
    theme: &Theme,
) -> Vec<Vec<Span<'static>>> {
    let lines: Vec<String> = body.lines().map(str::to_string).collect();
    match (syntax_highlight, ext) {
        (true, Some(ext)) => super::highlight::highlight_buffer(ext, &lines, theme),
        _ => lines.into_iter().map(|l| vec![Span::raw(l)]).collect(),
    }
}

pub(super) fn render_preview(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    let chrome = super::view_model::build_chrome(state);
    let preview = super::view_model::build_preview_vm(state);
    render_preview_vm(frame, state, &preview, &chrome, layout);
}

fn render_preview_vm(
    frame: &mut Frame,
    state: &AppState,
    preview: &super::view_model::PreviewVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    let footer_lines = wrap_line_count(&preview.footer, area.width.saturating_sub(2)).max(1);
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
            .map(|spans| apply_hscroll_spans(spans, preview.hscroll as usize))
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
        render_text_scrollbar(frame, chunks[0], total_lines, preview.scroll as usize);
    }
    render_footer(
        frame,
        chunks[1],
        "",
        &preview.footer,
        preview.footer_colored,
        &state.theme,
        layout,
    );
    if chrome.mouse_enabled {
        layout.close_button = Some(render_close_button(frame, area, &state.theme));
    }
}

pub(super) fn render_pins(frame: &mut Frame, state: &AppState, layout: &mut MouseLayout) {
    // Build Pins body directly so Palette-over-Pins still paints (screen is Palette).
    let chrome = super::view_model::build_chrome(state);
    let pins = super::view_model::build_pins_vm(state);
    render_pins_vm(frame, state, &pins, &chrome, layout);
}

fn render_pins_vm(
    frame: &mut Frame,
    state: &AppState,
    pins: &super::view_model::PinsVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    // Sync feedback (e.g. "already in sync") is carried in the Pins VM footer (see #72 / #241).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(footer_height(&pins.footer, area.width, &pins.footer_title)),
        ])
        .split(area);

    let items: Vec<ListItem> = match pins.empty {
        super::view_model::PinsEmptyKind::NoMappings => {
            vec![
                ListItem::new("  📌 No pinned mappings found (use p to pin a pair)")
                    .style(Style::default().fg(state.theme.dim)),
            ]
        }
        super::view_model::PinsEmptyKind::NoFilterMatch => {
            vec![ListItem::new("  🔍 No pins match the filter")
                .style(Style::default().fg(state.theme.dim))]
        }
        super::view_model::PinsEmptyKind::HasRows => pins
            .rows
            .iter()
            .map(|row| {
                let item = ListItem::new(hscroll_str(&row.label, pins.hscroll));
                if row.status == crate::domain::SyncStatus::Missing {
                    item.style(Style::default().fg(state.theme.del_color))
                } else {
                    item
                }
            })
            .collect(),
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(pins.title.clone())
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
    list_state.select(pins.selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if chrome.mouse_enabled {
        layout.list = Some(PaneHit {
            rect: chunks[0],
            offset: list_state.offset(),
        });
    }

    if pins.filtering {
        render_footer_line(
            frame,
            chunks[1],
            &pins.footer_title,
            input_line(
                "/",
                &state
                    .pins()
                    .expect("filtering PinsVm implies Screen::Pins payload")
                    .filter_query,
                "",
            ),
            &state.theme,
            layout,
        );
    } else {
        render_footer(
            frame,
            chunks[1],
            &pins.footer_title,
            &pins.footer,
            pins.footer_colored,
            &state.theme,
            layout,
        );
    }
    if chrome.mouse_enabled {
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
    // Build body directly so Palette-over-Gists still paints (screen is Palette).
    let chrome = super::view_model::build_chrome(state);
    let gists = super::view_model::build_gists_vm(state);
    render_gists_vm(frame, state, &gists, &chrome, layout);
}

fn render_gists_vm(
    frame: &mut Frame,
    state: &AppState,
    gists: &super::view_model::GistsVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    // Footer: filter input while filtering, else status or hints (see #72 / #250).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(footer_height(
                &gists.footer,
                area.width,
                &gists.footer_title,
            )),
        ])
        .split(area);

    let items: Vec<ListItem> = match gists.empty {
        super::view_model::GistsEmptyKind::HasRows => gists
            .rows
            .iter()
            .map(|row| ListItem::new(hscroll_str(&row.label, gists.hscroll)))
            .collect(),
        _ => {
            let msg = gists.empty_message.clone().unwrap_or_else(|| "  ".into());
            vec![ListItem::new(msg).style(Style::default().fg(state.theme.dim))]
        }
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(gists.title.clone())
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
    list_state.select(gists.selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if chrome.mouse_enabled {
        layout.list = Some(PaneHit {
            rect: chunks[0],
            offset: list_state.offset(),
        });
    }

    if gists.filtering {
        let filter_query = state
            .gist_manager()
            .map(|g| g.filter_query.clone())
            .unwrap_or_default();
        render_footer_line(
            frame,
            chunks[1],
            &gists.footer_title,
            input_line("/", &filter_query, ""),
            &state.theme,
            layout,
        );
    } else {
        render_footer(
            frame,
            chunks[1],
            &gists.footer_title,
            &gists.footer,
            gists.footer_colored,
            &state.theme,
            layout,
        );
    }
    if chrome.mouse_enabled {
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
    let chrome = super::view_model::build_chrome(state);
    let revs = super::view_model::build_revisions_vm(state);
    render_revisions_vm(frame, state, &revs, &chrome, layout);
}

fn render_revisions_vm(
    frame: &mut Frame,
    state: &AppState,
    revs: &super::view_model::RevisionsVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    let footer_lines = wrap_line_count(&revs.footer, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(footer_lines)])
        .split(area);

    let items: Vec<ListItem> = match revs.empty {
        super::view_model::RevisionsEmptyKind::HasRows => revs
            .rows
            .iter()
            .map(|row| ListItem::new(hscroll_str(row, revs.hscroll)))
            .collect(),
        _ => {
            let msg = revs.empty_message.clone().unwrap_or_else(|| "  ".into());
            vec![ListItem::new(msg).style(Style::default().fg(state.theme.dim))]
        }
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(revs.title.clone())
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
    list_state.select(revs.selected);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if chrome.mouse_enabled {
        layout.list = Some(PaneHit {
            rect: chunks[0],
            offset: list_state.offset(),
        });
    }
    render_footer(
        frame,
        chunks[1],
        "",
        &revs.footer,
        revs.footer_colored,
        &state.theme,
        layout,
    );
    if chrome.mouse_enabled {
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

/// Compaction-confirm background from a pure compact-gist view model.
fn render_compact_gist_bg_vm(
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
                .title(bg.block_title.clone())
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.accent))
                .style(theme.base_style())
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

/// The gist detail header from a view model: info line + `Files │ Comments` tabs.
fn render_detail_header_vm(
    frame: &mut Frame,
    area: Rect,
    detail: &super::view_model::GistDetailVm,
    chrome: &super::view_model::ChromeVm,
    theme: &Theme,
    layout: &mut MouseLayout,
) {
    let lines = vec![
        Line::from(detail.info_line.clone()),
        detail_focus_tabs_line(detail.focus, theme),
    ];
    frame.render_widget(
        Paragraph::new(lines).style(theme.base_style()).block(
            Block::default()
                .title(detail.block_title.clone())
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.accent))
                .style(theme.base_style())
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
    if chrome.mouse_enabled {
        // Tab line is the 2nd content row (border + gist-info line above it); content starts
        // after the left border (1) + horizontal padding (1). Labels: " Files " (7), " │ " (3),
        // " Comments " (10) — see detail_focus_tabs_line.
        let content_x = area.x + 2;
        let tabs_y = area.y + 2;
        layout.detail_tab_files = Some(Rect::new(content_x, tabs_y, 7, 1));
        layout.detail_tab_comments = Some(Rect::new(content_x + 10, tabs_y, 10, 1));
    }
}

/// Files tab from the view model (full file list; paint windows to the area height).
fn render_gist_file_list_vm(
    frame: &mut Frame,
    area: Rect,
    detail: &super::view_model::GistDetailVm,
    chrome: &super::view_model::ChromeVm,
    theme: &Theme,
    layout: &mut MouseLayout,
) {
    let files = &detail.files;
    let cursor = detail.file_cursor.min(files.len().saturating_sub(1));
    let visible_rows = (area.height as usize).saturating_sub(2);
    let offset = file_list_scroll(cursor, visible_rows, files.len());
    if chrome.mouse_enabled {
        layout.detail_files = Some(PaneHit { rect: area, offset });
    }
    let lines = file_rows(files, cursor, offset, visible_rows, true, theme);
    frame.render_widget(
        Paragraph::new(lines).style(theme.base_style()).block(
            Block::default()
                .title(format!("Files ({})", files.len()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.accent))
                .style(theme.base_style())
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

/// Comments pane from the view model: styles plain presentation facts and fills hit/scroll layout.
fn render_gist_comments_vm(
    frame: &mut Frame,
    area: Rect,
    comments: &super::view_model::CommentsPaneVm,
    theme: &Theme,
    layout: &mut MouseLayout,
) {
    use super::view_model::{CommentLineVm, CommentsAffordance, CommentsPaneVm};

    let mut body: Vec<Line> = Vec::new();
    let mut affordance_present = false;
    let mut title = "Comments".to_string();
    let mut scroll = 0u16;

    match comments {
        CommentsPaneVm::Loading => body.push(Line::from(Span::styled(
            "Loading comments…",
            Style::default().fg(theme.dim),
        ))),
        CommentsPaneVm::PromptLoad => body.push(Line::from(Span::styled(
            "Tab here to load comments",
            Style::default().fg(theme.dim),
        ))),
        CommentsPaneVm::Error { message } => body.push(Line::from(Span::styled(
            message.clone(),
            Style::default().fg(theme.del_color),
        ))),
        CommentsPaneVm::Empty => body.push(Line::from(Span::styled(
            "No comments",
            Style::default().fg(theme.dim),
        ))),
        CommentsPaneVm::Thread {
            title: t,
            affordance,
            lines,
            scroll: s,
        } => {
            title = t.clone();
            scroll = *s;
            let label = match affordance {
                CommentsAffordance::LoadingMore => "Loading…",
                CommentsAffordance::LoadOlder => {
                    affordance_present = true;
                    "↑ Load 30 older comments"
                }
                CommentsAffordance::StartOfThread => "— Start of thread —",
            };
            body.push(Line::from(Span::styled(
                label,
                Style::default().fg(theme.dim),
            )));
            body.push(Line::from(""));
            for line in lines {
                match line {
                    CommentLineVm::Author { text } => body.push(Line::from(Span::styled(
                        text.clone(),
                        Style::default().fg(theme.accent),
                    ))),
                    CommentLineVm::Body { text } => body.push(Line::from(text.clone())),
                    CommentLineVm::Blank => body.push(Line::from("")),
                }
            }
        }
    }

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

    let total_lines = body.len();
    let inner_rows = area.height.saturating_sub(2);
    layout.comments_max_scroll = Some((total_lines as u16).saturating_sub(inner_rows));

    frame.render_widget(
        Paragraph::new(body)
            .style(theme.base_style())
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .style(theme.base_style()),
            ),
        area,
    );
    render_text_scrollbar(frame, area, total_lines, scroll as usize);
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
    // Build body directly so Palette-over-GistDetail still paints (screen is Palette).
    let chrome = super::view_model::build_chrome(state);
    let detail = super::view_model::build_gist_detail_vm(state);
    render_gist_detail_vm(frame, state, &detail, &chrome, layout);
}

fn render_gist_detail_vm(
    frame: &mut Frame,
    state: &AppState,
    detail: &super::view_model::GistDetailVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    // Fixed 4-row header (borders + basic-info line + focus tabs); the active tab — the file
    // list or the comments, never both — fills the rest above the footer.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(3),
            Constraint::Length(footer_height(&detail.footer, area.width, "")),
        ])
        .split(area);
    if !detail.missing {
        render_detail_header_vm(frame, chunks[0], detail, chrome, &state.theme, layout);
        match detail.focus {
            DetailFocus::Files => {
                render_gist_file_list_vm(frame, chunks[1], detail, chrome, &state.theme, layout)
            }
            DetailFocus::Comments => {
                render_gist_comments_vm(frame, chunks[1], &detail.comments, &state.theme, layout)
            }
        }
    }
    render_footer(
        frame,
        chunks[2],
        "",
        &detail.footer,
        detail.footer_colored,
        &state.theme,
        layout,
    );

    let edit_modal = if detail.editing_description {
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
    if chrome.mouse_enabled {
        // When the edit-description modal is open, the close button belongs on it;
        // otherwise it sits on the full-screen detail view's top-right corner.
        layout.close_button = Some(render_close_button(
            frame,
            edit_modal.unwrap_or(area),
            &state.theme,
        ));
    }
}

pub(super) use super::text::hscroll_str;

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
pub(crate) enum RowMark {
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

/// Compose the full list-row string (including pin mark) that paint and hscroll max must share.
pub(super) fn marked_row_text(base: String, mark: RowMark) -> String {
    match mark {
        RowMark::Pinned => format!("📌 {base}"),
        RowMark::SameName | RowMark::None => base,
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

/// Full gist file-list row as painted — [`gist_row_label`] plus `★ ` when the gist is starred.
/// Hscroll max must measure this string (or [`marked_row_text`] of it), not the star-less label.
pub(super) fn gist_row_display(g: &RankedGistFile, view: GistView, state: &AppState) -> String {
    let label = gist_row_label(g, view);
    if state.gist_is_starred(&g.file.gist_id) {
        format!("★ {label}")
    } else {
        label
    }
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
    // Build List body directly so Palette-over-List still paints (screen is Palette).
    let chrome = super::view_model::build_chrome(state);
    let list = super::view_model::build_list_vm(state);
    render_list_vm(frame, state, &list, &chrome, layout);
}

fn list_pane_items(
    pane: &super::view_model::ListPaneVm,
    hscroll: u16,
    theme: &Theme,
) -> Vec<ListItem<'static>> {
    match pane.empty {
        super::view_model::ListPaneEmpty::HasRows => pane
            .rows
            .iter()
            .map(|row| {
                let item = ListItem::new(hscroll_str(&row.label, hscroll));
                match row.mark {
                    RowMark::SameName => item.style(Style::default().add_modifier(Modifier::BOLD)),
                    RowMark::Pinned | RowMark::None => item,
                }
            })
            .collect(),
        _ => {
            let msg = pane.empty_message.clone().unwrap_or_else(|| "  ".into());
            vec![ListItem::new(msg).style(Style::default().fg(theme.dim))]
        }
    }
}

fn render_list_vm(
    frame: &mut Frame,
    state: &AppState,
    list: &super::view_model::ListVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    let footer_body = match &list.footer {
        super::view_model::ListFooterVm::Hints { text }
        | super::view_model::ListFooterVm::Status { text } => text.clone(),
        super::view_model::ListFooterVm::Filtering { focus } => {
            let (pane, query) = match focus {
                FocusPane::Local => ("local", &state.local_filter_query),
                FocusPane::Gist => ("gist", &state.filter_query),
            };
            // Height sizing uses a plain-text approximation; the painted footer uses `input_line`.
            format!("filter {pane}: {query}_   (Tab next pane · Enter apply · Esc clear)")
        }
    };
    let footer_is_command = matches!(list.footer, super::view_model::ListFooterVm::Hints { .. });
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

    let local_items = list_pane_items(&list.local, list.local_hscroll, &state.theme);
    let local_offset = render_pane(
        frame,
        columns[0],
        &list.local.title,
        local_items,
        list.local.focused,
        list.local.selected,
        &state.theme,
    );
    if chrome.mouse_enabled {
        layout.local = Some(PaneHit {
            rect: columns[0],
            offset: local_offset,
        });
    }

    let gist_items = list_pane_items(&list.gist, list.gist_hscroll, &state.theme);
    let gist_offset = render_pane(
        frame,
        columns[1],
        &list.gist.title,
        gist_items,
        list.gist.focused,
        list.gist.selected,
        &state.theme,
    );
    if chrome.mouse_enabled {
        layout.gist = Some(PaneHit {
            rect: columns[1],
            offset: gist_offset,
        });
    }

    match &list.footer {
        super::view_model::ListFooterVm::Filtering { focus } => {
            let (pane, query) = match focus {
                FocusPane::Local => ("local", &state.local_filter_query),
                FocusPane::Gist => ("gist", &state.filter_query),
            };
            let line = input_line(
                &format!("filter {pane}: "),
                query,
                "   (Tab next pane · Enter apply · Esc clear)",
            );
            render_footer_line(frame, chunks[1], "", line, &state.theme, layout);
        }
        _ => {
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

/// Render just the diff content pane (no footer) into `area` from a [`DiffVm`].
fn render_diff_pane_vm(
    frame: &mut Frame,
    area: Rect,
    diff: &super::view_model::DiffVm,
    theme: &Theme,
) {
    let block = Block::default()
        .title(diff.title.clone())
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
    let chrome = super::view_model::build_chrome(state);
    let diff = super::view_model::build_diff_vm(state);
    render_diff_vm(frame, state, &diff, &chrome, layout);
}

fn render_diff_vm(
    frame: &mut Frame,
    state: &AppState,
    diff: &super::view_model::DiffVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    let footer_lines = wrap_line_count(&diff.footer, area.width.saturating_sub(2)).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(footer_lines)])
        .split(area);

    render_diff_pane_vm(frame, chunks[0], diff, &state.theme);

    render_footer(
        frame,
        chunks[1],
        "",
        &diff.footer,
        true,
        &state.theme,
        layout,
    );
    if chrome.mouse_enabled {
        layout.close_button = Some(render_close_button(frame, area, &state.theme));
    }
}

/// `Screen::Confirm`: the diff fills the screen as context behind a centered prompt modal,
/// keeping the overwrite gate's diff visible while the question is asked front-and-centre.
/// #72 audit: this modal intentionally does not surface `state.status`. It is a transient y/n
/// gate — confirming executes the action and transitions to `List`/`Gists`, where the result
/// status is shown; cancelling returns to the launching screen without setting a status here.
fn render_confirm_vm(
    frame: &mut Frame,
    state: &AppState,
    confirm: &super::view_model::ConfirmVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    match &confirm.background {
        super::view_model::ConfirmBackgroundVm::CompactGist(bg) => {
            render_compact_gist_bg_vm(frame, frame.area(), bg, &state.theme);
        }
        super::view_model::ConfirmBackgroundVm::Diff => {
            let diff = super::view_model::build_diff_vm(state);
            render_diff_pane_vm(frame, frame.area(), &diff, &state.theme);
        }
        super::view_model::ConfirmBackgroundVm::Empty => {}
    }
    let modal = match &confirm.kind {
        super::view_model::ConfirmModalKind::DescriptionInput {
            prefix,
            value: _,
            suffix,
        } => {
            // Cursor-aware paint still uses live `TextInput` from state (same buffer the VM
            // snapshot was built from this frame).
            render_centered_modal_input(
                frame,
                confirm.title,
                prefix,
                &state.description_input,
                suffix,
                confirm.border,
                &state.theme,
            )
        }
        super::view_model::ConfirmModalKind::Prompt { text } => {
            render_centered_modal(frame, confirm.title, text, confirm.border, &state.theme)
        }
    };
    if chrome.mouse_enabled {
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
fn palette_row_spans(
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

fn render_palette_vm(
    frame: &mut Frame,
    state: &AppState,
    palette: &super::view_model::PaletteVm,
    chrome: &super::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    let mut bg_layout = MouseLayout::default();
    match palette.origin_screen {
        Screen::List => render_list(frame, state, &mut bg_layout),
        Screen::Diff => render_diff(frame, state, &mut bg_layout),
        Screen::Preview => render_preview(frame, state, &mut bg_layout),
        Screen::Help(_) => render_help(frame, state, &mut bg_layout),
        Screen::Pins(_) => render_pins(frame, state, &mut bg_layout),
        Screen::Gists(_) => render_gists(frame, state, &mut bg_layout),
        Screen::GistDetail(_) => render_gist_detail(frame, state, &mut bg_layout),
        Screen::Revisions(_) => render_revisions(frame, state, &mut bg_layout),
        Screen::Config(_) => render_config(frame, state, &mut bg_layout),
        Screen::Confirm | Screen::Palette(_) => {}
    }

    let area = frame.area();
    let body_lines = palette.items.len() + usize::from(palette.has_query);
    let longest_row = palette
        .items
        .iter()
        .map(|item| 2 + palette.key_width + 2 + item.label.chars().count());
    let content_width = longest_row.max().unwrap_or(20) as u16;
    let width = if palette.has_query {
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
    let (x, y) = match (palette.mode, palette.anchor) {
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
    if palette.has_query {
        let query = state.palette().map(|p| p.query.clone()).unwrap_or_default();
        lines.push(input_line("> ", &query, ""));
    }
    if palette.items.is_empty() {
        lines.push(Line::from(Span::styled("  (no matches)", dim)));
    } else {
        for (i, item) in palette.items.iter().enumerate() {
            let row_style = if i == palette.selected {
                active
            } else if item.enabled {
                state.theme.base_style()
            } else {
                Style::default().fg(state.theme.dim)
            };
            lines.push(palette_row_spans(
                &item.key_hint,
                &item.label,
                palette.key_width,
                &state.theme,
                row_style,
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(state.theme.base_style()).block(
            Block::default()
                .title(palette.title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.theme.accent))
                .style(state.theme.base_style()),
        ),
        rect,
    );

    let inner = rect.inner(Margin::new(1, 1));
    let mut y = inner.y + u16::from(palette.has_query);
    for item in palette.items.iter() {
        if y >= inner.bottom() {
            break;
        }
        if chrome.mouse_enabled && item.enabled {
            layout
                .palette_rows
                .push(Rect::new(inner.x, y, inner.width, 1));
        }
        y = y.saturating_add(1);
    }
    if chrome.mouse_enabled {
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
