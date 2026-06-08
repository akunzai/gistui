use crate::domain::{LocalCandidate, PinnedMapping};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

pub fn known_config_paths(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".zshrc"),
        home.join(".gemini/antigravity-cli/settings.json"),
        home.join(".config/opencode/opencode.json"),
        home.join(".claude/settings.json"),
        home.join(".claude/statusline.sh"),
    ]
}

pub fn discover_local_candidates(
    cwd: &Path,
    home: &Path,
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

    for path in known_config_paths(home) {
        if path.is_file() {
            paths.push(path);
        }
    }

    for mapping in pinned {
        if mapping.local_path.is_file() {
            paths.push(mapping.local_path.clone());
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
    fn discovers_cwd_files_and_known_configs() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), "{}").unwrap();
        fs::create_dir_all(home.path().join(".claude")).unwrap();
        fs::write(home.path().join(".claude/settings.json"), "{}").unwrap();

        let candidates = discover_local_candidates(dir.path(), home.path(), &[]).unwrap();
        let paths: Vec<_> = candidates.iter().map(|c| c.path.clone()).collect();

        assert!(paths.contains(&dir.path().join("settings.json")));
        assert!(paths.contains(&home.path().join(".claude/settings.json")));
    }

    #[test]
    fn empty_candidate_mode_when_nothing_found() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let candidates = discover_local_candidates(dir.path(), home.path(), &[]).unwrap();
        assert!(is_empty_candidate_mode(&candidates));
    }
}
