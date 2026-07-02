use similar::{ChangeTag, TextDiff};
use std::fmt::Write;

/// Strip at most one file-final `\n` so that a trailing-newline-only difference does not
/// register as a change. Intentionally removes a single terminator (not all of them), so a
/// genuine blank-line change at EOF (`"a\n"` vs `"a\n\n"`) is still surfaced.
fn strip_final_newline(s: &str) -> &str {
    s.strip_suffix('\n').unwrap_or(s)
}

/// Whether `a` and `b` hold the same content. When `ignore_trailing_newline` is set, an
/// optional file-final newline is disregarded, so `"}"` and `"}\n"` compare equal — this is
/// the predicate the overwrite-confirm gate uses to decide a download/upload is a no-op.
pub fn content_eq(a: &str, b: &str, ignore_trailing_newline: bool) -> bool {
    if ignore_trailing_newline {
        strip_final_newline(a) == strip_final_newline(b)
    } else {
        a == b
    }
}

pub fn unified_diff(
    old_label: &str,
    old: &str,
    new_label: &str,
    new: &str,
    ignore_trailing_newline: bool,
) -> String {
    let (old, new) = if ignore_trailing_newline {
        (strip_final_newline(old), strip_final_newline(new))
    } else {
        (old, new)
    };
    let diff = TextDiff::from_lines(old, new);
    let mut out = format!("--- {old_label}\n+++ {new_label}\n");

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        out.push_str(sign);
        out.push_str(change.value());
        if change.missing_newline() {
            out.push('\n');
        }
    }

    out
}

/// Collapse a full unified diff (as produced by [`unified_diff`]) so that at most `radius`
/// unchanged context lines surround each change; longer unchanged runs are replaced with a
/// single `@@ N unchanged line(s) hidden @@` marker. The `--- / +++` header lines are always
/// kept. Output stays in the same `' '/'+'/'-'` prefixed format `diff_view` parses, and the
/// marker starts with `@` so it renders as neutral text.
pub fn collapse_context(diff: &str, radius: usize) -> String {
    let lines: Vec<&str> = diff.lines().collect();
    let is_change = |l: &str| (l.starts_with('-') || l.starts_with('+')) && !is_header(l);

    let mut keep = vec![false; lines.len()];
    for (i, &line) in lines.iter().enumerate() {
        if is_header(line) {
            keep[i] = true;
        } else if is_change(line) {
            let lo = i.saturating_sub(radius);
            let hi = (i + radius + 1).min(lines.len());
            for slot in keep.iter_mut().take(hi).skip(lo) {
                *slot = true;
            }
        }
    }

    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        if keep[i] {
            out.push_str(lines[i]);
            out.push('\n');
            i += 1;
        } else {
            let start = i;
            while i < lines.len() && !keep[i] {
                i += 1;
            }
            let hidden = i - start;
            let plural = if hidden == 1 { "" } else { "s" };
            let _ = writeln!(out, "@@ {hidden} unchanged line{plural} hidden @@");
        }
    }
    out
}

