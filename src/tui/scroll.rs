//! Shared body text + scroll offsets for Diff, Confirm, and Preview.
//!
//! Vertical max is the last addressable line (`lines - 1`; empty → 0). Horizontal
//! max is [`hscroll_max_among`]. Wrap is not this type's job — callers zero
//! `hscroll` themselves when wrapping.

use super::text::hscroll_max_among;

/// Scrollable text body: content plus vertical and horizontal offsets.
#[allow(dead_code)] // Wired into Diff/Confirm/Preview in the following commit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScrollBody {
    pub text: String,
    pub scroll: u16,
    pub hscroll: u16,
}

#[allow(dead_code)] // Wired into Diff/Confirm/Preview in the following commit.
impl ScrollBody {
    /// Last addressable line index (empty body → 0).
    fn vscroll_max(&self) -> u16 {
        self.text
            .lines()
            .count()
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16
    }

    /// Move down one line, clamped at [`Self::vscroll_max`].
    pub fn down(&mut self) {
        let max = self.vscroll_max();
        if self.scroll < max {
            self.scroll += 1;
        }
    }

    /// Move up one line, saturating at 0.
    pub fn up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    /// Page down by `lines`, clamped at [`Self::vscroll_max`].
    pub fn page_down(&mut self, lines: u16) {
        let max = self.vscroll_max();
        self.scroll = self.scroll.saturating_add(lines).min(max);
    }

    /// Page up by `lines`, saturating at 0.
    pub fn page_up(&mut self, lines: u16) {
        self.scroll = self.scroll.saturating_sub(lines);
    }

    /// Move right one character, clamped at the longest line.
    pub fn right(&mut self) {
        let max = hscroll_max_among(self.text.lines());
        if self.hscroll < max {
            self.hscroll += 1;
        }
    }

    /// Move left one character, saturating at 0.
    pub fn left(&mut self) {
        self.hscroll = self.hscroll.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_body_down_and_right_stay_at_zero() {
        let mut body = ScrollBody::default();
        body.down();
        body.right();
        assert_eq!(body.scroll, 0);
        assert_eq!(body.hscroll, 0);
    }

    #[test]
    fn vertical_cap_is_last_line_index() {
        let mut body = ScrollBody {
            text: "l1\nl2\nl3".into(),
            ..ScrollBody::default()
        };
        body.down();
        body.down();
        body.down();
        assert_eq!(body.scroll, 2);
    }

    #[test]
    fn up_saturates_at_zero() {
        let mut body = ScrollBody {
            text: "l1\nl2\nl3".into(),
            scroll: 1,
            ..ScrollBody::default()
        };
        body.up();
        body.up();
        assert_eq!(body.scroll, 0);
    }

    #[test]
    fn page_down_and_page_up_clamp() {
        let mut body = ScrollBody {
            text: "l1\nl2\nl3".into(),
            ..ScrollBody::default()
        };
        body.page_down(10);
        assert_eq!(body.scroll, 2);
        body.page_up(10);
        assert_eq!(body.scroll, 0);
    }

    #[test]
    fn horizontal_cap_is_longest_line() {
        let mut body = ScrollBody {
            text: "abcd\nab".into(),
            ..ScrollBody::default()
        };
        for _ in 0..10 {
            body.right();
        }
        assert_eq!(body.hscroll, 3);
    }

    #[test]
    fn left_saturates_at_zero() {
        let mut body = ScrollBody {
            text: "abcd\nab".into(),
            hscroll: 1,
            ..ScrollBody::default()
        };
        body.left();
        body.left();
        assert_eq!(body.hscroll, 0);
    }
}
