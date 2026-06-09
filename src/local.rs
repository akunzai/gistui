use crate::domain::{LocalCandidate, PinnedMapping};
use anyhow::Result;
use std::fs;
use std::path::Path;

/// Lists local file candidates under `cwd`.
///
/// When `recursive` is false only the immediate children of `cwd` are
/// returned (original behaviour). When `recursive` is true the tree is
/// walked up to 10 levels deep; hidden directories (name starts with `.`)
/// and names in `skip_dirs` are skipped.
pub fn discover_local_candidates(
    cwd: &Path,
    pinned: &[PinnedMapping],
    recursive: bool,
    skip_dirs: &[String],
    max_depth: u32,
) -> Result<Vec<LocalCandidate>> {
    let mut paths = Vec::new();

    if recursive {
        walk_recursive(cwd, &mut paths, 0, skip_dirs, max_depth);
    } else {
        for entry in fs::read_dir(cwd)? {
            let path = entry?.path();
            if path.is_file() {
                paths.push(path);
            }
        }
    }

    paths.sort();
    paths.dedup();

    Ok(paths
        .into_iter()
        .map(|path| {
            let pinned_match = pinned.iter().any(|m| m.local_path == path);
            let modified = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            LocalCandidate {
                path,
                pinned: pinned_match,
                modified,
            }
        })
        .collect())
}

fn walk_recursive(
    dir: &Path,
    paths: &mut Vec<std::path::PathBuf>,
    depth: u32,
    skip_dirs: &[String],
    max_depth: u32,
) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if skip_dirs.iter().any(|d| d == name) {
                continue;
            }
            walk_recursive(&path, paths, depth + 1, skip_dirs, max_depth);
        } else if path.is_file() {
            paths.push(path);
        }
    }
}

pub fn is_empty_candidate_mode(candidates: &[LocalCandidate]) -> bool {
    candidates.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_skip() -> Vec<String> {
        crate::config::AppConfig::default().skip_dirs
    }

    #[test]
    fn discovers_cwd_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), "{}").unwrap();
        fs::write(outside.path().join("elsewhere.json"), "{}").unwrap();

        let candidates =
            discover_local_candidates(dir.path(), &[], false, &default_skip(), 10).unwrap();
        let paths: Vec<_> = candidates.iter().map(|c| c.path.clone()).collect();

        assert!(paths.contains(&dir.path().join("settings.json")));
        assert!(!paths.contains(&outside.path().join("elsewhere.json")));
    }

    #[test]
    fn marks_pinned_cwd_files_without_pulling_outside_paths() {
        let dir = tempfile::tempdir().unwrap();
        let cwd_file = dir.path().join("settings.json");
        fs::write(&cwd_file, "{}").unwrap();
        let outside = dir.path().join("nope/elsewhere.json");
        let pinned = vec![
            PinnedMapping {
                local_path: cwd_file.clone(),
                gist_id: "a".into(),
                gist_filename: "settings.json".into(),
                direction: None,
                last_seen_hash: None,
            },
            PinnedMapping {
                local_path: outside,
                gist_id: "b".into(),
                gist_filename: "x".into(),
                direction: None,
                last_seen_hash: None,
            },
        ];

        let candidates =
            discover_local_candidates(dir.path(), &pinned, false, &default_skip(), 10).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, cwd_file);
        assert!(candidates[0].pinned);
    }

    #[test]
    fn empty_candidate_mode_when_nothing_found() {
        let dir = tempfile::tempdir().unwrap();
        let candidates =
            discover_local_candidates(dir.path(), &[], false, &default_skip(), 10).unwrap();
        assert!(is_empty_candidate_mode(&candidates));
    }

    #[test]
    fn recursive_finds_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/utils")).unwrap();
        fs::write(dir.path().join("README.md"), "").unwrap();
        fs::write(dir.path().join("src/main.rs"), "").unwrap();
        fs::write(dir.path().join("src/utils/helpers.rs"), "").unwrap();

        let candidates =
            discover_local_candidates(dir.path(), &[], true, &default_skip(), 10).unwrap();
        let paths: Vec<_> = candidates.iter().map(|c| c.path.clone()).collect();

        assert!(paths.contains(&dir.path().join("README.md")));
        assert!(paths.contains(&dir.path().join("src/main.rs")));
        assert!(paths.contains(&dir.path().join("src/utils/helpers.rs")));
    }

    #[test]
    fn recursive_skips_denied_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("node_modules/lodash")).unwrap();
        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        fs::write(dir.path().join("node_modules/lodash/index.js"), "").unwrap();
        fs::write(dir.path().join("target/debug/app"), "").unwrap();
        fs::write(dir.path().join("src.rs"), "").unwrap();

        let candidates =
            discover_local_candidates(dir.path(), &[], true, &default_skip(), 10).unwrap();
        let paths: Vec<_> = candidates.iter().map(|c| c.path.clone()).collect();

        assert!(paths.contains(&dir.path().join("src.rs")));
        assert!(!paths
            .iter()
            .any(|p| p.to_string_lossy().contains("node_modules")));
        assert!(!paths.iter().any(|p| p.to_string_lossy().contains("target")));
    }

    #[test]
    fn recursive_skips_hidden_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        fs::write(dir.path().join(".git/objects/abc"), "").unwrap();
        fs::write(dir.path().join("visible.rs"), "").unwrap();

        let candidates =
            discover_local_candidates(dir.path(), &[], true, &default_skip(), 10).unwrap();
        let paths: Vec<_> = candidates.iter().map(|c| c.path.clone()).collect();

        assert!(paths.contains(&dir.path().join("visible.rs")));
        assert!(!paths.iter().any(|p| p.to_string_lossy().contains(".git")));
    }

    #[test]
    fn recursive_custom_skip_dirs_are_respected() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("custom_skip")).unwrap();
        fs::write(dir.path().join("custom_skip/file.txt"), "").unwrap();
        fs::write(dir.path().join("visible.txt"), "").unwrap();

        let skip = vec!["custom_skip".to_string()];
        let candidates = discover_local_candidates(dir.path(), &[], true, &skip, 10).unwrap();
        let paths: Vec<_> = candidates.iter().map(|c| c.path.clone()).collect();

        assert!(paths.contains(&dir.path().join("visible.txt")));
        assert!(!paths
            .iter()
            .any(|p| p.to_string_lossy().contains("custom_skip")));
    }
}
