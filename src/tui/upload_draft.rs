//! The **Upload draft** (issue #461): one upload awaiting confirmation — its target (local
//! file ↔ gist file) and its pending content (the local file as read, any redact edit, the
//! JSON pretty/sort toggles, and the editor-watch flag).
//!
//! It lives on Confirm as `PendingAction::Upload`, so it is discarded with Confirm on cancel.
//! When the upload is confirmed, [`UploadDraft::content`] fixes the exact bytes that are sent,
//! and the upload job carries those bytes itself. Nothing reads Confirm again after that
//! (#460).

use std::path::PathBuf;

use super::RuntimeSettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadDraft {
    pub gist_id: String,
    pub filename: String,
    pub local_path: PathBuf,
    pub original_content: String,
    pub edited_content: Option<String>,
    pub json_pretty: bool,
    pub json_sort: bool,
    /// The gist side of the diff (empty for a file new to the gist).
    pub remote_content: String,
    pub local_label: String,
    pub gist_label: String,
    /// True while a GUI-editor background watch (see `bg::spawn_upload_edit_watch`) is
    /// live-updating the diff. Gates `y`/`e` on Confirm: the upload can't be confirmed, and a
    /// second editor instance can't be spawned, until the editor closes.
    pub watching: bool,
}

impl UploadDraft {
    /// Read the local file into a fresh draft. Returns the read error instead of defaulting
    /// to empty content: an unreadable, deleted, or non-UTF-8 file would otherwise render the
    /// whole gist as additions, so the caller must surface it and abort the upload.
    pub fn read(
        gist_id: String,
        filename: String,
        local_path: PathBuf,
        remote_content: String,
        local_label: String,
        gist_label: String,
    ) -> std::io::Result<Self> {
        // Cap before buffering: multi-GB locals must not be read into the redact buffer.
        crate::domain::ensure_text_size(remote_content.len() as u64)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let original_content = crate::domain::read_text_file_capped(&local_path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Self {
            gist_id,
            filename,
            local_path,
            original_content,
            edited_content: None,
            json_pretty: false,
            json_sort: false,
            remote_content,
            local_label,
            gist_label,
            watching: false,
        })
    }

    /// Whether `p` / `s` (JSON pretty / sort) apply to this upload.
    pub fn is_json(&self) -> bool {
        super::AppState::is_json_file(&self.local_path)
    }

    /// The bytes this upload sends: the redact edit if any, else the file as read, then the
    /// JSON toggles (JSON files only, left as-is if the text doesn't parse), then line-ending
    /// normalization when the setting is on.
    pub fn content(&self, settings: &RuntimeSettings) -> String {
        let base = self
            .edited_content
            .as_ref()
            .unwrap_or(&self.original_content);
        let content = if self.is_json() {
            crate::domain::transform_json(base, self.json_pretty, self.json_sort)
                .unwrap_or_else(|_| base.clone())
        } else {
            base.clone()
        };
        if settings.normalize_line_endings() {
            crate::diff::normalize_line_endings(&content).into_owned()
        } else {
            content
        }
    }

    /// The gist-vs-upload unified diff Confirm shows.
    pub fn diff(&self, settings: &RuntimeSettings) -> String {
        crate::diff::unified_diff(
            &self.gist_label,
            &self.remote_content,
            &self.local_label,
            &self.content(settings),
            settings.ignore_trailing_newline(),
        )
    }

    #[cfg(test)]
    pub fn fixture(gist_id: &str, filename: &str, local_path: impl AsRef<std::path::Path>) -> Self {
        Self {
            gist_id: gist_id.into(),
            filename: filename.into(),
            local_path: local_path.as_ref().to_path_buf(),
            original_content: String::new(),
            edited_content: None,
            json_pretty: false,
            json_sort: false,
            remote_content: String::new(),
            local_label: String::new(),
            gist_label: String::new(),
            watching: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::ConfigField;
    use std::path::Path;

    fn draft(filename: &str, original: &str) -> UploadDraft {
        UploadDraft {
            original_content: original.into(),
            ..UploadDraft::fixture("a", filename, Path::new("/tmp").join(filename))
        }
    }

    #[test]
    fn content_prefers_edited_content() {
        let mut d = draft("notes.txt", "token=abc123secret");
        d.edited_content = Some("token=REDACTED".into());
        assert_eq!(d.content(&RuntimeSettings::default()), "token=REDACTED");
    }

    #[test]
    fn content_prefers_edited_content_for_json() {
        let mut d = draft("settings.json", r#"{"token":"abc123secret"}"#);
        d.edited_content = Some(r#"{"token":"REDACTED"}"#.into());
        assert_eq!(
            d.content(&RuntimeSettings::default()),
            r#"{"token":"REDACTED"}"#
        );
    }

    #[test]
    fn content_applies_json_toggles_to_json_files_only() {
        let mut json = draft("settings.json", r#"{"b":1,"a":2}"#);
        json.json_sort = true;
        assert_eq!(
            json.content(&RuntimeSettings::default()),
            r#"{"a":2,"b":1}"#
        );

        let mut text = draft("notes.txt", r#"{"b":1,"a":2}"#);
        text.json_sort = true;
        assert_eq!(
            text.content(&RuntimeSettings::default()),
            r#"{"b":1,"a":2}"#
        );
    }

    #[test]
    fn content_normalizes_crlf_by_default() {
        let d = draft("notes.txt", "a\r\nb\r\n");
        assert_eq!(d.content(&RuntimeSettings::default()), "a\nb\n");
    }

    #[test]
    fn content_preserves_crlf_when_normalization_disabled() {
        let d = draft("notes.txt", "a\r\nb\r\n");
        let mut settings = RuntimeSettings::default();
        settings.adjust(ConfigField::NormalizeLineEndings, true);
        assert_eq!(d.content(&settings), "a\r\nb\r\n");
    }

    #[test]
    fn read_surfaces_an_unreadable_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.txt");
        let result = UploadDraft::read(
            "g1".into(),
            "gone.txt".into(),
            missing,
            String::new(),
            "local".into(),
            "gist".into(),
        );
        assert!(result.is_err());
    }
}
