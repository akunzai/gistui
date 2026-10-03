//! Physical rows, hunk positioning and scroll limits for side-by-side Diff.
//! Pure computation: callers retain content, selection and scroll ownership.
use crate::merge::Row;
use crate::tui::keys::{NavAction, PAGE_SCROLL};

pub(crate) const NUMBER_WIDTH: usize = 5;
const GUTTER_WIDTH: usize = 1 + NUMBER_WIDTH + 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DiffViewport {
    pub dimensions: Dimensions,
    pub scroll: u16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Dimensions {
    pub panes: [u16; 2],
    pub height: u16,
}

impl Dimensions {
    /// Pane widths include their two borders. Both sides use the narrower content width.
    pub(crate) fn content_width(self) -> usize {
        self.panes
            .into_iter()
            .map(|w| usize::from(w).saturating_sub(2 + GUTTER_WIDTH))
            .min()
            .unwrap_or(0)
            .max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Position {
    pub scroll: u16,
    pub hscroll: u16,
    pub selected: usize,
}

pub(crate) struct Geometry {
    rows: Vec<Row>,
    wrap: bool,
}

impl Geometry {
    pub(crate) fn new(rows: &[Row], width: usize, wrap: bool) -> Self {
        Self {
            rows: aligned_rows(rows, width, wrap),
            wrap,
        }
    }

    pub(crate) fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub(crate) fn reveal(&self, selected: usize) -> u16 {
        self.rows
            .iter()
            .position(|r| r.hunk == Some(selected))
            .unwrap_or(0)
            .min(u16::MAX as usize) as u16
    }

    pub(crate) fn navigate(&self, action: NavAction, mut position: Position) -> Position {
        let max = self.rows.len().saturating_sub(1).min(u16::MAX as usize) as u16;
        match action {
            NavAction::Up => position.scroll = position.scroll.saturating_sub(1),
            NavAction::Down => position.scroll = position.scroll.saturating_add(1).min(max),
            NavAction::PageUp => position.scroll = position.scroll.saturating_sub(PAGE_SCROLL),
            NavAction::PageDown => {
                position.scroll = position.scroll.saturating_add(PAGE_SCROLL).min(max)
            }
            NavAction::Left => position.hscroll = position.hscroll.saturating_sub(1),
            NavAction::Right if !self.wrap => {
                let max = self
                    .rows
                    .iter()
                    .flat_map(|r| [&r.local, &r.gist])
                    .filter_map(|s| s.as_ref())
                    .map(|(_, text)| text.chars().count())
                    .max()
                    .unwrap_or(0)
                    .min(u16::MAX as usize) as u16;
                position.hscroll = position.hscroll.saturating_add(1).min(max);
            }
            _ => {}
        }
        if !matches!(action, NavAction::Left | NavAction::Right) {
            if let Some(hunk) = self
                .rows
                .iter()
                .skip(position.scroll as usize)
                .find_map(|r| r.hunk)
            {
                position.selected = hunk;
            }
        }
        position
    }
}

/// Physical rows shared by painting, scroll bounds, hunk navigation and resize feedback.
/// Padding the shorter side keeps both sides on the same source row when one wraps.
fn aligned_rows(rows: &[crate::merge::Row], width: usize, wrap: bool) -> Vec<crate::merge::Row> {
    let mut result = Vec::new();
    for row in rows {
        let values = [&row.local, &row.gist];
        let content = values.map(|value| {
            let text = if row.omitted > 0 {
                format!("@@ {} unchanged lines hidden @@", row.omitted)
            } else {
                value
                    .as_ref()
                    .map(|(_, s)| s.replace('\t', "    "))
                    .unwrap_or_default()
            };
            if wrap {
                crate::tui::render::wrap_hanging(&text, width.max(1))
            } else {
                vec![text]
            }
        });
        for line in 0..content[0].len().max(content[1].len()).max(1) {
            let side = |index: usize| {
                (values[index].is_some() || row.omitted > 0).then(|| {
                    (
                        if line == 0 {
                            values[index].as_ref().map(|(n, _)| *n).unwrap_or(0)
                        } else {
                            0
                        },
                        content[index].get(line).cloned().unwrap_or_default(),
                    )
                })
            };
            result.push(crate::merge::Row {
                local: side(0),
                gist: side(1),
                hunk: row.hunk,
                omitted: 0,
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(local: Option<&str>, gist: Option<&str>, hunk: Option<usize>) -> Row {
        Row {
            local: local.map(|s| (7, s.into())),
            gist: gist.map(|s| (9, s.into())),
            hunk,
            omitted: 0,
        }
    }

    #[test]
    fn dimensions_account_for_borders_gutter_odd_and_narrow_panes() {
        for (panes, expected) in [([40, 41], 29), ([41, 40], 29), ([11, 12], 1), ([0, 0], 1)] {
            assert_eq!(Dimensions { panes, height: 12 }.content_width(), expected);
        }
    }

    #[test]
    fn asymmetric_wrap_pads_the_shorter_side_and_preserves_source_and_hunk() {
        let geometry = Geometry::new(
            &[
                row(Some("abcdefghij"), Some("xy"), Some(2)),
                row(Some("next"), None, Some(3)),
            ],
            4,
            true,
        );
        let rows = geometry.rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].local, Some((7, "abcd".into())));
        assert_eq!(rows[0].gist, Some((9, "xy".into())));
        assert_eq!(rows[1].local, Some((0, "efgh".into())));
        assert_eq!(rows[1].gist, Some((0, "".into())));
        assert_eq!(rows[2].local, Some((0, "ij".into())));
        assert_eq!(rows[2].hunk, Some(2));
        assert_eq!(rows[3].gist, None);
        assert_eq!(geometry.reveal(3), 3);
        assert_eq!(geometry.reveal(99), 0);
    }

    #[test]
    fn tabs_and_hidden_context_are_displayed_on_both_sides() {
        let mut omitted = row(None, None, None);
        omitted.omitted = 17;
        let geometry = Geometry::new(&[row(Some("\tx"), Some("界"), None), omitted], 80, false);
        assert_eq!(geometry.rows()[0].local, Some((7, "    x".into())));
        for value in [&geometry.rows()[1].local, &geometry.rows()[1].gist] {
            assert_eq!(*value, Some((0, "@@ 17 unchanged lines hidden @@".into())));
        }
    }

    #[test]
    fn navigation_uses_ten_rows_last_row_bounds_and_the_next_hunk() {
        let rows: Vec<_> = (0..15)
            .map(|i| {
                row(
                    Some("x"),
                    None,
                    match i {
                        0 => Some(0),
                        5 => Some(1),
                        12 => Some(2),
                        _ => None,
                    },
                )
            })
            .collect();
        let geometry = Geometry::new(&rows, 20, false);
        let start = Position {
            scroll: 0,
            hscroll: 0,
            selected: 0,
        };
        assert_eq!(geometry.navigate(NavAction::Up, start), start);
        assert_eq!(geometry.navigate(NavAction::PageUp, start), start);
        let page = geometry.navigate(NavAction::PageDown, start);
        assert_eq!(page.scroll, 10);
        assert_eq!(page.selected, 2);
        let end = geometry.navigate(NavAction::PageDown, page);
        assert_eq!(end.scroll, 14);
        assert_eq!(end.selected, 2, "no following hunk retains selection");
        assert_eq!(geometry.navigate(NavAction::Down, end), end);
        let previous = geometry.navigate(NavAction::PageUp, end);
        assert_eq!(previous.scroll, 4);
        assert_eq!(previous.selected, 1);
        assert_eq!(geometry.navigate(NavAction::Up, previous).scroll, 3);
        assert_eq!(geometry.navigate(NavAction::Down, previous).scroll, 5);
    }

    #[test]
    fn horizontal_bounds_count_characters_and_wrapping_disables_right() {
        let rows = [row(Some("界a"), None, Some(0))];
        let start = Position {
            scroll: 0,
            hscroll: 0,
            selected: 9,
        };
        let geometry = Geometry::new(&rows, 1, false);
        let one = geometry.navigate(NavAction::Right, start);
        let two = geometry.navigate(NavAction::Right, one);
        assert_eq!(two.hscroll, 2);
        assert_eq!(
            two.selected, 9,
            "horizontal navigation never changes selection"
        );
        assert_eq!(geometry.navigate(NavAction::Right, two), two);
        assert_eq!(geometry.navigate(NavAction::Left, one), start);
        assert_eq!(geometry.navigate(NavAction::Left, start), start);
        assert_eq!(
            Geometry::new(&rows, 1, true).navigate(NavAction::Right, start),
            start
        );
    }

    #[test]
    fn empty_geometry_and_large_positions_keep_u16_limits() {
        let start = Position {
            scroll: 0,
            hscroll: 0,
            selected: 0,
        };
        let empty = Geometry::new(&[], 0, true);
        assert_eq!(empty.reveal(0), 0);
        assert_eq!(empty.navigate(NavAction::Down, start), start);
        let mut rows = vec![row(Some("x"), None, None); 65537];
        rows[65536].hunk = Some(1);
        let geometry = Geometry::new(&rows, 1, false);
        assert_eq!(geometry.reveal(1), u16::MAX);
        let end = Position {
            scroll: u16::MAX,
            ..start
        };
        assert_eq!(geometry.navigate(NavAction::Down, end).scroll, u16::MAX);
        assert_eq!(geometry.navigate(NavAction::PageDown, end).scroll, u16::MAX);
        let geometry = Geometry::new(&[row(Some(&"x".repeat(65536)), None, None)], 1, false);
        assert_eq!(
            geometry
                .navigate(
                    NavAction::Right,
                    Position {
                        hscroll: u16::MAX,
                        ..start
                    }
                )
                .hscroll,
            u16::MAX
        );
    }
}
