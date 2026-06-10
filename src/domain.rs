use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    /// File mtime as Unix seconds, captured at discovery so the "recent" local
    /// sort stays a pure comparison (no filesystem access in `handle_key`).
    pub modified: Option<u64>,
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

/// Lowercase hex SHA-256 of `bytes`. Used as the stable, content-only digest
/// persisted in `PinnedMapping.last_seen_hash` (the config never stores content).
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
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

    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

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

pub fn transform_json(
    content: &str,
    pretty: bool,
    sort: bool,
) -> Result<String, serde_json::Error> {
    if !pretty && !sort {
        return Ok(content.to_string());
    }
    let mut val: serde_json::Value = serde_json::from_str(content)?;
    if sort {
        val = sort_json_value(val);
    }
    if pretty {
        serde_json::to_string_pretty(&val)
    } else {
        serde_json::to_string(&val)
    }
}

pub fn sort_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(String, serde_json::Value)> = map
                .into_iter()
                .map(|(k, v)| (k, sort_json_value(v)))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut sorted_map = serde_json::Map::new();
            for (k, v) in entries {
                sorted_map.insert(k, v);
            }
            serde_json::Value::Object(sorted_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(sort_json_value).collect())
        }
        other => other,
    }
}

#[cfg(test)]
mod json_tests {
    use super::*;

    #[test]
    fn test_transform_json_noop() {
        let input = r#"{"z":1,"a":2}"#;
        let output = transform_json(input, false, false).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn test_transform_json_pretty_only() {
        let input = r#"{"z":1,"a":2}"#;
        let output = transform_json(input, true, false).unwrap();
        // Should be formatted, but preserve original key order ("z" then "a")
        let expected = "{\n  \"z\": 1,\n  \"a\": 2\n}";
        assert_eq!(output, expected);
    }

    #[test]
    fn test_transform_json_sort_only() {
        let input = r#"{"z":1,"a":2}"#;
        let output = transform_json(input, false, true).unwrap();
        // Should be compact, but sorted keys ("a" then "z")
        assert_eq!(output, r#"{"a":2,"z":1}"#);
    }

    #[test]
    fn test_transform_json_pretty_and_sort() {
        let input = r#"{"z":1,"a":{"y":3,"x":4}}"#;
        let output = transform_json(input, true, true).unwrap();
        // Should be formatted, and keys sorted recursively ("a" then "z", and "x" then "y")
        let expected = "{\n  \"a\": {\n    \"x\": 4,\n    \"y\": 3\n  },\n  \"z\": 1\n}";
        assert_eq!(output, expected);
    }

    #[test]
    fn test_transform_json_invalid() {
        let input = r#"{"z":1,"a":}"#;
        let result = transform_json(input, true, true);
        assert!(result.is_err());
    }
}
