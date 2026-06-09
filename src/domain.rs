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
}
