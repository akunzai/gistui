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

    /// Whether the two sides hold the same content, so there is nothing to sync.
    pub fn identical(&self, a: &str, b: &str) -> bool {
        crate::diff::content_eq(a, b, self.ignore_trailing_newline)
    }

    /// The unified diff from `old` to `new`.
    pub fn diff(&self, old_label: &str, old: &str, new_label: &str, new: &str) -> String {
        crate::diff::unified_diff(old_label, old, new_label, new, self.ignore_trailing_newline)
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
