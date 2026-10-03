//! In-memory, bidirectional hunk copies. Byte ranges always refer to the full buffers,
//! independent of context collapse and terminal wrapping.
use crate::sync_content::SyncPolicy;
use similar::{capture_diff_slices, Algorithm, DiffTag};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub local: Option<(usize, String)>,
    pub gist: Option<(usize, String)>,
    pub hunk: Option<usize>,
    pub omitted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Hunk {
    local: Range<usize>,
    gist: Range<usize>,
    rows: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Merge {
    pub local: String,
    pub gist: String,
    pub baseline_local: Option<String>,
    pub baseline_gist: String,
    pub selected: usize,
    policy: SyncPolicy,
    undo: Vec<(String, String)>,
    rows: Vec<Row>,
    hunks: Vec<Hunk>,
}

impl Merge {
    pub fn new(local: Option<String>, gist: String, policy: SyncPolicy) -> Self {
        let mut result = Self {
            local: local.clone().unwrap_or_default(),
            gist: gist.clone(),
            baseline_local: local,
            baseline_gist: gist,
            selected: 0,
            policy,
            undo: Vec::new(),
            rows: Vec::new(),
            hunks: Vec::new(),
        };
        result.recompute();
        result
    }

    pub fn local_dirty(&self) -> bool {
        self.local != self.baseline_local.as_deref().unwrap_or_default()
    }

    pub fn gist_dirty(&self) -> bool {
        self.gist != self.baseline_gist
    }

    pub fn dirty(&self) -> bool {
        self.local_dirty() || self.gist_dirty()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn hunk_count(&self) -> usize {
        self.hunks.len()
    }

    pub fn jump(&mut self, forward: bool) {
        if forward {
            self.selected = (self.selected + 1).min(self.hunks.len().saturating_sub(1));
        } else {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    pub fn stage(&mut self, to_gist: bool) -> bool {
        let Some(hunk) = self.hunks.get(self.selected) else {
            return false;
        };
        let (source, source_range, destination, destination_range) = if to_gist {
            (
                &self.local,
                hunk.local.clone(),
                &self.gist,
                hunk.gist.clone(),
            )
        } else {
            (
                &self.gist,
                hunk.gist.clone(),
                &self.local,
                hunk.local.clone(),
            )
        };
        let mut result = destination.clone();
        result.replace_range(destination_range, &source[source_range]);
        if crate::domain::ensure_text_size(result.len() as u64).is_err() || result == *destination {
            return false;
        }
        self.undo.push((self.local.clone(), self.gist.clone()));
        if to_gist {
            self.gist = result;
        } else {
            self.local = result;
        }
        self.recompute();
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some((local, gist)) = self.undo.pop() else {
            return false;
        };
        self.local = local;
        self.gist = gist;
        self.recompute();
        true
    }

    /// Clear undo after any successful write: undo must never reinstate unsaved snapshots
    /// of a side that has already been committed, especially after a partial save.
    pub fn saved(&mut self, local: Option<String>, gist: Option<String>) {
        if let Some(local) = local {
            self.local = local.clone();
            self.baseline_local = Some(local);
        }
        if let Some(gist) = gist {
            self.gist = gist.clone();
            self.baseline_gist = gist;
        }
        self.undo.clear();
        self.recompute();
    }

    /// Compare under a new policy. Bytes, baselines and undo are untouched; only the hunks
    /// (and so the selection bound) are recomputed.
    pub fn set_policy(&mut self, policy: SyncPolicy) {
        if self.policy != policy {
            self.policy = policy;
            self.recompute();
        }
    }

    pub fn policy(&self) -> SyncPolicy {
        self.policy
    }

    pub fn visible_rows(&self, radius: Option<usize>) -> Vec<Row> {
        let Some(radius) = radius else {
            return self.rows.clone();
        };
        if self.hunks.is_empty() {
            return self.rows.clone();
        }
        let mut keep = vec![false; self.rows.len()];
        for hunk in &self.hunks {
            let start = hunk.rows.start.saturating_sub(radius);
            let end = (hunk.rows.end + radius).min(keep.len());
            keep[start..end].fill(true);
        }
        let mut result = Vec::new();
        let mut index = 0;
        while index < self.rows.len() {
            if keep[index] {
                result.push(self.rows[index].clone());
                index += 1;
            } else {
                let start = index;
                while index < self.rows.len() && !keep[index] {
                    index += 1;
                }
                result.push(Row {
                    local: None,
                    gist: None,
                    hunk: None,
                    omitted: index - start,
                });
            }
        }
        result
    }

    fn recompute(&mut self) {
        self.rows.clear();
        self.hunks.clear();
        let local = lines(&self.local);
        let gist = lines(&self.gist);
        let keys = |lines: &[&str]| -> Vec<String> {
            lines
                .iter()
                .enumerate()
                .map(|(i, line)| self.policy.comparison_line(line, i + 1 == lines.len()))
                .collect()
        };
        let local_keys = keys(&local);
        let gist_keys = keys(&gist);
        let ops = capture_diff_slices(Algorithm::Myers, &local_keys, &gist_keys);
        let mut local_offset = 0;
        let mut gist_offset = 0;
        for op in ops {
            let lr = op.old_range();
            let gr = op.new_range();
            let changed = op.tag() != DiffTag::Equal;
            let start_row = self.rows.len();
            let hunk = changed.then_some(self.hunks.len());
            for i in 0..lr.len().max(gr.len()) {
                let side = |lines: &[&str], range: &Range<usize>| {
                    (i < range.len()).then(|| {
                        let index = range.start + i;
                        let text = trim_eol(lines[index]);
                        let eol_only = changed
                            && i < lr.len()
                            && i < gr.len()
                            && trim_eol(local[lr.start + i]) == trim_eol(gist[gr.start + i]);
                        let note = if changed && !lines[index].ends_with(['\r', '\n']) {
                            " [no newline]"
                        } else if eol_only {
                            if lines[index].ends_with("\r\n") {
                                " [CRLF]"
                            } else if lines[index].ends_with('\r') {
                                " [CR]"
                            } else {
                                " [LF]"
                            }
                        } else {
                            ""
                        };
                        (index + 1, format!("{text}{note}"))
                    })
                };
                self.rows.push(Row {
                    local: side(&local, &lr),
                    gist: side(&gist, &gr),
                    hunk,
                    omitted: 0,
                });
            }
            let local_len: usize = local[lr].iter().map(|l| l.len()).sum();
            let gist_len: usize = gist[gr].iter().map(|l| l.len()).sum();
            if changed {
                self.hunks.push(Hunk {
                    local: local_offset..local_offset + local_len,
                    gist: gist_offset..gist_offset + gist_len,
                    rows: start_row..self.rows.len(),
                });
            }
            local_offset += local_len;
            gist_offset += gist_len;
        }
        if self.policy.identical(&self.local, &self.gist) {
            self.hunks.clear();
            for row in &mut self.rows {
                row.hunk = None;
            }
        }
        self.selected = self.selected.min(self.hunks.len().saturating_sub(1));
    }
}

fn trim_eol(line: &str) -> &str {
    line.strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .or_else(|| line.strip_suffix('\r'))
        .unwrap_or(line)
}

fn lines(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\n' || bytes[i] == b'\r' {
            if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
            result.push(&text[start..=i]);
            start = i + 1;
        }
        i += 1;
    }
    if start < text.len() {
        result.push(&text[start..]);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn merge(local: &str, gist: &str) -> Merge {
        Merge::new(Some(local.into()), gist.into(), SyncPolicy::default())
    }

    #[test]
    fn opposite_directions_keep_unselected_hunks_and_undo_both() {
        let mut m = merge(
            "local\nsame\nold\nsame\nkeep local\n",
            "old\nsame\ngist\nsame\nkeep gist\n",
        );
        assert_eq!(m.hunk_count(), 3);
        m.stage(true);
        assert_eq!(m.gist, "local\nsame\ngist\nsame\nkeep gist\n");
        m.stage(false);
        assert_eq!(m.local, "local\nsame\ngist\nsame\nkeep local\n");
        assert!(m.local_dirty() && m.gist_dirty());
        m.undo();
        m.undo();
        assert!(!m.dirty());
    }

    #[test]
    fn collapsed_later_hunk_splices_absolute_utf8_ranges() {
        let middle = "unchanged\n".repeat(30);
        let mut m = merge(&format!("甲\n{middle}乙\n"), &format!("一\n{middle}二\n"));
        assert!(m.visible_rows(Some(1)).iter().any(|r| r.omitted > 0));
        m.jump(true);
        m.stage(true);
        assert_eq!(m.gist, format!("一\n{middle}乙\n"));
    }

    #[test]
    fn insertion_deletion_empty_and_eof_keep_exact_bytes() {
        for (local, gist) in [
            ("", "a\n"),
            ("a\n", ""),
            ("a", "a\n"),
            ("a\r\nb\r\n", "a\r\nx\r\n"),
            ("a\rb\r", "a\rx\r"),
        ] {
            for direction in [true, false] {
                let mut m = merge(local, gist);
                m.stage(direction);
                if direction {
                    assert_eq!(m.gist, local);
                } else {
                    assert_eq!(m.local, gist);
                }
            }
        }
    }

    #[test]
    fn policy_ignores_newlines_and_saved_side_cannot_be_undone() {
        let mut m = Merge::new(
            Some("a\r\n".into()),
            "a".into(),
            SyncPolicy {
                normalize_line_endings: true,
                ignore_trailing_newline: true,
            },
        );
        assert_eq!(m.hunk_count(), 0);
        assert!(!m.stage(true));
        m = merge("a\nsame\nb\n", "x\nsame\ny\n");
        m.stage(true);
        m.stage(false);
        m.saved(None, Some(m.gist.clone()));
        assert!(!m.gist_dirty());
        assert!(m.local_dirty());
        assert!(!m.undo());
    }
    #[test]
    fn ignored_empty_final_newline_has_no_hunk_and_eol_only_changes_are_labelled() {
        let policy = SyncPolicy {
            normalize_line_endings: true,
            ignore_trailing_newline: true,
        };
        let mut m = Merge::new(Some(String::new()), "\n".into(), policy);
        assert_eq!(m.hunk_count(), 0);
        assert!(!m.stage(false));
        m = merge("a\r\n", "a\n");
        let rows = m.visible_rows(None);
        assert!(rows[0].local.as_ref().unwrap().1.ends_with("[CRLF]"));
        assert!(rows[0].gist.as_ref().unwrap().1.ends_with("[LF]"));
    }
}
