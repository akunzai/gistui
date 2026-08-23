use crate::domain::GistCatalog;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Location of the on-disk gist-list cache (`$XDG_CACHE_HOME/gistui/gists.json`).
pub fn cache_path() -> Result<PathBuf> {
    let dir = dirs::cache_dir().context("locate cache directory")?;
    Ok(dir.join("gistui").join("gists.json"))
}

/// Loads the cached Gist catalog. Incompatible or corrupt caches are ignored.
pub fn load_gist_cache(path: &Path) -> Option<GistCatalog> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Writes the gist snapshot to the cache, best-effort.
pub fn save_gist_cache(path: &Path, cache: &GistCatalog) {
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::GistFile;

    fn gist(id: &str, filename: &str) -> GistFile {
        GistFile {
            description: "desc".into(),
            updated_at: "2026-06-09T00:00:00Z".into(),
            created_at: "2026-06-09T00:00:00Z".into(),
            ..GistFile::fixture(id, filename)
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gistui").join("gists.json");
        let cache = GistCatalog {
            owned: vec![gist("a", "one.txt"), gist("b", "two.txt")],
            starred: vec![gist("star1", "x.md")],
            starred_ids: ["star1".into()].into(),
            user_login: Some("me".into()),
            comment_counts: [("a".into(), 2)].into(),
            fork_counts: [("a".into(), 1)].into(),
            star_counts: [("a".into(), 3)].into(),
        };

        save_gist_cache(&path, &cache);
        let loaded = load_gist_cache(&path).unwrap();

        assert_eq!(loaded, cache);
    }

    #[test]
    fn load_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert!(load_gist_cache(&path).is_none());
    }

    #[test]
    fn load_corrupt_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gists.json");
        fs::write(&path, "not json").unwrap();
        assert!(load_gist_cache(&path).is_none());
    }
}
