//! `Screen::Help` — key handling, view-model, paint, and palette items colocated in one
//! file (issue #287, Phase 2).

use crate::tui::keys::{point_in, NavAction, PAGE_SCROLL};
use crate::tui::view_model::{ChromeVm, HelpIndexItemVm, HelpModeVm, HelpVm};
use crate::tui::{AppState, HelpState, HelpTopic, KeyOutcome, MouseLayout, PaneHit, Screen};
use crossterm::event::KeyCode;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Padding, Paragraph},
    Frame,
};

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::List;
const HELP_INDEX_TITLE: &str = "Help — pick a topic (1-9,g,0 / ↑↓ Enter · Esc back)";
const HELP_INDEX_SHORTCUTS: &[(char, HelpTopic)] = &[
    ('1', HelpTopic::List),
    ('2', HelpTopic::Pins),
    ('3', HelpTopic::GistManager),
    ('4', HelpTopic::GistDetail),
    ('5', HelpTopic::Revisions),
    ('6', HelpTopic::Diff),
    ('7', HelpTopic::Preview),
    ('8', HelpTopic::Upload),
    ('9', HelpTopic::Config),
    ('g', HelpTopic::General),
    ('0', HelpTopic::About),
];

fn help_index_shortcut(topic: HelpTopic) -> char {
    HELP_INDEX_SHORTCUTS
        .iter()
        .find_map(|&(key, mapped)| (mapped == topic).then_some(key))
        .expect("every help topic has an index shortcut")
}

fn help_topic_for_index_shortcut(key: char) -> Option<HelpTopic> {
    HELP_INDEX_SHORTCUTS
        .iter()
        .find_map(|&(mapped, topic)| (mapped == key).then_some(topic))
}

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

/// `wheel_step`'s Help-screen case: the topic body scrolls (3 lines/tick) but the topic
/// index is a plain list (1 row/tick).
pub(crate) fn wants_content_scroll(help: &HelpState) -> bool {
    !help.index_open
}

pub(crate) fn wheel_step(help: &HelpState) -> usize {
    if wants_content_scroll(help) {
        3
    } else {
        1
    }
}