fn is_header(line: &str) -> bool {
    line.starts_with("---") || line.starts_with("+++")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_shows_insert_and_delete() {
        let diff = unified_diff("gist", "a\nb\n", "local", "a\nc\n", false);
        assert!(diff.contains("-b\n"));
        assert!(diff.contains("+c\n"));
    }

    #[test]
    fn trailing_newline_only_difference_is_a_phantom_when_strict() {
        // Strict mode reproduces issue #149: a file-final newline shows as a -}/+} pair.
        let diff = unified_diff("gist", "x\n}\n", "local", "x\n}", false);
        assert!(diff.contains("-}\n"));
        assert!(diff.contains("+}"));
    }

    #[test]
    fn ignore_trailing_newline_suppresses_the_phantom() {
        // With the option on, the trailing-newline-only delta disappears entirely.
        let diff = unified_diff("gist", "x\n}\n", "local", "x\n}", true);
        assert!(!diff.contains("-}"));
        assert!(!diff.contains("+}"));
        assert!(diff.contains(" }"));
    }

    #[test]
    fn missing_newline_on_a_non_final_line_does_not_merge_into_the_next() {
        // Regression: `old`'s last line has no trailing newline, but `new` appends more
        // content after that same line, so it is no longer the diff's true final line.
        // Without compensating for `Change::missing_newline`, the delete and the following
        // insert/change were concatenated onto a single raw line (no `\n` between them).
        let old = "a\nreset-author = x";
        let new = "a\nreset-author = x\ncleanup = y\n";
        let diff = unified_diff("gist", old, "local", new, false);
        let lines: Vec<&str> = diff.lines().collect();
        assert!(lines.contains(&"-reset-author = x"));
        assert!(lines.contains(&"+reset-author = x"));
        assert!(lines.contains(&"+cleanup = y"));
        // Each change lands on its own line — none of them got glued together.
        assert!(!lines.iter().any(|l| l.contains("x+")));
    }

    #[test]
    fn ignore_trailing_newline_still_shows_real_changes() {
        let diff = unified_diff("gist", "a\nb\n", "local", "a\nc", true);
        assert!(diff.contains("-b"));
        assert!(diff.contains("+c"));
    }

    #[test]
    fn content_eq_respects_trailing_newline_setting() {
        assert!(content_eq("}\n", "}", true));
        assert!(!content_eq("}\n", "}", false));
        // Only one terminator is ignored: a genuine trailing blank line still differs.
        assert!(!content_eq("a\n\n", "a\n", true));
        // Real content changes never compare equal.
        assert!(!content_eq("a\n", "b\n", true));
    }

    #[test]
    fn collapse_context_keeps_changes_and_nearby_context() {
        // 6 equal lines, a change, 6 more equal lines; radius 2 keeps 2 each side.
        let old = "a\nb\nc\nd\ne\nf\nX\ng\nh\ni\nj\nk\nl\n";
        let new = "a\nb\nc\nd\ne\nf\nY\ng\nh\ni\nj\nk\nl\n";
        let collapsed = collapse_context(&unified_diff("old", old, "new", new, false), 2);
        let lines: Vec<&str> = collapsed.lines().collect();

        // The change is preserved.
        assert!(lines.contains(&"-X"));
        assert!(lines.contains(&"+Y"));
        // Two context lines on each side are kept; the far ones are hidden behind a marker.
        assert!(lines.contains(&" e"));
        assert!(lines.contains(&" f"));
        assert!(lines.contains(&" g"));
        assert!(lines.contains(&" h"));
        assert!(!lines.contains(&" a"));
        assert!(!lines.contains(&" l"));
        assert!(collapsed.contains("unchanged line"));
        // Headers always survive.
        assert!(lines.contains(&"--- old"));
        assert!(lines.contains(&"+++ new"));
    }

    #[test]
    fn collapse_context_with_large_radius_is_lossless() {
        let diff = unified_diff("g", "a\nb\nc\n", "l", "a\nx\nc\n", false);
        let collapsed = collapse_context(&diff, 100);
        assert!(!collapsed.contains("hidden"));
        assert!(collapsed.contains(" a"));
        assert!(collapsed.contains(" c"));
    }

    #[test]
    fn collapse_context_separates_disjoint_hunks() {
        // Two changes far apart: each keeps its own context window, the long equal runs
        // before/between/after them collapse to markers, and the two windows do not merge.
        let old = "a\nb\nc\nd\ne\nf\ng\nX\nh\ni\nj\nk\nl\nm\nn\no\np\nY\nq\nr\ns\nt\n";
        let new = "a\nb\nc\nd\ne\nf\ng\nX2\nh\ni\nj\nk\nl\nm\nn\no\np\nY2\nq\nr\ns\nt\n";
        let collapsed = collapse_context(&unified_diff("g", old, "l", new, false), 2);
        let lines: Vec<&str> = collapsed.lines().collect();

        // Both hunks preserved.
        assert!(lines.contains(&"-X"));
        assert!(lines.contains(&"+X2"));
        assert!(lines.contains(&"-Y"));
        assert!(lines.contains(&"+Y2"));
        // Two context lines kept on each side of each change (the X hunk, then the Y hunk).
        assert!(lines.contains(&" g") && lines.contains(&" h"));
        assert!(lines.contains(&" p") && lines.contains(&" q"));
        // The equal run BETWEEN the two hunks is hidden — the windows did not merge.
        assert!(!lines.contains(&" k"));
        // Three collapsed runs: before X, between the hunks, and after Y.
        assert_eq!(collapsed.matches("unchanged line").count(), 3);
    }
}
