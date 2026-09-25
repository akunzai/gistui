//! The **Sync policy** (issue #464): the content rules for syncing a local file with a gist
//! file. It decides which bytes an upload or create sends, which bytes a download writes,
//! when the two sides count as identical, and how the diff between them reads. The
//! `normalize_line_endings` and `ignore_trailing_newline` settings drive it.
//!
//! Callers ask the policy and never re-derive these rules themselves. A pin baseline is the
//! hash of the bytes on disk after a sync. For a download, that is exactly what
//! [`SyncPolicy::write_download`] returns. Restoring a revision copies gist to gist, so its
//! payload is not a sync and does not go through [`SyncPolicy::outbound`].

use std::borrow::Cow;
use std::path::Path;

use anyhow::Result;

use crate::actions::DownloadMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncPolicy {
    /// Rewrite CRLF / lone CR to LF in the bytes sent and written.
    pub normalize_line_endings: bool,
    /// Treat a file-final newline as insignificant when comparing and diffing.
    pub ignore_trailing_newline: bool,
}

impl SyncPolicy {
    /// The bytes an upload or create sends for `content`.
    pub fn outbound<'a>(&self, content: &'a str) -> Cow<'a, str> {
        self.line_endings(content)
    }

    /// The bytes a download writes for the gist's `content`.
    pub fn to_disk<'a>(&self, content: &'a str) -> Cow<'a, str> {
        self.line_endings(content)
    }

    fn line_endings<'a>(&self, content: &'a str) -> Cow<'a, str> {
        if self.normalize_line_endings {
            crate::diff::normalize_line_endings(content)
        } else {
            Cow::Borrowed(content)
        }
    }

    /// Whether the two sides hold the same content, so there is nothing to sync. Line endings
    /// count only when normalization is off: then they survive a sync, so a difference in
    /// them is something to sync (#465).
    pub fn identical(&self, a: &str, b: &str) -> bool {
        if self.normalize_line_endings {
            return crate::diff::content_eq(a, b, self.ignore_trailing_newline);
        }
        if self.ignore_trailing_newline {
            strip_final_line_ending(a) == strip_final_line_ending(b)
        } else {
            a == b
        }
    }

    /// The unified diff from `old` to `new`. Lines are always compared with line endings
    /// normalized, since a `\r` is invisible on screen. When normalization is off and the
    /// two sides use different line endings, a note under the header says so (#465).
    pub fn diff(&self, old_label: &str, old: &str, new_label: &str, new: &str) -> String {
        let diff =
            crate::diff::unified_diff(old_label, old, new_label, new, self.ignore_trailing_newline);
        let (old_eol, new_eol) = (line_ending_style(old), line_ending_style(new));
        if self.normalize_line_endings || old_eol == new_eol {
            return diff;
        }
        let note = format!("@@ line endings differ: --- {old_eol}, +++ {new_eol} @@\n");
        match diff.match_indices('\n').nth(1) {
            Some((i, _)) => format!("{}{note}{}", &diff[..=i], &diff[i + 1..]),
            None => diff + &note,
        }
    }

    /// The read-only preview diff. It is framed as an upload (gist → local) when
    /// `upload_orientation` is set, and as a download (local → gist) otherwise.
    pub fn preview_diff(
        &self,
        upload_orientation: bool,
        local_label: &str,
        local: &str,
        gist_label: &str,
        gist: &str,
    ) -> String {
        if upload_orientation {
            self.diff(gist_label, gist, local_label, local)
        } else {
            self.diff(local_label, local, gist_label, gist)
        }
    }

    /// Write the gist's `content` to `local_path` as [`Self::to_disk`] dictates. Returns the
    /// bytes written, which are also the pin baseline.
    pub fn write_download(
        &self,
        local_path: &Path,
        content: &str,
        mode: DownloadMode,
    ) -> Result<String> {
        let written = self.to_disk(content).into_owned();
        crate::actions::execute_download(local_path, &written, mode)?;
        Ok(written)
    }
}

/// Strip one file-final line ending (`\r\n`, `\n`, or `\r`).
fn strip_final_line_ending(s: &str) -> &str {
    s.strip_suffix("\r\n")
        .or_else(|| s.strip_suffix('\n'))
        .or_else(|| s.strip_suffix('\r'))
        .unwrap_or(s)
}

