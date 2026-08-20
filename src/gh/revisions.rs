//! Gist revision history and per-revision file fetch/parse (issue #301).

use super::{raw_url_fetch_plan, GhCommentUser};
use crate::actions::{run_command, CommandPlan, CommandRunner};
use crate::domain::{GistRevision, GistRevisionChangeStatus};
use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// Plan for listing every revision of a gist via the REST API.
pub fn gist_commits_plan(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--paginate".into(),
            format!("/gists/{gist_id}/commits?per_page=100"),
        ],
    }
}

/// Plan for fetching a single gist revision snapshot (files + metadata at that SHA).
pub fn gist_revision_plan(gist_id: &str, version: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec!["api".into(), format!("/gists/{gist_id}/{version}")],
    }
}

pub fn fetch_gist_commits_json(runner: &dyn CommandRunner, gist_id: &str) -> Result<String> {
    run_command(runner, &gist_commits_plan(gist_id))
}

pub fn fetch_gist_revision_json(
    runner: &dyn CommandRunner,
    gist_id: &str,
    version: &str,
) -> Result<String> {
    run_command(runner, &gist_revision_plan(gist_id, version))
}

/// Canonical gist revision raw URL (`owner` form works for large third-party gists).
pub fn build_gist_revision_raw_url(
    owner_login: &str,
    gist_id: &str,
    version: &str,
    filename: &str,
) -> String {
    if owner_login.is_empty() {
        format!("https://gist.githubusercontent.com/{gist_id}/raw/{version}/{filename}")
    } else {
        format!(
            "https://gist.githubusercontent.com/{owner_login}/{gist_id}/raw/{version}/{filename}"
        )
    }
}

fn revision_file_entry<'a>(
    files: &'a serde_json::Map<String, serde_json::Value>,
    filename: &str,
) -> Option<&'a serde_json::Value> {
    if let Some(entry) = files.get(filename) {
        return Some(entry);
    }
    files
        .values()
        .find(|entry| entry.get("filename").and_then(|f| f.as_str()) == Some(filename))
}

fn revision_entry_raw_url(entry: &serde_json::Value) -> Option<String> {
    entry
        .get("raw_url")
        .and_then(|u| u.as_str())
        .filter(|u| !u.is_empty())
        .map(str::to_string)
}

fn fetch_revision_file_via_raw_url(
    runner: &dyn CommandRunner,
    url: &str,
) -> Result<RevisionFileContent> {
    run_command(runner, &raw_url_fetch_plan(url)).map(RevisionFileContent::Present)
}

