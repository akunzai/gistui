use crate::domain::{GistFile, PinnedMapping};
use std::collections::HashSet;
use std::path::Component;
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
    PathSegment,
    Recent,
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
    let local_tokens = meaningful_path_tokens(local_path);

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

            let gist_tokens = text_tokens(&format!("{} {}", file.description, file.filename));
            if local_tokens.iter().any(|token| gist_tokens.contains(token)) {
                score += 250;
                reasons.push(MatchReason::PathSegment);
            }

            if !file.updated_at.is_empty() {
                score += 1;
                reasons.push(MatchReason::Recent);
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

fn meaningful_path_tokens(path: &Path) -> HashSet<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => None,
        })
        .flat_map(|component| text_tokens(component.trim_start_matches('.')))
        .filter(|token| !is_common_container_dir(token))
        .collect()
}

fn text_tokens(text: &str) -> HashSet<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(String::from)
        .collect()
}

fn is_common_container_dir(token: &str) -> bool {
    matches!(token, "users" | "home" | "tmp" | "var" | "private")
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
    fn exact_filename_beats_path_hint() {
        let local = PathBuf::from("/Users/me/.claude/settings.json");
        let files = vec![
            gist("a", "claude config", "other.json"),
            gist("b", "misc", "settings.json"),
        ];

        let ranked = rank_gist_files(&local, &files, &[]);
        assert_eq!(ranked[0].file.gist_id, "b");
    }

    #[test]
    fn dot_prefixed_config_directory_can_match_path_segment() {
        let local = PathBuf::from("/Users/me/.claude/settings.json");
        let files = vec![gist("a", "claude config", "other.json")];

        let ranked = rank_gist_files(&local, &files, &[]);
        assert!(ranked[0].reasons.contains(&MatchReason::PathSegment));
    }

    #[test]
    fn short_ancestor_token_does_not_match_unrelated_words() {
        let local = PathBuf::from("/Users/me/project/config.json");
        let files = vec![gist("a", "theme notes", "readme.md")];

        let ranked = rank_gist_files(&local, &files, &[]);
        assert!(!ranked[0].reasons.contains(&MatchReason::PathSegment));
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

    #[test]
    fn no_path_segment_reason_for_unrelated_tokens() {
        let local = PathBuf::from("/Users/me/.claude/settings.json");
        let files = vec![gist("a", "editor theme", "readme.md")];

        let ranked = rank_gist_files(&local, &files, &[]);
        assert!(!ranked[0].reasons.contains(&MatchReason::PathSegment));
    }
}
