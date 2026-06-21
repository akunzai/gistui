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

/// Suggested sync action for a pinned pair, decided by comparing modification
/// times. Pure; derived by [`sync_status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    /// Both sides carry the same modification time.
    InSync,
    /// Local is newer → upload local into the gist.
    Push,
    /// Remote is newer → download the gist into the local file.
    Pull,
    /// A timestamp is unavailable — direction cannot be suggested.
    Unknown,
}

impl SyncStatus {
    /// Single-glyph indicator shown in the Pins list.
    pub fn icon(self) -> &'static str {
        match self {
            SyncStatus::InSync => "✓",
            SyncStatus::Push => "↑",
            SyncStatus::Pull => "↓",
            SyncStatus::Unknown => "?",
        }
    }
}

/// Pure decision: which side is newer? `local_ts`/`remote_ts` are Unix seconds;
/// `None` means the timestamp was unavailable.
pub fn sync_status(local_ts: Option<u64>, remote_ts: Option<u64>) -> SyncStatus {
    match (local_ts, remote_ts) {
        (Some(l), Some(r)) if l > r => SyncStatus::Push,
        (Some(l), Some(r)) if r > l => SyncStatus::Pull,
        (Some(_), Some(_)) => SyncStatus::InSync,
        _ => SyncStatus::Unknown,
    }
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
    /// `owner.login` from the gist list API (empty when unknown).
    #[serde(default)]
    pub owner_login: String,
    /// Upstream gist id when this row is a fork under your account.
    #[serde(default)]
    pub fork_of_id: Option<String>,
    /// Direct raw URL from the gist list API; used when `gh gist view` fails on huge gists.
    #[serde(default)]
    pub raw_url: Option<String>,
    /// MIME type from the gist list API (`files[].type`), when present.
    #[serde(default)]
    pub content_type: Option<String>,
    /// GraphQL global id from the gist list API (`node_id`), for stargazer counts.
    #[serde(default)]
    pub node_id: Option<String>,
}

/// File extensions that are treated as non-text for preview/diff (images, archives, media, …).
const BINARY_EXTENSIONS: &[&str] = &[
    "avif", "bmp", "bz2", "deb", "dll", "dmg", "dylib", "eot", "exe", "flac", "gif", "gz", "heic",
    "heif", "ico", "iso", "jpeg", "jpg", "mkv", "mov", "mp3", "mp4", "ogg", "otf", "pdf", "png",
    "rar", "rpm", "so", "tar", "tif", "tiff", "ttf", "wav", "wasm", "webm", "webp", "woff",
    "woff2", "xz", "zip", "7z",
];

fn mime_is_non_text(mime: &str) -> bool {
    let lower = mime.to_ascii_lowercase();
    if lower.starts_with("image/")
        || lower.starts_with("audio/")
        || lower.starts_with("video/")
        || lower.starts_with("font/")
    {
        return true;
    }
    matches!(
        lower.as_str(),
        "application/octet-stream"
            | "application/pdf"
            | "application/zip"
            | "application/gzip"
            | "application/x-gzip"
            | "application/x-tar"
            | "application/x-bzip2"
            | "application/x-7z-compressed"
            | "application/vnd.microsoft.portable-executable"
            | "application/wasm"
    )
}

fn mime_is_text(mime: &str) -> bool {
    let lower = mime.to_ascii_lowercase();
    if lower.starts_with("text/") {
        return true;
    }
    if lower.starts_with("application/x-") && !mime_is_non_text(&lower) {
        // GitHub gist script types: application/x-python, application/x-sh, …
        return true;
    }
    matches!(
        lower.as_str(),
        "application/json"
            | "application/javascript"
            | "application/ld+json"
            | "application/xml"
            | "application/yaml"
            | "application/x-yaml"
            | "application/graphql"
    )
}

/// True when the filename extension is a known binary/image type.
pub fn extension_looks_binary(filename: &str) -> bool {
    std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            BINARY_EXTENSIONS
                .iter()
                .any(|&deny| e.eq_ignore_ascii_case(deny))
        })
}

/// Whether gistui can show this gist file as text in preview/diff views.
/// Uses the list API MIME type when available, otherwise the filename extension.
pub fn gist_file_is_text_previewable(filename: &str, content_type: Option<&str>) -> bool {
    if let Some(mime) = content_type.filter(|s| !s.is_empty()) {
        if mime_is_non_text(mime) {
            return false;
        }
        if mime_is_text(mime) {
            return true;
        }
    }
    !extension_looks_binary(filename)
}

