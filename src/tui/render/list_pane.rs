//! One bordered list pane: rows, horizontal scroll, truncation, empty state, title, and the
//! mouse hit target it records (issue #367).
//!
//! Every screen that paints a list of selectable rows goes through [`render_list_pane`] — the
//! two List panes, Gist manager, Pinned Mappings, and Revisions. The row geometry below is the
//! implementation, not the interface: callers describe the pane with a [`ListPaneVm`] and never
//! assemble the widget themselves, so a change to clipping or scrolling lands in one place.
//!
//! Settings and Help's topic index deliberately stay out — their chrome differs (a different
//! highlight symbol, a bottom title, untruncated rows) and two callers would not justify the
//! seam.

use super::{cell_width, fit_title, truncate_end, ELLIPSIS};
use crate::tui::text::hscroll_str;
use crate::tui::theme::Theme;
use crate::tui::view_model::{ListPaneEmpty, ListPaneVm, RowEmphasis};
use crate::tui::{MouseFrame, PaneHit, PaneTarget};
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{
    Block, BorderType, Borders, List, ListItem, ListState, Padding, Scrollbar,
    ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

/// Highlight prefix every row list paints (`▶` plus a space). Kept in one place so the
/// truncation budget and the widget's `highlight_symbol` cannot drift. Public only because
/// Help's topic index paints its own list with the same prefix.
pub(crate) const LIST_HIGHLIGHT_SYMBOL: &str = "▶ ";

/// Borders (2) + `Padding::horizontal(1)` (2) around a list pane's inner rows.
const LIST_CHROME_CELLS: u16 = 4;

/// Narrowest a list pane may be made without turning into a slit: [`LIST_CHROME_CELLS`], the
/// two cells [`LIST_HIGHLIGHT_SYMBOL`] occupies (a const cannot measure its display width, so
/// the pair is asserted in the tests below), and eight cells of filename. It lives beside the
/// row budget it is built from; the List screen's divider drag (issue #395) clamps against it.
pub(crate) const MIN_PANE_CELLS: u16 = LIST_CHROME_CELLS + 2 + 8;

/// Paint one bordered list pane into `area` and, when the mouse is on, record where it landed.
///
/// `pane.focused` drives both the border colour and the selection highlight; `pane.hscroll`
/// moves the selected row only; `pane.scrollbar` is painted when the rows overflow.
pub(crate) fn render_list_pane(
    frame: &mut Frame,
    area: Rect,
    pane: &ListPaneVm,
    theme: &Theme,
    mouse_enabled: bool,
    layout: &mut MouseFrame,
    target: PaneTarget,
) {
    let items = pane_items(pane, area.width, theme);
    let item_count = items.len();

    let border_style = if pane.focused {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.dim)
    };
    // The border colour alone signals which pane is active; row text stays at full brightness
    // in both panes so it is always legible. Focused selection is a solid bar (whole row);
    // unfocused just bolds the row.
    let highlight_style = if pane.focused {
        Style::default()
            .bg(theme.accent)
            .fg(theme.fg_on_accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
    };

    // Titles sit between the two border corners; segments that do not fit are dropped here
    // rather than clipped mid-word by the block (#338).
    let title = fit_title(&pane.title, area.width.saturating_sub(2) as usize);
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
        .highlight_symbol(LIST_HIGHLIGHT_SYMBOL);

    let mut list_state = ListState::default();
    list_state.select(pane.selected);
    frame.render_stateful_widget(list, area, &mut list_state);

    // Show a scrollbar when the list overflows its viewport.
    if pane.scrollbar {
        let viewport = area.height.saturating_sub(2) as usize;
        if viewport > 0 && item_count > viewport {
            let mut scrollbar_state =
                ScrollbarState::new(item_count).position(pane.selected.unwrap_or(0));
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

    if mouse_enabled {
        layout.register_pane(
            target,
            PaneHit {
                rect: area,
                offset: list_state.offset(),
            },
            item_count,
        );
    }
}

/// Recolour the two adjoining border columns that separate a pair of side-by-side panes,
/// keeping the corners and title junctions the blocks already painted. The List screen uses
/// it to show that its divider is being dragged (issue #395) — a highlight that belongs to
/// neither pane, so it cannot ride on `ListPaneVm::focused`.
pub(crate) fn highlight_pane_divider(frame: &mut Frame, area: Rect, divider_x: u16, color: Color) {
    let buffer = frame.buffer_mut();
    for x in [divider_x, divider_x + 1] {
        for y in area.y..area.bottom() {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_fg(color);
            }
        }
    }
}

/// Rows as widget items, or the single dim line an empty pane shows instead.
fn pane_items(pane: &ListPaneVm, pane_width: u16, theme: &Theme) -> Vec<ListItem<'static>> {
    if pane.empty != ListPaneEmpty::HasRows {
        let message = pane.empty_message.clone().unwrap_or_else(|| "  ".into());
        return vec![ListItem::new(message).style(Style::default().fg(theme.dim))];
    }
    pane.rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let item = ListItem::new(visible_list_row(
                &row.label,
                row_hscroll(pane.selected, i, pane.hscroll),
                pane_width,
            ));
            match row.emphasis {
                RowEmphasis::None => item,
                RowEmphasis::Strong => item.style(Style::default().add_modifier(Modifier::BOLD)),
                RowEmphasis::Danger => item.style(Style::default().fg(theme.del_color)),
            }
        })
        .collect()
}

