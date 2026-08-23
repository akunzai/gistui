//! Mouse hit registration, target resolution, and cross-frame gesture state.

use ratatui::layout::Rect;

/// A clickable list pane recorded after ratatui has chosen its scroll offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneHit {
    pub rect: Rect,
    pub offset: usize,
}

impl PaneHit {
    pub fn index_at(self, row: u16, visible_len: usize) -> Option<usize> {
        let top = self.rect.y + 1;
        let bottom = self.rect.bottom().saturating_sub(1);
        if row < top || row >= bottom {
            return None;
        }
        let idx = self.offset + (row - top) as usize;
        (idx < visible_len).then_some(idx)
    }
}

/// Rendered geometry for the List screen's draggable divider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitHit {
    pub area: Rect,
    pub divider_x: u16,
}

impl SplitHit {
    pub fn grabbed(self, col: u16, row: u16) -> bool {
        row >= self.area.y
            && row < self.area.bottom()
            && col + 1 >= self.divider_x
            && col <= self.divider_x + 2
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneTarget {
    Local,
    Gist,
    List,
    DetailFiles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowTarget {
    Pane {
        pane: PaneTarget,
        index: Option<usize>,
    },
    Palette(usize),
}

impl RowTarget {
    /// The row index for a hit on `PaneTarget::List`, or `None` for any other pane, a blank
    /// area, or a Palette row. Shared by every single-list-pane screen's `click_select_*`
    /// (Config, Help, Pins, Gists, Revisions; issue #408) — List and GistDetail have more than
    /// one named pane and match `Pane { pane, index }` directly instead.
    pub fn list_index(self) -> Option<usize> {
        match self {
            RowTarget::Pane {
                pane: PaneTarget::List,
                index,
            } => index,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTarget {
    PaletteClose,
    Close,
    Repo,
    TopGists,
    TopPins,
    TopConfig,
    TopHelp,
    DetailFilesTab,
    DetailCommentsTab,
    CommentsLoadOlder,
    Divider(SplitHit),
    Row(RowTarget),
}

#[derive(Debug, Clone, Copy)]
enum HitRegion {
    Rect(HitTarget, Rect),
    Pane(PaneTarget, PaneHit, usize),
}

/// Per-frame semantic mouse targets. Registration order never controls precedence.
#[derive(Debug, Default, Clone)]
pub struct MouseFrame {
    hits: Vec<HitRegion>,
    intercept_all: bool,
}

impl MouseFrame {
    pub fn clear(&mut self) {
        self.hits.clear();
        self.intercept_all = false;
    }

    pub fn intercept_all(&mut self) {
        self.intercept_all = true;
    }

    pub fn register(&mut self, target: HitTarget, rect: Rect) {
        self.hits.push(HitRegion::Rect(target, rect));
    }

    pub fn register_pane(&mut self, pane: PaneTarget, hit: PaneHit, len: usize) {
        self.hits.push(HitRegion::Pane(pane, hit, len));
    }

    pub fn pane(&self, target: PaneTarget) -> Option<PaneHit> {
        self.hits.iter().rev().find_map(|hit| match hit {
            HitRegion::Pane(pane, hit, _) if *pane == target => Some(*hit),
            _ => None,
        })
    }

    pub fn split(&self) -> Option<SplitHit> {
        self.hits.iter().rev().find_map(|hit| match hit {
            HitRegion::Rect(HitTarget::Divider(split), _) => Some(*split),
            _ => None,
        })
    }

    /// Resolve the semantic pane/index target at `(col, row)`, considering only
    /// `register_pane` hits — never `Divider` or other whole-body `Rect` hits, which
    /// register far wider than their visual grab zone and would otherwise mask every
    /// row underneath them (issue #408). Panes never overlap, so at most one can match.
    pub fn resolve_pane(&self, col: u16, row: u16) -> Option<RowTarget> {
        self.hits.iter().rev().find_map(|hit| match *hit {
            HitRegion::Pane(pane, hit, len) if point_in(hit.rect, col, row) => {
                Some(RowTarget::Pane {
                    pane,
                    index: hit.index_at(row, len),
                })
            }
            _ => None,
        })
    }

    pub fn resolve(&self, col: u16, row: u16) -> Option<HitTarget> {
        let mut found = self.hits.iter().filter_map(|hit| match *hit {
            HitRegion::Rect(target, rect) if point_in(rect, col, row) => Some(target),
            HitRegion::Pane(pane, hit, len) if point_in(hit.rect, col, row) => {
                Some(HitTarget::Row(RowTarget::Pane {
                    pane,
                    index: hit.index_at(row, len),
                }))
            }
            _ => None,
        });
        if self.intercept_all {
            return found.find(|target| {
                matches!(
                    target,
                    HitTarget::PaletteClose | HitTarget::Row(RowTarget::Palette(_))
                )
            });
        }
        found.min_by_key(priority)
    }
}

fn priority(target: &HitTarget) -> u8 {
    match target {
        HitTarget::PaletteClose => 0,
        HitTarget::Close => 1,
        HitTarget::Repo => 2,
        HitTarget::TopGists => 3,
        HitTarget::TopPins => 4,
        HitTarget::TopConfig => 5,
        HitTarget::TopHelp => 6,
        HitTarget::DetailFilesTab | HitTarget::DetailCommentsTab => 7,
        HitTarget::CommentsLoadOlder => 8,
        HitTarget::Divider(_) => 9,
        HitTarget::Row(_) => 10,
    }
}

pub fn point_in(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
}

pub const DOUBLE_CLICK_MS: u128 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressKind {
    Click,
    DoubleClick,
}

/// Gesture facts that outlive one rendered frame.
#[derive(Debug, Default, Clone)]
pub struct MouseSession {
    last_press: Option<(u16, u16)>,
    divider_drag: bool,
}

impl MouseSession {
    pub fn press(&mut self, col: u16, row: u16, elapsed_ms: u128) -> PressKind {
        let kind = if self.last_press == Some((col, row)) && elapsed_ms <= DOUBLE_CLICK_MS {
            PressKind::DoubleClick
        } else {
            PressKind::Click
        };
        self.last_press = Some((col, row));
        kind
    }

    pub fn begin_divider_drag(&mut self) {
        self.divider_drag = true;
    }

    pub fn is_dragging(&self) -> bool {
        self.divider_drag
    }

    pub fn interrupt(&mut self) {
        self.divider_drag = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_uses_fixed_priority() {
        let rect = Rect::new(0, 0, 5, 5);
        let mut frame = MouseFrame::default();
        frame.register(HitTarget::TopHelp, rect);
        frame.register(HitTarget::Close, rect);
        assert_eq!(frame.resolve(1, 1), Some(HitTarget::Close));
    }

    #[test]
    fn palette_row_keeps_source_identity() {
        let mut frame = MouseFrame::default();
        frame.intercept_all();
        frame.register(HitTarget::Row(RowTarget::Palette(3)), Rect::new(1, 2, 5, 1));
        assert_eq!(
            frame.resolve(2, 2),
            Some(HitTarget::Row(RowTarget::Palette(3)))
        );
    }

    #[test]
    fn resolve_pane_ignores_a_wide_divider_rect_covering_the_pane() {
        let mut frame = MouseFrame::default();
        // Mirrors the List screen: Divider registers over the whole body, wider than its
        // visual grab zone, while the pane occupies a narrower column within it.
        frame.register(
            HitTarget::Divider(SplitHit {
                area: Rect::new(0, 0, 40, 10),
                divider_x: 20,
            }),
            Rect::new(0, 0, 40, 10),
        );
        frame.register_pane(
            PaneTarget::Local,
            PaneHit {
                rect: Rect::new(0, 0, 20, 10),
                offset: 0,
            },
            3,
        );
        assert_eq!(
            frame.resolve_pane(2, 1),
            Some(RowTarget::Pane {
                pane: PaneTarget::Local,
                index: Some(0),
            })
        );
        // Off every pane, even though it is still inside the Divider's wide rect.
        assert_eq!(frame.resolve_pane(30, 1), None);
    }

    #[test]
    fn session_classifies_and_interrupts() {
        let mut session = MouseSession::default();
        assert_eq!(session.press(2, 3, u128::MAX), PressKind::Click);
        assert_eq!(session.press(2, 3, DOUBLE_CLICK_MS), PressKind::DoubleClick);
        session.begin_divider_drag();
        assert!(session.is_dragging());
        session.interrupt();
        assert!(!session.is_dragging());
    }
}