/// Fetch one file at a gist revision SHA. Uses the revision API when it works; on HTTP
/// failures or truncated payloads, falls back to the revision `raw_url` or the canonical
/// `gist.githubusercontent.com/.../raw/{sha}/{file}` URL.
pub fn fetch_revision_file(
    runner: &dyn CommandRunner,
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<RevisionFileContent> {
    let constructed = build_gist_revision_raw_url(owner_login, gist_id, version, filename);
    match fetch_gist_revision_json(runner, gist_id, version) {
        Ok(raw) => {
            let root: serde_json::Value =
                serde_json::from_str(&raw).context("parse gh gist revision JSON")?;
            let Some(files) = root.get("files").and_then(|f| f.as_object()) else {
                return Ok(RevisionFileContent::Absent);
            };
            let Some(entry) = revision_file_entry(files, filename) else {
                return Ok(RevisionFileContent::Absent);
            };
            match classify_revision_file(entry)? {
                RevisionFileContent::Present(content) => Ok(RevisionFileContent::Present(content)),
                RevisionFileContent::Truncated => revision_entry_raw_url(entry)
                    .map(|url| fetch_revision_file_via_raw_url(runner, &url))
                    .unwrap_or_else(|| fetch_revision_file_via_raw_url(runner, &constructed)),
                RevisionFileContent::Absent => revision_entry_raw_url(entry)
                    .map(|url| fetch_revision_file_via_raw_url(runner, &url))
                    .unwrap_or(Ok(RevisionFileContent::Absent)),
            }
        }
        Err(api_err) => fetch_revision_file_via_raw_url(runner, &constructed).with_context(|| {
            format!("revision API failed ({api_err}); raw URL fallback also failed")
        }),
    }
}

pub fn fetch_revision_file_text(
    runner: &dyn CommandRunner,
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<String> {
    match fetch_revision_file(runner, gist_id, version, filename, owner_login)? {
        RevisionFileContent::Present(content) => Ok(content),
        RevisionFileContent::Truncated => {
            bail!("file too large for API preview (>1 MB)")
        }
        RevisionFileContent::Absent => bail!("{filename} not present in this revision"),
    }
}

pub fn fetch_revision_file_text_optional(
    runner: &dyn CommandRunner,
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<String> {
    match fetch_revision_file(runner, gist_id, version, filename, owner_login)? {
        RevisionFileContent::Present(content) => Ok(content),
        RevisionFileContent::Truncated => bail!("file too large for API preview (>1 MB)"),
        RevisionFileContent::Absent => Ok(String::new()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionFileContent {
    Present(String),
    Truncated,
    Absent,
}

#[derive(Debug, Deserialize)]
struct GhGistCommit {
    version: String,
    #[serde(default)]
    committed_at: String,
    #[serde(default)]
    user: Option<GhCommentUser>,
    #[serde(default)]
    change_status: GhGistChangeStatus,
}

#[derive(Debug, Deserialize, Default)]
struct GhGistChangeStatus {
    #[serde(default)]
    total: u32,
    #[serde(default)]
    additions: u32,
    #[serde(default)]
    deletions: u32,
}

pub fn parse_gist_commits_json(raw: &str) -> Result<Vec<GistRevision>> {
    let commits: Vec<GhGistCommit> =
        serde_json::from_str(raw).context("parse gh gist commits JSON")?;
    Ok(commits
        .into_iter()
        .map(|c| GistRevision {
            version: c.version,
            committed_at: c.committed_at,
            user: c
                .user
                .map(|u| u.login)
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "(unknown)".to_string()),
            change_status: GistRevisionChangeStatus {
                total: c.change_status.total,
                additions: c.change_status.additions,
                deletions: c.change_status.deletions,
            },
        })
        .collect())
}

/// Extract one file's text from a revision snapshot (`GET /gists/{id}/{sha}`).
pub fn revision_file_content(raw: &str, filename: &str) -> Result<RevisionFileContent> {
    let root: serde_json::Value =
        serde_json::from_str(raw).context("parse gh gist revision JSON")?;
    let Some(files) = root.get("files").and_then(|f| f.as_object()) else {
        return Ok(RevisionFileContent::Absent);
    };
    if let Some(entry) = files.get(filename) {
        return classify_revision_file(entry);
    }
    for entry in files.values() {
        if entry.get("filename").and_then(|f| f.as_str()) == Some(filename) {
            return classify_revision_file(entry);
        }
    }
    Ok(RevisionFileContent::Absent)
}

fn classify_revision_file(entry: &serde_json::Value) -> Result<RevisionFileContent> {
    if entry.get("truncated").and_then(|t| t.as_bool()) == Some(true) {
        return Ok(RevisionFileContent::Truncated);
    }
    match entry.get("content").and_then(|c| c.as_str()) {
        Some(content) => Ok(RevisionFileContent::Present(content.to_string())),
        None => Ok(RevisionFileContent::Absent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gist_commits_into_revisions() {
        let raw = include_str!("../../tests/fixtures/gh/gist-commits.json");
        let revisions = parse_gist_commits_json(raw).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].version, "abc111def222333444");
        assert_eq!(revisions[0].user, "akunzai");
        assert_eq!(revisions[0].change_status.additions, 2);
        assert_eq!(revisions[0].change_status.deletions, 1);
        assert_eq!(revisions[1].committed_at, "2026-06-01T08:00:00Z");
    }

    #[test]
    fn revision_file_content_reads_present_and_truncated() {
        let raw = include_str!("../../tests/fixtures/gh/gist-revision.json");
        match revision_file_content(raw, "settings.json").unwrap() {
            RevisionFileContent::Present(content) => {
                assert!(content.contains("\"old\": true"));
            }
            other => panic!("expected Present, got {other:?}"),
        }
        let truncated = r#"{"files":{"a.txt":{"filename":"a.txt","truncated":true}}}"#;
        assert_eq!(
            revision_file_content(truncated, "a.txt").unwrap(),
            RevisionFileContent::Truncated
        );
        assert_eq!(
            revision_file_content(truncated, "missing.txt").unwrap(),
            RevisionFileContent::Absent
        );
    }

    #[test]
    fn build_gist_revision_raw_url_includes_owner_when_known() {
        let url = build_gist_revision_raw_url("karpathy", "abc123", "deadbeef", "notes.md");
        assert_eq!(
            url,
            "https://gist.githubusercontent.com/karpathy/abc123/raw/deadbeef/notes.md"
        );
    }

    #[test]
    fn fetch_revision_file_falls_back_when_revision_api_fails() {
        use crate::actions::test_support::SeqRunner;
        use crate::actions::CommandOutput;

        let url = build_gist_revision_raw_url("karpathy", "g1", "sha1", "f.md");
        let runner = SeqRunner::new(vec![
            CommandOutput {
                success: false,
                stdout: String::new(),
                stderr: "HTTP 502".into(),
            },
            CommandOutput {
                success: true,
                stdout: "revision body".into(),
                stderr: String::new(),
            },
        ]);

        let content = fetch_revision_file(&runner, "g1", "sha1", "f.md", "karpathy").unwrap();
        assert_eq!(
            content,
            RevisionFileContent::Present("revision body".into())
        );
        let calls = runner.calls();
        assert_eq!(calls[0], gist_revision_plan("g1", "sha1"));
        assert_eq!(calls[1], raw_url_fetch_plan(&url));
    }
}
