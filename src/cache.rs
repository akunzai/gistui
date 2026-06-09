use crate::domain::GistFile;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Location of the on-disk gist-list cache (`$XDG_CACHE_HOME/gistui/gists.json`).
pub fn cache_path() -> Result<PathBuf> {
    let dir = dirs::cache_dir().context("locate cache directory")?;
    Ok(dir.join("gistui").join("gists.json"))
}

/// Loads the cached gist list. The cache is best-effort: any missing file or parse error
/// yields an empty list rather than failing startup.
pub fn load_cached_gists(path: &Path) -> Vec<GistFile> {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Writes the gist list to the cache, best-effort. Errors (e.g. missing cache dir) are
/// swallowed so a failed cache write never disrupts the app.
pub fn save_cached_gists(path: &Path, gists: &[GistFile]) {
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if let Ok(json) = serde_json::to_string(gists) {
        let _ = fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gist(id: &str, filename: &str) -> GistFile {
        GistFile {
            gist_id: id.into(),
            description: "desc".into(),
            filename: filename.into(),
            public: false,
            updated_at: "2026-06-09T00:00:00Z".into(),
            created_at: "2026-06-09T00:00:00Z".into(),
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gistui").join("gists.json");
        let gists = vec![gist("a", "one.txt"), gist("b", "two.txt")];

        save_cached_gists(&path, &gists);
        let loaded = load_cached_gists(&path);

        assert_eq!(loaded, gists);
    }

    #[test]
    fn load_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert!(load_cached_gists(&path).is_empty());
    }

    #[test]
    fn load_corrupt_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gists.json");
        fs::write(&path, "not json").unwrap();
        assert!(load_cached_gists(&path).is_empty());
    }
}
