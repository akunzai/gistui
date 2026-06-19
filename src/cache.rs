use crate::domain::GistFile;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Location of the on-disk gist-list cache (`$XDG_CACHE_HOME/gistui/gists.json`).
pub fn cache_path() -> Result<PathBuf> {
    let dir = dirs::cache_dir().context("locate cache directory")?;
    Ok(dir.join("gistui").join("gists.json"))
}

/// Full startup cache: owned + starred lists and the metadata that powers the gist manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GistListCache {
    #[serde(default)]
    pub owned: Vec<GistFile>,
    #[serde(default)]
    pub starred: Vec<GistFile>,
    #[serde(default)]
    pub starred_ids: Vec<String>,
    #[serde(default)]
    pub user_login: Option<String>,
    #[serde(default)]
    pub comment_counts: HashMap<String, u32>,
    #[serde(default)]
    pub fork_counts: HashMap<String, u32>,
}

impl GistListCache {
    pub fn starred_ids_set(&self) -> HashSet<String> {
        self.starred_ids.iter().cloned().collect()
    }
}

/// Loads the cached gist snapshot. Accepts the legacy bare `[GistFile, …]` format.
pub fn load_gist_cache(path: &Path) -> Option<GistListCache> {
    let raw = fs::read_to_string(path).ok()?;
    if let Ok(cache) = serde_json::from_str::<GistListCache>(&raw) {
        return Some(cache);
    }
    serde_json::from_str::<Vec<GistFile>>(&raw)
        .ok()
        .map(|owned| GistListCache {
            owned,
            ..GistListCache::default()
        })
}

/// Writes the gist snapshot to the cache, best-effort.
pub fn save_gist_cache(path: &Path, cache: &GistListCache) {
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = fs::write(path, json);
    }
}

/// Legacy helper — loads only the owned gist rows from cache.
pub fn load_cached_gists(path: &Path) -> Vec<GistFile> {
    load_gist_cache(path).map(|c| c.owned).unwrap_or_default()
}

/// Legacy helper — saves only owned gist rows (prefer [`save_gist_cache`]).
pub fn save_cached_gists(path: &Path, gists: &[GistFile]) {
    save_gist_cache(
        path,
        &GistListCache {
            owned: gists.to_vec(),
            ..GistListCache::default()
        },
    );
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
            owner_login: String::new(),
            fork_of_id: None,
            raw_url: None,
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gistui").join("gists.json");
        let cache = GistListCache {
            owned: vec![gist("a", "one.txt"), gist("b", "two.txt")],
            starred: vec![gist("star1", "x.md")],
            starred_ids: vec!["star1".into()],
            user_login: Some("me".into()),
            comment_counts: [("a".into(), 2)].into(),
            fork_counts: [("a".into(), 1)].into(),
        };

        save_gist_cache(&path, &cache);
        let loaded = load_gist_cache(&path).unwrap();

        assert_eq!(loaded, cache);
    }

    #[test]
    fn load_legacy_bare_array() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gists.json");
        let gists = vec![gist("a", "one.txt")];
        fs::write(&path, serde_json::to_string(&gists).unwrap()).unwrap();
        let loaded = load_gist_cache(&path).unwrap();
        assert_eq!(loaded.owned, gists);
        assert!(loaded.starred.is_empty());
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
