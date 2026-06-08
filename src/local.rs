use crate::domain::{LocalCandidate, PinnedMapping};
use anyhow::Result;
use std::fs;
use std::path::Path;

/// Lists the files in `cwd` as local candidates. Discovery is scoped to the
/// current working directory only; `pinned` is used solely to mark which of those
/// files already have a saved gist mapping (it does not pull in out-of-cwd paths).
pub fn discover_local_candidates(
    cwd: &Path,
    pinned: &[PinnedMapping],
) -> Result<Vec<LocalCandidate>> {
    let mut paths = Vec::new();

    for entry in fs::read_dir(cwd)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            paths.push(path);
        }
    }

    paths.sort();
    paths.dedup();

    Ok(paths
        .into_iter()
        .map(|path| {
            let pinned_match = pinned.iter().any(|m| m.local_path == path);
            LocalCandidate {
                path,
                pinned: pinned_match,
            }
        })
        .collect())
}

pub fn is_empty_candidate_mode(candidates: &[LocalCandidate]) -> bool {
    candidates.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_cwd_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), "{}").unwrap();
        fs::write(outside.path().join("elsewhere.json"), "{}").unwrap();

        let candidates = discover_local_candidates(dir.path(), &[]).unwrap();
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

        let candidates = discover_local_candidates(dir.path(), &pinned).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, cwd_file);
        assert!(candidates[0].pinned);
    }

    #[test]
    fn empty_candidate_mode_when_nothing_found() {
        let dir = tempfile::tempdir().unwrap();
        let candidates = discover_local_candidates(dir.path(), &[]).unwrap();
        assert!(is_empty_candidate_mode(&candidates));
    }
}