/// Horizontal offset applied to one list row: only the selected row moves (issue #341).
fn row_hscroll(selected: Option<usize>, index: usize, hscroll: u16) -> u16 {
    if selected == Some(index) {
        hscroll
    } else {
        0
    }
}

/// Visible list-row text: horizontal scroll first, then truncate to the inner content
/// width (borders, padding, and the highlight symbol) so ratatui cannot silently clip.
/// A non-zero offset keeps a leading `…` so the skip is visible (issue #341).
fn visible_list_row(label: &str, hscroll: u16, pane_width: u16) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeChoice;
    use crate::tui::render::tests::{buffer_text, render_state, render_state_size};
    use crate::tui::view_model::{PaneTitleVm, RowVm};
    use crate::tui::FocusPane;
    use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

    fn list_row_pane_width(content_cells: u16) -> u16 {
        LIST_CHROME_CELLS + cell_width(LIST_HIGHLIGHT_SYMBOL) as u16 + content_cells
    }

    fn pane_of(rows: &[&str]) -> ListPaneVm {
        ListPaneVm {
            title: PaneTitleVm::new("Pane".to_string()),
            focused: true,
            selected: (!rows.is_empty()).then_some(0),
            empty: if rows.is_empty() {
                ListPaneEmpty::NoItems
            } else {
                ListPaneEmpty::HasRows
            },
            empty_message: rows.is_empty().then(|| "  nothing here".to_string()),
            rows: rows
                .iter()
                .map(|label| RowVm {
                    label: (*label).to_string(),
                    emphasis: RowEmphasis::None,
                })
                .collect(),
            hscroll: 0,
            scrollbar: false,
        }
    }

    /// `MIN_PANE_CELLS` adds the highlight symbol's display width as a literal, because a
    /// const cannot call `cell_width`. This is what keeps the two in step.
    #[test]
    fn min_pane_cells_matches_the_highlight_symbol_it_budgets_for() {
        assert_eq!(cell_width(LIST_HIGHLIGHT_SYMBOL), 2);
    }

    /// Paint one pane on its own backend — the module's interface is the test surface.
    fn paint(pane: &ListPaneVm, width: u16, height: u16, mouse: bool) -> (Buffer, Option<PaneHit>) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let theme = Theme::for_choice(ThemeChoice::Dark);
        let mut layout = MouseFrame::default();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_list_pane(
                    frame,
                    area,
                    pane,
                    &theme,
                    mouse,
                    &mut layout,
                    PaneTarget::List,
                );
            })
            .unwrap();
        (
            terminal.backend().buffer().clone(),
            layout.pane(PaneTarget::List),
        )
    }

    /// First cell of row `index`'s label: border, `Padding::horizontal(1)`, then the highlight
    /// symbol every row is indented by once any row is selected.
    fn row_label_cell(buffer: &Buffer, width: u16, index: u16) -> ratatui::buffer::Cell {
        let chrome = 2 + cell_width(LIST_HIGHLIGHT_SYMBOL) as u16;
        buffer.content()[((index + 1) * width + chrome) as usize].clone()
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

    /// Issue #341: the offset belongs to the selected row alone.
    #[test]
    fn row_hscroll_moves_only_the_selected_row() {
        assert_eq!(row_hscroll(Some(1), 1, 4), 4);
        assert_eq!(row_hscroll(Some(1), 0, 4), 0);
        assert_eq!(row_hscroll(None, 0, 4), 0);
    }

    /// Issue #340: a row wider than the pane ends in `…` rather than looking complete.
    #[test]
    fn render_list_pane_marks_a_clipped_row_with_an_ellipsis() {
        let width = list_row_pane_width(14);
        let (buffer, _) = paint(&pane_of(&["0123456789abcdefghij"]), width, 4, false);
        let text = buffer_text(&buffer);
        assert!(
            text.contains("0123456789abc…"),
            "clipped row lost its ellipsis: {text}"
        );
    }

    /// Issue #341: only the selected row scrolls; its siblings stay readable from their start.
    #[test]
    fn render_list_pane_scrolls_only_the_selected_row() {
        let mut pane = pane_of(&["alpha-keep", "beta-XYZmarker"]);
        pane.selected = Some(1);
        pane.hscroll = 5;
        let (buffer, _) = paint(&pane, list_row_pane_width(20), 5, false);
        let text = buffer_text(&buffer);
        assert!(text.contains("alpha-keep"), "unselected row moved: {text}");
        assert!(
            text.contains("…XYZmarker"),
            "selected row did not scroll behind an ellipsis: {text}"
        );
        assert!(
            !text.contains("beta-"),
            "selected row kept its head: {text}"
        );
    }

    /// An empty pane paints its prebuilt message in place of rows.
    #[test]
    fn render_list_pane_paints_the_empty_message_instead_of_rows() {
        let (buffer, _) = paint(&pane_of(&[]), 30, 4, false);
        let text = buffer_text(&buffer);
        assert!(
            text.contains("nothing here"),
            "empty message missing: {text}"
        );
    }

    /// Emphasis is resolved by the view-model builder; paint only looks it up.
    #[test]
    fn render_list_pane_applies_row_emphasis() {
        let theme = Theme::for_choice(ThemeChoice::Dark);
        let width = list_row_pane_width(20);
        let mut pane = pane_of(&["plain", "strong", "danger"]);
        pane.selected = None;
        pane.rows[1].emphasis = RowEmphasis::Strong;
        pane.rows[2].emphasis = RowEmphasis::Danger;
        let (buffer, _) = paint(&pane, width, 6, false);
        // Nothing is selected, so ratatui adds no highlight indent.
        let cell = |index: u16| buffer.content()[((index + 1) * width + 2) as usize].clone();
        assert!(!cell(0).style().add_modifier.contains(Modifier::BOLD));
        assert!(cell(1).style().add_modifier.contains(Modifier::BOLD));
        assert_eq!(cell(2).style().fg, Some(theme.del_color));
    }

    /// The pane records where it landed only when the mouse is on.
    #[test]
    fn render_list_pane_records_its_hit_target_only_when_the_mouse_is_on() {
        let pane = pane_of(&["row"]);
        let (_, hit) = paint(&pane, 30, 5, true);
        let hit = hit.expect("mouse on records a hit target");
        assert_eq!(hit.rect.width, 30);
        assert_eq!(hit.offset, 0);

        let (_, hit) = paint(&pane, 30, 5, false);
        assert!(hit.is_none(), "mouse off must not record a hit target");
    }

    /// An unfocused pane is signalled by its border, not by dimming its rows.
    #[test]
    fn render_list_pane_dims_the_border_of_an_unfocused_pane() {
        let theme = Theme::for_choice(ThemeChoice::Dark);
        let mut pane = pane_of(&["selected", "sibling"]);
        let (focused, _) = paint(&pane, 20, 5, false);
        pane.focused = false;
        let (unfocused, _) = paint(&pane, 20, 5, false);
        assert_eq!(focused.content()[0].style().fg, Some(theme.accent));
        assert_eq!(unfocused.content()[0].style().fg, Some(theme.dim));
        // Only the selection changes shape with focus (solid bar vs bold); an unselected
        // row keeps full brightness either way.
        assert_eq!(
            row_label_cell(&focused, 20, 1).style().fg,
            row_label_cell(&unfocused, 20, 1).style().fg,
            "row text brightness must not depend on focus"
        );
    }

    /// Issue #367: Revisions used to paint its rows with the raw `hscroll_str`, so neither
    /// #340 (ellipsis on a clipped row) nor #341 (scroll the selected row only) reached it.
    /// Routing it through this module applies both.
    #[test]
    fn revisions_rows_clip_with_an_ellipsis_and_scroll_only_the_selection() {
        use crate::domain::{GistRevision, GistRevisionChangeStatus};

        let revision = |user: &str| GistRevision {
            version: "abc1234def".into(),
            committed_at: "2026-06-10T00:00:00Z".into(),
            user: user.into(),
            change_status: GistRevisionChangeStatus {
                total: 1,
                additions: 1,
                deletions: 0,
            },
        };
        let mut state = crate::tui::test_support::state_with_gists();
        state.screen = crate::tui::Screen::Revisions(Box::default());
        let rev = state.revision_mut().expect("expected Screen::Revisions");
        rev.gist_id = Some("g1".into());
        rev.entries = Some(vec![
            revision("aaa-FIRSTROW-marker"),
            revision("bbb-SECONDROW-marker"),
        ]);
        rev.cursor.index = 1;
        rev.cursor.hscroll = 8;

        let text = render_state_size(&state, 40, 12);
        assert!(
            !text.contains("aaa-FIRSTROW-marker"),
            "unselected row must be clipped to the pane width: {text}"
        );
        assert!(text.contains('…'), "clipped row lost its ellipsis: {text}");
        assert!(
            text.contains("#1  "),
            "unselected row lost its start: {text}"
        );
        assert!(
            !text.contains("#2  "),
            "selected row should have scrolled past its head: {text}"
        );
    }

    /// Issue #341: scrolling the selected long path must not eat the start of its siblings.
    #[test]
    fn list_hscroll_leaves_unselected_rows_readable_from_their_start() {
        let mut state = crate::tui::test_support::state_with_local_paths(&[
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
}
