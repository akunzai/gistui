//! Shared list selection + horizontal scroll for every selectable list: the two List
//! panes, Pins, the Gists manager, and Revisions (issue #415).
//!
//! Bounds (`len`, `hmax`, page `step`) are computed by the caller with `&AppState`
//! *before* taking `&mut` on the payload, so navigation never fights the borrow
//! checker (issue #274). On the List screen that borrow is taken through
//! `AppState::focused_cursor_mut`, which is where `focus` picks a pane.
//!
//! Policy the module owns, and screens must not restate: a vertical move clears the
//! horizontal offset, because the offset belongs to the row it was scrolled on.
//! Anything a screen does *around* a cursor move — the List screen re-ranking its other
//! pane, or the local scan re-resolving a selection to the same file path — stays on
//! that screen; this module knows nothing about ranking, panes, or paths.

/// Index + horizontal scroll for a single-column list that resets hscroll when the
/// selection moves vertically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListCursor {
    pub index: usize,
    pub hscroll: u16,
}

impl ListCursor {
    /// Move selection down one row; no-op at the bottom. Resets hscroll on move.
    pub fn down(&mut self, len: usize) {
        if self.index + 1 < len {
            self.index += 1;
            self.hscroll = 0;
        }
    }

    /// Move selection up one row; no-op at the top. Resets hscroll on move.
    pub fn up(&mut self) {
        if self.index > 0 {
            self.index -= 1;
            self.hscroll = 0;
        }
    }

    /// Page selection down by `step` rows (clamped). Resets hscroll when `len > 0`.
    pub fn page_down(&mut self, len: usize, step: usize) {
        if len > 0 {
            let max = len - 1;
            self.index = (self.index + step).min(max);
            self.hscroll = 0;
        }
    }

    /// Page selection up by `step` rows (clamped at 0). Always resets hscroll.
    pub fn page_up(&mut self, step: usize) {
        self.index = self.index.saturating_sub(step);
        self.hscroll = 0;
    }

    /// Scroll the focused row right, clamped to `hmax`.
    pub fn right(&mut self, hmax: u16) {
        self.hscroll = (self.hscroll + 1).min(hmax);
    }

    /// Scroll the focused row left (floor 0).
    pub fn left(&mut self) {
        self.hscroll = self.hscroll.saturating_sub(1);
    }

    /// Jump to the first row and clear hscroll (filter edit, sort change, …).
    pub fn reset(&mut self) {
        self.index = 0;
        self.hscroll = 0;
    }

    /// Select a concrete row (e.g. mouse click) and clear hscroll.
    pub fn select(&mut self, index: usize) {
        self.index = index;
        self.hscroll = 0;
    }

    /// Clamp index into `0..len` after the visible list shrinks.
    /// Empty list → index 0. Does not touch hscroll.
    pub fn clamp_len(&mut self, len: usize) {
        if len == 0 {
            self.index = 0;
        } else {
            self.index = self.index.min(len - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn down_up_move_and_reset_hscroll() {
        let mut c = ListCursor {
            index: 0,
            hscroll: 3,
        };
        c.down(3);
        assert_eq!(
            c,
            ListCursor {
                index: 1,
                hscroll: 0
            }
        );
        c.hscroll = 2;
        c.down(3);
        assert_eq!(
            c,
            ListCursor {
                index: 2,
                hscroll: 0
            }
        );
        c.down(3); // already at bottom
        assert_eq!(
            c,
            ListCursor {
                index: 2,
                hscroll: 0
            }
        );
        c.hscroll = 5;
        c.up();
        assert_eq!(
            c,
            ListCursor {
                index: 1,
                hscroll: 0
            }
        );
        c.up();
        c.up(); // floor
        assert_eq!(
            c,
            ListCursor {
                index: 0,
                hscroll: 0
            }
        );
    }

    #[test]
    fn page_down_up_use_step_and_reset_hscroll() {
        let mut c = ListCursor {
            index: 0,
            hscroll: 4,
        };
        c.page_down(20, 10);
        assert_eq!(
            c,
            ListCursor {
                index: 10,
                hscroll: 0
            }
        );
        c.hscroll = 1;
        c.page_down(20, 10);
        assert_eq!(
            c,
            ListCursor {
                index: 19,
                hscroll: 0
            }
        ); // clamped
        c.hscroll = 2;
        c.page_up(10);
        assert_eq!(
            c,
            ListCursor {
                index: 9,
                hscroll: 0
            }
        );
        c.page_up(100);
        assert_eq!(
            c,
            ListCursor {
                index: 0,
                hscroll: 0
            }
        );
    }

    #[test]
    fn page_down_on_empty_list_is_noop() {
        let mut c = ListCursor {
            index: 5,
            hscroll: 2,
        };
        c.page_down(0, 10);
        assert_eq!(
            c,
            ListCursor {
                index: 5,
                hscroll: 2
            }
        );
    }

    #[test]
    fn left_right_clamp_hscroll() {
        let mut c = ListCursor::default();
        c.right(2);
        c.right(2);
        c.right(2); // clamp
        assert_eq!(c.hscroll, 2);
        c.left();
        assert_eq!(c.hscroll, 1);
        c.left();
        c.left();
        assert_eq!(c.hscroll, 0);
    }

    #[test]
    fn reset_and_select_clear_hscroll() {
        let mut c = ListCursor {
            index: 4,
            hscroll: 7,
        };
        c.reset();
        assert_eq!(c, ListCursor::default());
        c.select(3);
        assert_eq!(
            c,
            ListCursor {
                index: 3,
                hscroll: 0
            }
        );
    }

    #[test]
    fn clamp_len_empty_and_shrink() {
        let mut c = ListCursor {
            index: 9,
            hscroll: 3,
        };
        c.clamp_len(5);
        assert_eq!(
            c,
            ListCursor {
                index: 4,
                hscroll: 3
            },
            "hscroll preserved on clamp"
        );
        c.clamp_len(0);
        assert_eq!(
            c,
            ListCursor {
                index: 0,
                hscroll: 3
            }
        );
        c.index = 0;
        c.clamp_len(10);
        assert_eq!(c.index, 0);
    }
}
