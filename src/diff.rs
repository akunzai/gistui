use similar::{ChangeTag, TextDiff};
use std::fmt::Write;

pub fn unified_diff(old_label: &str, old: &str, new_label: &str, new: &str) -> String {
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
        let diff = unified_diff("gist", "a\nb\n", "local", "a\nc\n");
        assert!(diff.contains("-b\n"));
        assert!(diff.contains("+c\n"));
    }

    #[test]
    fn collapse_context_keeps_changes_and_nearby_context() {
        // 6 equal lines, a change, 6 more equal lines; radius 2 keeps 2 each side.
        let old = "a\nb\nc\nd\ne\nf\nX\ng\nh\ni\nj\nk\nl\n";
        let new = "a\nb\nc\nd\ne\nf\nY\ng\nh\ni\nj\nk\nl\n";
        let collapsed = collapse_context(&unified_diff("old", old, "new", new), 2);
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
        let diff = unified_diff("g", "a\nb\nc\n", "l", "a\nx\nc\n");
        let collapsed = collapse_context(&diff, 100);
        assert!(!collapsed.contains("hidden"));
        assert!(collapsed.contains(" a"));
        assert!(collapsed.contains(" c"));
    }
}
