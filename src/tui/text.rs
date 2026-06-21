//! Pure display/format helpers shared by the state layer (`AppState`) and the render layer,
//! kept out of `render` so `AppState` logic does not depend on the presentation module.

use crate::domain::GistComment;
use std::path::Path;

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