/// Short reason for blocking preview/diff (e.g. "image file", "binary file").
pub fn gist_file_non_previewable_reason(
    filename: &str,
    content_type: Option<&str>,
) -> &'static str {
    if content_type
        .filter(|s| !s.is_empty())
        .is_some_and(|m| m.to_ascii_lowercase().starts_with("image/"))
    {
        return "image file";
    }
    if extension_looks_binary(filename) {
        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if matches!(
            ext.as_deref(),
            Some(
                "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "ico"
                    | "bmp"
                    | "tif"
                    | "tiff"
                    | "heic"
                    | "heif"
                    | "avif"
            )
        ) {
            return "image file";
        }
        return "binary file";
    }
    if content_type
        .filter(|s| !s.is_empty())
        .is_some_and(mime_is_non_text)
    {
        return "binary file";
    }
    "non-text file"
}

/// Status line when preview/diff is blocked for a gist file.
pub fn non_previewable_status(filename: &str, content_type: Option<&str>) -> String {
    let reason = gist_file_non_previewable_reason(filename, content_type);
    format!("cannot preview — {reason} (use o for browser or d to download)")
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
    pub owner_login: String,
    pub fork_of_id: Option<String>,
}

impl GistFile {
    /// A bare row carrying only the identity a sync/diff/upload operation needs —
    /// `gist_id`, `filename`, and `raw_url`. All display/metadata fields default to empty,
    /// so adding a new field to `GistFile` no longer means editing every call site that
    /// builds one of these throwaway targets.
    pub fn for_sync(gist_id: String, filename: String, raw_url: Option<String>) -> Self {
        GistFile {
            gist_id,
            description: String::new(),
            filename,
            public: false,
            updated_at: String::new(),
            created_at: String::new(),
            owner_login: String::new(),
            fork_of_id: None,
            raw_url,
            content_type: None,
            node_id: None,
        }
    }

    pub fn is_owned_by(&self, login: &str) -> bool {
        !login.is_empty() && self.owner_login == login
    }

    pub fn is_fork(&self) -> bool {
        self.fork_of_id.is_some()
    }
}

/// One entry from `gh api /gists/{id}/commits` — a gist revision (newest-first in the API).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GistRevision {
    pub version: String,
    pub committed_at: String,
    pub user: String,
    pub change_status: GistRevisionChangeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GistRevisionChangeStatus {
    pub total: u32,
    pub additions: u32,
    pub deletions: u32,
}

/// Short git-style prefix of a gist revision `version` SHA for display.
pub fn short_sha(version: &str) -> &str {
    if version.len() <= 7 {
        version
    } else {
        &version[..7]
    }
}

/// A single gist comment, mirroring one object from `gh api /gists/{id}/comments`.
/// The body is kept as raw plain text; the TUI wraps it to width (no markdown rendering).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GistComment {
    pub author: String,
    pub created_at: String,
    pub body: String,
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

/// Parse the `YYYY-MM-DDThh:mm:ssZ` UTC form GitHub returns into Unix seconds.
/// Returns `None` on any malformed component. No external date crate (days-from-civil).
pub fn parse_rfc3339_to_unix(s: &str) -> Option<u64> {
    let bytes = s.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let num = |slice: &str| slice.parse::<i64>().ok();
    let year = num(&s[0..4])?;
    let month = num(&s[5..7])?;
    let day = num(&s[8..10])?;
    let hour = num(&s[11..13])?;
    let min = num(&s[14..16])?;
    let sec = num(&s[17..19])?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&min)
        || !(0..=60).contains(&sec)
    {
        return None;
    }
    // days_from_civil (Howard Hinnant): days since 1970-01-01 for a proleptic
    // Gregorian (y, m, d).
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1; // [0,365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era * 146097 + doe - 719468;
    let total = days * 86400 + hour * 3600 + min * 60 + sec;
    u64::try_from(total).ok()
}

