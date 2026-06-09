use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedMapping {
    pub local_path: PathBuf,
    pub gist_id: String,
    pub gist_filename: String,
    pub direction: Option<SyncDirection>,
    pub last_seen_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCandidate {
    pub path: PathBuf,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GistFile {
    pub gist_id: String,
    pub description: String,
    pub filename: String,
    pub public: bool,
    pub updated_at: String,
    pub created_at: String,
}

/// A gist-level view of the flat [`GistFile`] rows: one entry per gist, carrying
/// the shared metadata plus how many files it holds. Used by `Screen::Gists`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GistGroup {
    pub id: String,
    pub description: String,
    pub public: bool,
    pub updated_at: String,
    pub created_at: String,
    pub file_count: usize,
}

/// Collapses the flat per-file rows into one [`GistGroup`] per gist, preserving
/// the first-seen order of `files` (which mirrors the `gh` list order).
pub fn group_gists(files: &[GistFile]) -> Vec<GistGroup> {
    let mut groups: Vec<GistGroup> = Vec::new();
    for file in files {
        if let Some(group) = groups.iter_mut().find(|g| g.id == file.gist_id) {
            group.file_count += 1;
        } else {
            groups.push(GistGroup {
                id: file.gist_id.clone(),
                description: file.description.clone(),
                public: file.public,
                updated_at: file.updated_at.clone(),
                created_at: file.created_at.clone(),
                file_count: 1,
            });
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(gist_id: &str, filename: &str, desc: &str, public: bool) -> GistFile {
        GistFile {
            gist_id: gist_id.into(),
            description: desc.into(),
            filename: filename.into(),
            public,
            updated_at: "2026-06-08T00:00:00Z".into(),
            created_at: "2026-06-08T00:00:00Z".into(),
        }
    }

    #[test]
    fn groups_files_by_gist_with_counts_and_order() {
        let files = vec![
            file("abc", "a.rs", "first", false),
            file("abc", "b.rs", "first", false),
            file("xyz", "c.md", "second", true),
        ];

        let groups = group_gists(&files);

        assert_eq!(groups.len(), 2);
        // First-seen order is preserved (gh list order).
        assert_eq!(groups[0].id, "abc");
        assert_eq!(groups[0].file_count, 2);
        assert_eq!(groups[0].description, "first");
        assert!(!groups[0].public);
        assert_eq!(groups[1].id, "xyz");
        assert_eq!(groups[1].file_count, 1);
        assert!(groups[1].public);
    }

    #[test]
    fn empty_input_yields_no_groups() {
        assert!(group_gists(&[]).is_empty());
    }
}
