use crate::domain::{LocalCandidate, PinnedMapping};
use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const VCS_METADATA_DIRS: [&str; 3] = [".git", ".hg", ".svn"];

/// Lists local file candidates under `cwd`.
///
/// When `recursive` is false only the immediate children of `cwd` are
/// returned (original behaviour). When `recursive` is true the tree is
/// walked up to `max_depth`; only version-control metadata and names in
/// `skip_dirs` are skipped. Paths resolving to the same file collapse to one
/// candidate, preferring a pinned path, then the shallowest path.
/// File modification time as Unix seconds, or `None` when it can't be read
/// (missing file, permission error, or a pre-epoch timestamp).
pub fn file_mtime_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

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

    if recursive {
        paths = deduplicate_resolved_paths(paths, cwd, pinned);
    }
    paths.sort();

    Ok(paths
        .into_iter()
        .map(|path| {
            let pinned_match = pinned.iter().any(|m| m.local_path == path);
            let modified = file_mtime_secs(&path);
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
        if path.is_dir() {
            if VCS_METADATA_DIRS.contains(&name) || skip_dirs.iter().any(|d| d == name) {
                continue;
            }
            walk_recursive(&path, paths, depth + 1, skip_dirs, max_depth);
        } else if path.is_file() {
            paths.push(path);
        }
    }
}

fn deduplicate_resolved_paths(
    paths: Vec<PathBuf>,
    cwd: &Path,
    pinned: &[PinnedMapping],
) -> Vec<PathBuf> {
    let mut paths_by_target: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();

    for path in paths {
        let target = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        paths_by_target.entry(target).or_default().push(path);
    }

    paths_by_target
        .into_values()
        .map(|aliases| {
            aliases
                .into_iter()
                .min_by(|left, right| {
                    path_priority(left, cwd, pinned).cmp(&path_priority(right, cwd, pinned))
                })
                .expect("each target has at least one path")
        })
        .collect()
}

fn path_priority<'a>(
    path: &'a Path,
    cwd: &Path,
    pinned: &[PinnedMapping],
) -> (bool, usize, &'a Path) {
    (
        !pinned.iter().any(|mapping| mapping.local_path == path),
        path_depth(path, cwd),
        path,
    )
}

fn path_depth(path: &Path, cwd: &Path) -> usize {
    path.strip_prefix(cwd).unwrap_or(path).components().count()
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
    fn empty_dir_yields_no_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let candidates =
            discover_local_candidates(dir.path(), &[], false, &default_skip(), 10).unwrap();
        assert!(candidates.is_empty());
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
    #[cfg(unix)]
    fn recursive_includes_hidden_paths_excludes_vcs_and_deduplicates_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let hidden_file = dir.path().join(".hidden-file");
        let hidden_dir_file = dir.path().join(".config/settings.toml");
        let pinned_file = dir.path().join("nested/pinned.toml");
        let alias = dir.path().join(".aliases/pinned.toml");
        fs::create_dir_all(dir.path().join(".config")).unwrap();
        fs::create_dir_all(dir.path().join(".aliases")).unwrap();
        fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        fs::create_dir_all(dir.path().join(".hg/store")).unwrap();
        fs::create_dir_all(dir.path().join(".svn/pristine")).unwrap();
        fs::create_dir_all(dir.path().join(".ignored")).unwrap();
        fs::create_dir_all(dir.path().join("nested")).unwrap();
        fs::write(&hidden_file, "").unwrap();
        fs::write(&hidden_dir_file, "").unwrap();
        fs::write(&pinned_file, "").unwrap();
        fs::write(dir.path().join(".git/objects/abc"), "").unwrap();
        fs::write(dir.path().join(".hg/store/abc"), "").unwrap();
        fs::write(dir.path().join(".svn/pristine/abc"), "").unwrap();
        fs::write(dir.path().join(".ignored/file"), "").unwrap();
        std::os::unix::fs::symlink(&pinned_file, &alias).unwrap();
        let pinned = vec![PinnedMapping {
            local_path: pinned_file.clone(),
            gist_id: "gist".into(),
            gist_filename: "pinned.toml".into(),
            direction: None,
            last_seen_hash: None,
        }];
        let skip_dirs = vec![".ignored".to_string()];

        let candidates =
            discover_local_candidates(dir.path(), &pinned, true, &skip_dirs, 10).unwrap();
        let paths: Vec<_> = candidates
            .iter()
            .map(|candidate| candidate.path.clone())
            .collect();
        let flat_candidates =
            discover_local_candidates(dir.path(), &pinned, false, &skip_dirs, 10).unwrap();

        assert!(paths.contains(&hidden_file));
        assert!(paths.contains(&hidden_dir_file));
        assert!(paths.contains(&pinned_file));
        assert!(!paths.contains(&alias));
        assert!(!paths
            .iter()
            .any(|path| path.starts_with(dir.path().join(".git"))));
        assert!(!paths
            .iter()
            .any(|path| path.starts_with(dir.path().join(".hg"))));
        assert!(!paths
            .iter()
            .any(|path| path.starts_with(dir.path().join(".svn"))));
        assert!(!paths
            .iter()
            .any(|path| path.starts_with(dir.path().join(".ignored"))));
        assert!(candidates
            .iter()
            .any(|candidate| candidate.path == pinned_file && candidate.pinned));
        assert!(flat_candidates
            .iter()
            .all(|candidate| paths.contains(&candidate.path)));
    }

    #[test]
    #[cfg(unix)]
    fn recursive_deduplication_prefers_the_shallowest_unpinned_alias() {
        let dir = tempfile::tempdir().unwrap();
        let direct = dir.path().join("shared.toml");
        let alias = dir.path().join("nested/alias.toml");
        fs::create_dir_all(dir.path().join("nested")).unwrap();
        fs::write(&direct, "").unwrap();
        std::os::unix::fs::symlink(&direct, &alias).unwrap();

        let candidates =
            discover_local_candidates(dir.path(), &[], true, &default_skip(), 10).unwrap();
        let paths: Vec<_> = candidates.iter().map(|candidate| &candidate.path).collect();

        assert_eq!(paths, vec![&direct]);
    }

    #[test]
    #[cfg(unix)]
    fn recursive_discovery_deduplicates_flat_aliases_by_path_priority() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.toml");
        let alias = dir.path().join("b.toml");
        fs::write(&file, "").unwrap();
        std::os::unix::fs::symlink(&file, &alias).unwrap();

        let flat = discover_local_candidates(dir.path(), &[], false, &default_skip(), 10).unwrap();
        let recursive =
            discover_local_candidates(dir.path(), &[], true, &default_skip(), 10).unwrap();
        let flat_paths: Vec<_> = flat.iter().map(|candidate| &candidate.path).collect();
        let recursive_paths: Vec<_> = recursive.iter().map(|candidate| &candidate.path).collect();

        assert_eq!(flat_paths, vec![&file, &alias]);
        assert_eq!(recursive_paths, vec![&file]);
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
