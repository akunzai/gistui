//! Pure display/format helpers shared by the state layer (`AppState`) and the render layer,
//! kept out of `render` so `AppState` logic does not depend on the presentation module.
//!
//! # Horizontal scroll units (issue #247)
//!
//! List-row hscroll measures and advances in **Unicode scalar values** (`char`s), matching
//! [`hscroll_str`] which skips with `chars().skip`. That keeps max and paint on the same
//! unit. Display-column width (East Asian width / `unicode-width`) is intentionally **not**
//! used here: mixing column widths with char-based skip would reintroduce drift. A future
//! change may switch both measure and skip together.

use crate::domain::GistComment;
use std::path::Path;

/// Length of `s` in the units used by list horizontal scroll (Unicode scalars).
pub(super) fn text_len(s: &str) -> usize {
    s.chars().count()
}

/// Drop the first `offset` characters of `text` for horizontal scrolling.
pub(super) fn hscroll_str(text: &str, offset: u16) -> String {
    text.chars().skip(offset as usize).collect()
}

/// Highest scroll offset so the last character of a string of `len` can become the first
/// visible character (same formula used across list / gists / pins / diff panes).
pub(super) fn hscroll_max_for_len(len: usize) -> u16 {
    len.saturating_sub(1).min(u16::MAX as usize) as u16
}

/// [`hscroll_max_for_len`] for a concrete display string.
pub(super) fn hscroll_max_for_text(text: &str) -> u16 {
    hscroll_max_for_len(text_len(text))
}

/// Max hscroll over many display strings (empty iterator → 0).
pub(super) fn hscroll_max_among<I, S>(texts: I) -> u16
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    // Monotonic in length: max of per-string caps == cap of the longest string.
    texts
        .into_iter()
        .map(|s| hscroll_max_for_text(s.as_ref()))
        .max()
        .unwrap_or(0)
}

/// A local file path shortened relative to `cwd` for list-row display.
pub(super) fn local_row_label(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd).unwrap_or(path).display().to_string()
}

/// Logical line count of the rendered comment block — must mirror `render::comment_lines`
/// (1 author header + body lines + 1 blank per comment). The amount to bump the comment
/// scroll by when older comments are prepended.
pub(super) fn comment_lines_count(comments: &[GistComment]) -> u16 {
    comments
        .iter()
        .map(|c| 2 + c.body.lines().count())
        .sum::<usize>()
        .min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_len_counts_scalars_not_bytes() {
        assert_eq!(text_len(""), 0);
        assert_eq!(text_len("ab"), 2);
        assert_eq!(text_len("★ f"), 3);
        assert_eq!(text_len("↔ x"), 3);
    }

    #[test]
    fn hscroll_str_skips_scalars() {
        assert_eq!(hscroll_str("★ long-name", 0), "★ long-name");
        assert_eq!(hscroll_str("★ long-name", 2), "long-name");
        assert_eq!(hscroll_str("★ long-name", 100), "");
    }

    #[test]
    fn hscroll_max_allows_last_char_first() {
        assert_eq!(hscroll_max_for_text(""), 0);
        assert_eq!(hscroll_max_for_text("a"), 0);
        assert_eq!(hscroll_max_for_text("ab"), 1);
        // Star prefix is two scalars ("★" + space); max must include them.
        assert_eq!(hscroll_max_for_text("★ ab"), 3);
    }

    #[test]
    fn hscroll_max_among_picks_longest() {
        assert_eq!(hscroll_max_among(std::iter::empty::<&str>()), 0);
        assert_eq!(
            hscroll_max_among(["a", "★★ longer", "bb"]),
            hscroll_max_for_text("★★ longer")
        );
    }
}
