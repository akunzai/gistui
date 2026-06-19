use crate::domain::{GistFile, LocalCandidate, PinnedMapping};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedGistFile {
    pub file: GistFile,
    pub score: u16,
    pub reasons: Vec<MatchReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchReason {
    Pinned,
    ExactFilename,
}

pub fn rank_gist_files(
    local_path: &Path,
    gist_files: &[GistFile],
    pinned: &[PinnedMapping],
) -> Vec<RankedGistFile> {
    let local_filename = local_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();

    let mut ranked: Vec<_> = gist_files
        .iter()
        .cloned()
        .map(|file| {
            let mut score = 0;
            let mut reasons = Vec::new();

            if pinned.iter().any(|m| {
                m.local_path == local_path
                    && m.gist_id == file.gist_id
                    && m.gist_filename == file.filename
            }) {
                score += 10_000;
                reasons.push(MatchReason::Pinned);
            }

            if file.filename == local_filename {
                score += 1_000;
                reasons.push(MatchReason::ExactFilename);
            }

            RankedGistFile {
                file,
                score,
                reasons,
            }
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.file.filename.cmp(&b.file.filename))
    });
    ranked
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedLocal {
    pub candidate: LocalCandidate,
    pub score: u16,
    pub reasons: Vec<MatchReason>,
}

/// The mirror of [`rank_gist_files`]: scores local files by how well they match a
/// selected gist (used for the gist-pane-driven reverse ranking).
pub fn rank_local_files(
    gist: &GistFile,
    locals: &[LocalCandidate],
    pinned: &[PinnedMapping],
) -> Vec<RankedLocal> {
    let mut ranked: Vec<_> = locals
        .iter()
        .cloned()
        .map(|candidate| {
            let local_filename = candidate
                .path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let mut score = 0;
            let mut reasons = Vec::new();

            if pinned.iter().any(|m| {
                m.local_path == candidate.path
                    && m.gist_id == gist.gist_id
                    && m.gist_filename == gist.filename
            }) {
                score += 10_000;
                reasons.push(MatchReason::Pinned);
            }

            if local_filename == gist.filename {
                score += 1_000;
                reasons.push(MatchReason::ExactFilename);
            }

            RankedLocal {
                candidate,
                score,
                reasons,
            }
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.candidate.path.cmp(&b.candidate.path))
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn gist(id: &str, description: &str, filename: &str) -> GistFile {
        GistFile {
            gist_id: id.into(),
            description: description.into(),
            filename: filename.into(),
            public: false,
            updated_at: "2026-06-08T00:00:00Z".into(),
            created_at: "2026-06-08T00:00:00Z".into(),
            owner_login: String::new(),
            fork_of_id: None,
        }
    }

    #[test]
    fn pinned_mapping_wins_over_filename_match() {
        let local = PathBuf::from("/Users/me/.claude/settings.json");
        let files = vec![
            gist("a", "exact filename", "settings.json"),
            gist("b", "old pinned", "other.json"),
        ];
        let pinned = vec![PinnedMapping {
            local_path: local.clone(),
            gist_id: "b".into(),
            gist_filename: "other.json".into(),
            direction: None,
            last_seen_hash: None,
        }];

        let ranked = rank_gist_files(&local, &files, &pinned);
        assert_eq!(ranked[0].file.gist_id, "b");
        assert!(ranked[0].reasons.contains(&MatchReason::Pinned));
    }

    #[test]
    fn exact_filename_ranks_above_no_match() {
        let local = PathBuf::from("/Users/me/.claude/settings.json");
        let files = vec![
            gist("a", "claude config", "other.json"),
            gist("b", "misc", "settings.json"),
        ];

        let ranked = rank_gist_files(&local, &files, &[]);
        assert_eq!(ranked[0].file.gist_id, "b");
    }

    #[test]
    fn filename_tie_break_ascending_when_scores_are_equal() {
        let local = PathBuf::from("/Users/me/project/config.json");
        let files = vec![
            gist("a", "unrelated", "zeta.txt"),
            gist("b", "unrelated", "alpha.txt"),
        ];

        let ranked = rank_gist_files(&local, &files, &[]);
        assert_eq!(ranked[0].file.filename, "alpha.txt");
        assert_eq!(ranked[1].file.filename, "zeta.txt");
    }

    fn local(path: &str) -> LocalCandidate {
        LocalCandidate {
            path: PathBuf::from(path),
            pinned: false,
            modified: None,
        }
    }

    #[test]
    fn rank_local_files_prefers_exact_filename_match_to_the_gist() {
        let target = gist("a", "claude config", "settings.json");
        let locals = vec![
            local("/Users/me/project/other.json"),
            local("/Users/me/.claude/settings.json"),
        ];

        let ranked = rank_local_files(&target, &locals, &[]);
        assert_eq!(
            ranked[0].candidate.path,
            PathBuf::from("/Users/me/.claude/settings.json")
        );
        assert!(ranked[0].reasons.contains(&MatchReason::ExactFilename));
    }

    #[test]
    fn rank_local_files_pin_outranks_filename() {
        let target = gist("b", "notes", "todo.md");
        let pinned_local = local("/Users/me/work/scratch.txt");
        let locals = vec![local("/Users/me/work/todo.md"), pinned_local.clone()];
        let pinned = vec![PinnedMapping {
            local_path: pinned_local.path.clone(),
            gist_id: "b".into(),
            gist_filename: "todo.md".into(),
            direction: None,
            last_seen_hash: None,
        }];

        let ranked = rank_local_files(&target, &locals, &pinned);
        assert_eq!(ranked[0].candidate.path, pinned_local.path);
        assert!(ranked[0].reasons.contains(&MatchReason::Pinned));
    }
}