/// Compact relative-age label for `secs_ago` seconds in the past.
/// Approximate (month = 30d, year = 365d); a staleness hint, not calendar-exact.
/// Zero or negative (now/future) renders as "now".
pub fn humanize_age(secs_ago: i64) -> String {
    if secs_ago <= 0 {
        return "now".to_string();
    }
    let s = secs_ago;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else if s < 7 * 86_400 {
        format!("{}d", s / 86_400)
    } else if s < 35 * 86_400 {
        format!("{}w", s / (7 * 86_400))
    } else if s < 365 * 86_400 {
        format!("{}mo", s / (30 * 86_400))
    } else {
        format!("{}y", s / (365 * 86_400))
    }
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
                owner_login: file.owner_login.clone(),
                fork_of_id: file.fork_of_id.clone(),
            });
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_status_from_mtime() {
        assert_eq!(sync_status(Some(20), Some(10)), SyncStatus::Push); // local newer
        assert_eq!(sync_status(Some(10), Some(20)), SyncStatus::Pull); // remote newer
        assert_eq!(sync_status(Some(15), Some(15)), SyncStatus::InSync);
        assert_eq!(sync_status(None, Some(10)), SyncStatus::Unknown);
        assert_eq!(sync_status(Some(10), None), SyncStatus::Unknown);
        assert_eq!(sync_status(None, None), SyncStatus::Unknown);
    }

    #[test]
    fn sync_status_icons() {
        assert_eq!(SyncStatus::InSync.icon(), "✓");
        assert_eq!(SyncStatus::Push.icon(), "↑");
        assert_eq!(SyncStatus::Pull.icon(), "↓");
        assert_eq!(SyncStatus::Unknown.icon(), "?");
    }

    #[test]
    fn parse_rfc3339_to_unix_known_values() {
        // 1970-01-01T00:00:00Z == 0
        assert_eq!(parse_rfc3339_to_unix("1970-01-01T00:00:00Z"), Some(0));
        // 2024-01-02T03:04:05Z == 1704164645
        assert_eq!(
            parse_rfc3339_to_unix("2024-01-02T03:04:05Z"),
            Some(1704164645)
        );
    }

    #[test]
    fn parse_rfc3339_to_unix_rejects_garbage() {
        assert_eq!(parse_rfc3339_to_unix(""), None);
        assert_eq!(parse_rfc3339_to_unix("not-a-date"), None);
        assert_eq!(parse_rfc3339_to_unix("2024-13-99T99:99:99Z"), None);
    }

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
            owner_login: "owner".into(),
            fork_of_id: None,

            raw_url: None,
            content_type: None,
            node_id: None,
        }
    }

    #[test]
    fn gist_file_previewable_uses_mime_and_extension() {
        assert!(gist_file_is_text_previewable(
            "notes.md",
            Some("text/markdown")
        ));
        assert!(gist_file_is_text_previewable(
            "data.json",
            Some("application/json")
        ));
        assert!(!gist_file_is_text_previewable(
            "logo.png",
            Some("image/png")
        ));
        assert!(!gist_file_is_text_previewable("photo.jpg", None));
        assert!(gist_file_is_text_previewable("script.py", None));
        assert!(!gist_file_is_text_previewable(
            "script.py",
            Some("image/png")
        ));
    }

    #[test]
    fn non_previewable_reason_labels_images() {
        assert_eq!(
            gist_file_non_previewable_reason("x.png", Some("image/png")),
            "image file"
        );
        assert_eq!(
            gist_file_non_previewable_reason("data.bin", Some("application/octet-stream")),
            "binary file"
        );
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

    #[test]
    fn humanize_age_buckets() {
        assert_eq!(humanize_age(0), "now");
        assert_eq!(humanize_age(-5), "now");
        assert_eq!(humanize_age(5), "5s");
        assert_eq!(humanize_age(59), "59s");
        assert_eq!(humanize_age(60), "1m");
        assert_eq!(humanize_age(59 * 60), "59m");
        assert_eq!(humanize_age(60 * 60), "1h");
        assert_eq!(humanize_age(23 * 3600), "23h");
        assert_eq!(humanize_age(24 * 3600), "1d");
        assert_eq!(humanize_age(6 * 86400), "6d");
        assert_eq!(humanize_age(7 * 86400), "1w");
        assert_eq!(humanize_age(34 * 86400), "4w");
        assert_eq!(humanize_age(35 * 86400), "1mo");
        assert_eq!(humanize_age(359 * 86400), "11mo");
        assert_eq!(humanize_age(365 * 86400), "1y");
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