impl AppState {
    pub(crate) fn handle_key_help(&mut self, code: KeyCode) -> KeyOutcome {
        let topics = HelpTopic::all();
        let leave = matches!(code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?'));
        {
            let Some(help) = self.help_mut() else {
                return KeyOutcome::None;
            };
            if help.index_open {
                match code {
                    KeyCode::Enter => {
                        help.topic = topics[help.index_sel];
                        help.index_open = false;
                        help.scroll = 0;
                    }
                    KeyCode::Char(key) if let Some(topic) = help_topic_for_index_shortcut(key) => {
                        help.topic = topic;
                        help.index_open = false;
                        help.scroll = 0;
                    }
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {}
                    _ => {}
                }
            } else {
                match code {
                    KeyCode::Tab => {
                        help.index_sel = topics.iter().position(|&t| t == help.topic).unwrap_or(0);
                        help.index_open = true;
                    }
                    KeyCode::Char(key) if let Some(topic) = help_topic_for_index_shortcut(key) => {
                        help.topic = topic;
                        help.scroll = 0;
                    }
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {}
                    _ => {}
                }
            }
        }
        if leave {
            self.leave();
        }
        KeyOutcome::None
    }

    /// Arrow / hjkl / page-key navigation for `Screen::Help`: moves the topic-index
    /// selection, or scrolls the topic body.
    pub(crate) fn apply_navigation_help(&mut self, action: NavAction) -> bool {
        let Screen::Help(help) = &mut self.screen else {
            return false;
        };
        let topics = HelpTopic::all();
        if help.index_open {
            match action {
                NavAction::Up => help.index_sel = help.index_sel.saturating_sub(1),
                NavAction::Down => {
                    if help.index_sel + 1 < topics.len() {
                        help.index_sel += 1;
                    }
                }
                _ => return false,
            }
        } else {
            match action {
                NavAction::Up => {
                    help.scroll = help.scroll.saturating_sub(1);
                }
                NavAction::Down => {
                    help.scroll = help.scroll.saturating_add(1);
                }
                NavAction::PageUp => {
                    help.scroll = help.scroll.saturating_sub(PAGE_SCROLL);
                }
                NavAction::PageDown => {
                    help.scroll = help.scroll.saturating_add(PAGE_SCROLL);
                }
                _ => return false,
            }
        }
        true
    }

    /// Select the clicked topic-index row on `Screen::Help`. Only set when the topic index is
    /// open (render_help), so this is a no-op while viewing a topic's body. Returns `true`
    /// when a row was hit.
    pub(crate) fn click_select_help(&mut self, col: u16, row: u16, layout: &MouseLayout) -> bool {
        let Screen::Help(help) = &mut self.screen else {
            return false;
        };
        if let Some(hit) = layout.list {
            if point_in(hit.rect, col, row) {
                if let Some(idx) = hit.index_at(row, HelpTopic::all().len()) {
                    help.index_sel = idx;
                    return true;
                }
            }
        }
        false
    }
}

/// Help body only — usable under Palette-over-Help as well.
pub(crate) fn build_help_vm(state: &AppState) -> HelpVm {
    let help = state.help().cloned().unwrap_or_default();
    let mode = if help.index_open {
        let items = HelpTopic::all()
            .iter()
            .map(|&topic| HelpIndexItemVm {
                key: help_index_shortcut(topic).to_string(),
                title: topic.title().to_string(),
            })
            .collect();
        HelpModeVm::Index {
            items,
            selected: help.index_sel,
        }
    } else {
        let title = format!(
            "Help · {} — Tab topics · ↑↓ scroll · Esc back",
            help.topic.title()
        );
        let (lines, about_repo_line) = if help.topic == HelpTopic::About {
            (about_topic_lines_plain(state), Some(ABOUT_REPO_LINE))
        } else {
            (
                help_topic_body(help.topic)
                    .lines()
                    .map(str::to_string)
                    .collect(),
                None,
            )
        };
        HelpModeVm::Topic {
            title,
            lines,
            scroll: help.scroll,
            about_repo_line,
        }
    };
    HelpVm { mode }
}

/// Fixed row (0-indexed, within the topic body) of the clickable repo-URL line — used to
/// place `MouseLayout::repo_link`'s hit-rect. Kept stable regardless of update-check state
/// (see `about_topic_lines`) so this constant never has to change.
pub(crate) const ABOUT_REPO_LINE: usize = 2;

/// Plain-text About topic lines for the pure view-model (issue #241). Paint re-applies the
/// underlined repo style from [`ABOUT_REPO_LINE`].
pub(crate) fn about_topic_lines_plain(state: &AppState) -> Vec<String> {
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

pub(crate) fn help_topic_body(topic: HelpTopic) -> &'static str {
    match topic {
        HelpTopic::List => {
            "\
Navigation
  Tab        switch pane (Local / Gists)
  1 / 2      jump to the Local / Gist pane
  Up/Down    move the selection (also j / k)
  Left/Right scroll the selected long row horizontally (also h / l)
  Ctrl+b/f   page up / down by 10 (also PageUp / PageDown)

List screen
  Footer     shows the primary screen actions; narrow terminals keep whole hints and the leave key
  r          toggle recursive file discovery (includes hidden paths; skips configured dirs)
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
  Top bar    click (g)ists / (P)ins / (C)onfig / (?)Help (top-right, every screen)
  Right-click  open the context menu at the click (same as ;)
  ; / Ctrl+p   open the menu / command palette from the keyboard (see General)"
        }
        HelpTopic::Pins => {
            r#"  Up/Down    move between pins (also j / k)
  PageUp/Dn  page by 10 (also Ctrl+b / Ctrl+f)
  Left/Right scroll the selected long local path horizontally (also h / l; ~ = home)
  /          filter pins by path or filename (↑↓ move · PgUp/PgDn page · Enter apply · Esc clear)
             ←/→/Home/End move the text cursor · Del deletes ahead
  o          cycle sort: default / local path / gist filename
  Enter      diff the selected pair (then d pull / u push from the diff)
  s          smart-sync (newer side wins; skips if already identical)
  u          force push  (upload local → gist)
  d          force pull  (download gist → local, diff + y/n confirm)
  x          unpin the selected pair
  Footer     shows sync actions; the status row explains the glyphs
  status     ✓ synced · ↑ local newer · ↓ remote newer · ✕ missing · ? unknown
  Each row shows (local <age> · gist <age>) relative modification times.
"#
        }
        HelpTopic::GistManager => {
            r#"  Footer     shows the primary screen actions; narrow terminals keep whole hints and the leave key
  Up/Down    move between gists (also j / k)
  PageUp/Dn  page by 10 (also Ctrl+b / Ctrl+f)
  Left/Right scroll the selected long description horizontally (also h / l)
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
  ★ prefix = you starred it; ⑂ prefix = this gist is a fork.
"#
        }
        HelpTopic::GistDetail => {
            "\
  Tab        switch tab: Files / Comments (Comments shows its total; opens on Files)
  Up/Down    move the file cursor (Files tab) or scroll comments (also j / k)
  PageUp/Dn  page comments / file cursor by 10 (also Ctrl+b / Ctrl+f)
  m          load 30 older comments (Comments tab; also click the top line)
  Enter      preview the cursor-selected file (file list focused; blocked for binary)
  1-9        preview the content of the Nth file (full-screen; R refresh, q back)
             rows show each file's size and type; non-text files are tagged binary
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
  Left/Right scroll the selected long row horizontally (also h / l)
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
  y          copy the gist URL to the clipboard
  Y          copy the file content to the clipboard
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
  Show full diff         on / off (opens Diff expanded; c still toggles it)
  Ignore trailing newline  on / off (diff + overwrite confirm)
  Recursive scan depth   0–20 (r recursive discovery)
  Diff context lines     0–50 (c in Diff still toggles full vs this radius)

File-only: skip_dirs — ~/.config/gistui/config.toml (or $XDG_CONFIG_HOME)"
        }
        HelpTopic::General => {
            "\
  Esc / q    close an overlay; from the list, press twice to quit the app
  ?          show this help
  Tab        in this help, open the topic index (1-9 / g / 0 jump straight to a topic)
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

pub(crate) fn render_help_vm(
    frame: &mut Frame,
    state: &AppState,
    help: &HelpVm,
    chrome: &ChromeVm,
    layout: &mut MouseLayout,
) {
    let area = frame.area();
    let area = crate::tui::render_top_bar(frame, area, &state.theme, chrome.mouse_enabled, layout);
    match &help.mode {
        HelpModeVm::Index { items, selected } => {
            let list_items: Vec<ListItem> = items
                .iter()
                .map(|item| ListItem::new(format!("  {:>2}  {}", item.key, item.title)))
                .collect();
            let list = List::new(list_items)
                .block(
                    Block::default()
                        .title(crate::tui::render::fit_block_title(
                            HELP_INDEX_TITLE,
                            area.width,
                        ))
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
                .highlight_symbol(crate::tui::render::list_pane::LIST_HIGHLIGHT_SYMBOL);
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
        HelpModeVm::Topic {
            title,
            lines,
            scroll,
            about_repo_line,
        } => {
            // Borders + horizontal padding: wrap to the inner width so a narrow
            // pane reflows instead of clipping mid-word (#342).
            let inner_width = area.width.saturating_sub(4) as usize;
            let body_lines: Vec<Line<'static>> = lines
                .iter()
                .enumerate()
                .flat_map(|(i, text)| {
                    let underline = about_repo_line == &Some(i);
                    crate::tui::render::wrap_hanging(text, inner_width)
                        .into_iter()
                        .map(move |part| {
                            if underline {
                                let repo = part.trim_start();
                                let indent_len = part.len() - repo.len();
                                let indent = part[..indent_len].to_string();
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
                                Line::from(part)
                            }
                        })
                })
                .collect();
            frame.render_widget(
                Paragraph::new(Text::from(body_lines))
                    .style(state.theme.base_style())
                    .scroll((*scroll, 0))
                    .block(
                        Block::default()
                            .title(crate::tui::render::fit_block_title(title, area.width))
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
        layout.close_button = Some(crate::tui::render_close_button(frame, area, &state.theme));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::*;

    use crate::tui::tests::{help_mut, help_ref, state_with_gists};

    #[test]
    fn footer_help_rows_align_with_the_key_column() {
        let pins = help_topic_body(HelpTopic::Pins);
        let legend = "✓ synced · ↑ local newer · ↓ remote newer · ✕ missing · ? unknown";
        assert_eq!(pins.matches(legend).count(), 1);

        let pin_lines: Vec<_> = pins.lines().collect();
        let footer = pin_lines
            .iter()
            .position(|line| {
                *line == "  Footer     shows sync actions; the status row explains the glyphs"
            })
            .expect("Pins footer");
        let status = pin_lines
            .iter()
            .position(|line| line.starts_with("  status"))
            .expect("Pins status");
        assert_eq!(footer + 1, status);

        assert!(help_topic_body(HelpTopic::GistManager).starts_with("  Footer"));
    }

    #[test]
    fn help_topic_view_tab_opens_index_at_current_topic() {
        let mut state = initial_state();
        state.screen = Screen::Help(Box::default());
        help_mut(&mut state).topic = HelpTopic::GistManager; // index 2
        state.handle_key(KeyCode::Tab);
        assert!(help_ref(&state).index_open);
        assert_eq!(help_ref(&state).index_sel, 2);
    }

    #[test]
    fn help_topic_view_number_switches_topic() {
        let mut state = initial_state();
        state.screen = Screen::Help(Box::default());
        help_mut(&mut state).topic = HelpTopic::List;
        help_mut(&mut state).scroll = 5;
        state.handle_key(KeyCode::Char('2')); // 2 -> Pins (index 1)
        assert_eq!(help_ref(&state).topic, HelpTopic::Pins);
        assert_eq!(help_ref(&state).scroll, 0);
        assert!(!help_ref(&state).index_open);
    }

    #[test]
    fn help_topic_view_zero_key_switches_to_about() {
        let mut state = initial_state();
        state.screen = Screen::Help(Box::default());
        help_mut(&mut state).topic = HelpTopic::List;
        help_mut(&mut state).scroll = 5;
        state.handle_key(KeyCode::Char('0')); // 0 -> About (index 10, the 11th topic)
        assert_eq!(help_ref(&state).topic, HelpTopic::About);
        assert_eq!(help_ref(&state).scroll, 0);
        assert!(!help_ref(&state).index_open);
    }

    #[test]
    fn help_index_zero_key_opens_about_from_the_index_list() {
        let mut state = initial_state();
        state.screen = Screen::Help(Box::default());
        help_mut(&mut state).index_open = true;
        state.handle_key(KeyCode::Char('0'));
        assert!(!help_ref(&state).index_open);
        assert_eq!(help_ref(&state).topic, HelpTopic::About);
    }

    #[test]
    fn each_help_index_shortcut_opens_its_displayed_topic() {
        let mut state = initial_state();
        state.screen = Screen::Help(Box::new(HelpState {
            index_open: true,
            ..HelpState::default()
        }));
        let HelpModeVm::Index { items, .. } = build_help_vm(&state).mode else {
            panic!("expected help topic index");
        };
        assert_eq!(items.len(), HelpTopic::all().len());
        assert!(HELP_INDEX_TITLE.contains("1-9,g,0"));

        for (item, topic) in items.iter().zip(HelpTopic::all()) {
            assert_eq!(item.key.chars().count(), 1, "{}", item.title);
            let key = item
                .key
                .chars()
                .next()
                .expect("every index row has a shortcut");
            state.screen = Screen::Help(Box::new(HelpState {
                index_open: true,
                ..HelpState::default()
            }));
            state.handle_key(KeyCode::Char(key));
            assert_eq!(help_ref(&state).topic, topic, "{}", item.title);
            assert!(!help_ref(&state).index_open, "{}", item.title);
        }
    }

    #[test]
    fn help_index_navigation_and_enter_open_every_topic() {
        for (index, topic) in HelpTopic::all().into_iter().enumerate() {
            let mut state = initial_state();
            state.screen = Screen::Help(Box::default());
            help_mut(&mut state).index_open = true;

            for _ in 0..index {
                state.handle_key(KeyCode::Down);
            }
            assert_eq!(help_ref(&state).index_sel, index);
            state.handle_key(KeyCode::Enter);
            assert!(!help_ref(&state).index_open);
            assert_eq!(help_ref(&state).topic, topic);
        }
    }

    #[test]
    fn close_button_click_outside_is_noop() {
        let mut state = state_with_gists();
        state.screen = Screen::Help(Box::default());
        let layout = MouseLayout {
            close_button: Some(Rect::new(36, 0, 5, 1)),
            ..Default::default()
        };
        // col 35 is just outside the left edge of the close button
        let out = state.handle_mouse(MouseInput::Click { col: 35, row: 0 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert!(state.screen.is_help());
    }

    #[test]
    fn click_off_list_screen_is_noop() {
        let mut state = state_with_gists();
        state.screen = Screen::Help(Box::default());
        let hit = PaneHit {
            rect: Rect::new(20, 0, 20, 10),
            offset: 0,
        };
        let layout = MouseLayout {
            gist: Some(hit),
            ..Default::default()
        };
        let before_screen = state.screen.clone();
        let out = state.handle_mouse(MouseInput::Click { col: 25, row: 1 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(state.screen, before_screen);
    }

    #[test]
    fn wheel_step_help_body_moves_three() {
        // Help body (help_index_open = false): one scroll-down tick must advance help_scroll by 3.
        let mut state = initial_state();
        state.screen = Screen::Help(Box::default());
        help_mut(&mut state).index_open = false;
        help_mut(&mut state).scroll = 0;
        state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
        assert_eq!(help_ref(&state).scroll, 3);
    }

    #[test]
    fn wheel_step_help_index_moves_one() {
        // Help topic index (help_index_open = true): one scroll-down tick must move index by 1.
        let mut state = initial_state();
        state.screen = Screen::Help(Box::default());
        help_mut(&mut state).index_open = true;
        help_mut(&mut state).index_sel = 0;
        state.handle_mouse(MouseInput::ScrollDown, &MouseLayout::default());
        assert_eq!(help_ref(&state).index_sel, 1);
    }

    #[test]
    fn help_index_click_selects_and_double_click_opens_topic() {
        let mut state = initial_state();
        state.screen = Screen::Help(Box::default());
        help_mut(&mut state).index_open = true;
        let hit = PaneHit {
            rect: Rect::new(0, 0, 40, 15),
            offset: 0,
        };
        let layout = MouseLayout {
            list: Some(hit),
            ..Default::default()
        };
        // Row 2 is the 2nd content row (border at row 0) -> idx 1 (Pins).
        let out = state.handle_mouse(MouseInput::Click { col: 5, row: 2 }, &layout);
        assert_eq!(out, KeyOutcome::None);
        assert_eq!(help_ref(&state).index_sel, 1);
        assert!(help_ref(&state).index_open); // a single click only selects, it doesn't open yet

        let by_mouse = state.handle_mouse(MouseInput::DoubleClick { col: 5, row: 2 }, &layout);
        assert_eq!(by_mouse, KeyOutcome::None);
        assert!(!help_ref(&state).index_open);
        assert_eq!(help_ref(&state).topic, HelpTopic::Pins);
    }
}
