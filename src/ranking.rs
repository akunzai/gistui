use crate::domain::{GistFile, LocalCandidate, PinnedMapping};
use std::cmp::Ordering;
use std::path::Path;

/// How a list row relates to the opposite pane's selection (sort key; UI maps this to style).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMark {
    /// No pin / filename match.
    None,
    /// Same basename as the opposite selection.
    ExactFilename,
    /// Explicit pin mapping (ranks above exact name).
    Pinned,
}

impl MatchMark {
    /// Sort weight: higher wins. Explicit — not derived from variant order.
    pub const fn rank(self) -> u8 {
        match self {
            MatchMark::None => 0,
            MatchMark::ExactFilename => 1,
            MatchMark::Pinned => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedGistFile {
    pub file: GistFile,
    pub mark: MatchMark,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedLocal {
    pub candidate: LocalCandidate,
    pub mark: MatchMark,
}

fn match_mark(
    local_path: &Path,
    local_filename: &str,
    gist_id: &str,
    gist_filename: &str,
    pinned: &[PinnedMapping],
) -> MatchMark {
    if pinned.iter().any(|m| {
        m.local_path == local_path && m.gist_id == gist_id && m.gist_filename == gist_filename
    }) {
        MatchMark::Pinned
    } else if local_filename == gist_filename {
        MatchMark::ExactFilename
    } else {
        MatchMark::None
    }
}

fn cmp_mark_then(a_mark: MatchMark, b_mark: MatchMark, tie: Ordering) -> Ordering {
    b_mark.rank().cmp(&a_mark.rank()).then(tie)
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
            let mark = match_mark(
                local_path,
                local_filename,
                &file.gist_id,
                &file.filename,
                pinned,
            );
            RankedGistFile { file, mark }
        })
        .collect();

    ranked.sort_by(|a, b| cmp_mark_then(a.mark, b.mark, a.file.filename.cmp(&b.file.filename)));
    ranked
}

/// Scores local files by how well they match a selected gist (gist-pane reverse ranking).
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
            let mark = match_mark(
                &candidate.path,
                local_filename,
                &gist.gist_id,
                &gist.filename,
                pinned,
            );
            RankedLocal { candidate, mark }
        })
        .collect();

    ranked.sort_by(|a, b| cmp_mark_then(a.mark, b.mark, a.candidate.path.cmp(&b.candidate.path)));
    ranked
}

/// Unranked pane row (no opposite selection driving the score).
pub fn unranked_gist(file: GistFile) -> RankedGistFile {
    RankedGistFile {
        file,
        mark: MatchMark::None,
    }
}

pub fn unranked_local(candidate: LocalCandidate) -> RankedLocal {
    RankedLocal {
        candidate,
        mark: MatchMark::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn gist(id: &str, description: &str, filename: &str) -> GistFile {
        GistFile {
            description: description.into(),
            updated_at: "2026-06-08T00:00:00Z".into(),
            created_at: "2026-06-08T00:00:00Z".into(),
            ..GistFile::fixture(id, filename)
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
        assert_eq!(ranked[0].mark, MatchMark::Pinned);
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
        assert_eq!(ranked[0].mark, MatchMark::ExactFilename);
    }

    #[test]
    fn filename_tie_break_ascending_when_marks_are_equal() {
        let local = PathBuf::from("/Users/me/project/config.json");
        let files = vec![
            gist("a", "unrelated", "zeta.txt"),
            gist("b", "unrelated", "alpha.txt"),
        ];

        let ranked = rank_gist_files(&local, &files, &[]);
        assert_eq!(ranked[0].file.filename, "alpha.txt");
        assert_eq!(ranked[1].file.filename, "zeta.txt");
    }

    #[test]
    fn mark_rank_is_explicit_not_discriminant_order() {
        assert!(MatchMark::Pinned.rank() > MatchMark::ExactFilename.rank());
        assert!(MatchMark::ExactFilename.rank() > MatchMark::None.rank());
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
        assert_eq!(ranked[0].mark, MatchMark::ExactFilename);
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
        assert_eq!(ranked[0].mark, MatchMark::Pinned);
    }
}