/// The line-ending style of `s`: `CRLF`, `LF`, `CR`, `mixed`, or `none` when it has no
/// line break at all.
fn line_ending_style(s: &str) -> &'static str {
    let crlf = s.matches("\r\n").count();
    let lf = s.matches('\n').count() - crlf;
    let cr = s.matches('\r').count() - crlf;
    match (crlf > 0, lf > 0, cr > 0) {
        (false, false, false) => "none",
        (true, false, false) => "CRLF",
        (false, true, false) => "LF",
        (false, false, true) => "CR",
        _ => "mixed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(normalize_line_endings: bool, ignore_trailing_newline: bool) -> SyncPolicy {
        SyncPolicy {
            normalize_line_endings,
            ignore_trailing_newline,
        }
    }

    #[test]
    fn outbound_and_to_disk_follow_the_setting() {
        assert_eq!(policy(true, false).outbound("a\r\nb\r"), "a\nb\n");
        assert_eq!(policy(true, false).to_disk("a\r\nb\r"), "a\nb\n");
        assert_eq!(policy(false, false).outbound("a\r\nb\r"), "a\r\nb\r");
        assert_eq!(policy(false, false).to_disk("a\r\nb\r"), "a\r\nb\r");
    }

    /// #465's table: {CRLF, LF} gist × {normalize on, off} → bytes sent and written,
    /// whether a CRLF-vs-LF pair is identical, and whether the diff carries the note.
    #[test]
    fn line_ending_table() {
        let (crlf, lf) = ("a\r\nb\r\n", "a\nb\n");
        struct Row {
            normalize: bool,
            gist: &'static str,
            sent_or_written: &'static str,
            identical_to_lf_local: bool,
            note: Option<&'static str>,
        }
        let rows = [
            Row {
                normalize: true,
                gist: crlf,
                sent_or_written: lf,
                identical_to_lf_local: true,
                note: None,
            },
            Row {
                normalize: true,
                gist: lf,
                sent_or_written: lf,
                identical_to_lf_local: true,
                note: None,
            },
            Row {
                normalize: false,
                gist: crlf,
                sent_or_written: crlf,
                identical_to_lf_local: false,
                note: Some("@@ line endings differ: --- CRLF, +++ LF @@\n"),
            },
            Row {
                normalize: false,
                gist: lf,
                sent_or_written: lf,
                identical_to_lf_local: true,
                note: None,
            },
        ];
        for row in rows {
            let p = policy(row.normalize, false);
            let ctx = format!("normalize={} gist={:?}", row.normalize, row.gist);
            assert_eq!(p.outbound(row.gist), row.sent_or_written, "outbound {ctx}");
            assert_eq!(p.to_disk(row.gist), row.sent_or_written, "to_disk {ctx}");
            assert_eq!(
                p.identical(row.gist, lf),
                row.identical_to_lf_local,
                "identical {ctx}"
            );
            let diff = p.diff("gist", row.gist, "local", lf);
            match row.note {
                Some(note) => assert_eq!(diff, format!("--- gist\n+++ local\n{note} a\n b\n")),
                None => assert!(!diff.contains("line endings differ"), "{ctx}: {diff}"),
            }
        }
    }

    #[test]
    fn identical_with_normalization_off_ignores_only_a_final_line_ending() {
        let p = policy(false, true);
        assert!(p.identical("a\r\nb\r\n", "a\r\nb"));
        assert!(!p.identical("a\r\nb\r\n", "a\nb\n"));
    }

    #[test]
    fn line_ending_note_survives_collapsed_context() {
        let old: String = (0..20).map(|i| format!("line {i}\r\n")).collect();
        let new: String = (0..20).map(|i| format!("line {i}\n")).collect();
        let diff = policy(false, false).diff("gist", &old, "local", &new);
        let collapsed = crate::diff::collapse_context(&diff, 3);
        assert!(collapsed
            .starts_with("--- gist\n+++ local\n@@ line endings differ: --- CRLF, +++ LF @@\n"));
    }

    #[test]
    fn identical_honours_ignore_trailing_newline() {
        assert!(policy(true, true).identical("a", "a\n"));
        assert!(!policy(true, false).identical("a", "a\n"));
    }

    #[test]
    fn preview_diff_flips_with_orientation() {
        let p = policy(true, false);
        let download = p.preview_diff(false, "local: a", "old\n", "gist b", "new\n");
        assert!(download.starts_with("--- local: a\n+++ gist b\n"));
        assert!(download.contains("-old\n") && download.contains("+new\n"));

        let upload = p.preview_diff(true, "local: a", "old\n", "gist b", "new\n");
        assert!(upload.starts_with("--- gist b\n+++ local: a\n"));
        assert!(upload.contains("-new\n") && upload.contains("+old\n"));
    }

    #[test]
    fn write_download_returns_the_bytes_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        for (normalize, expected) in [(true, "a\nb\n"), (false, "a\r\nb\r\n")] {
            let path = dir.path().join(format!("out-{normalize}.txt"));
            let written = policy(normalize, false)
                .write_download(&path, "a\r\nb\r\n", DownloadMode::CreateNew)
                .unwrap();
            assert_eq!(written, expected);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        }
    }
}
