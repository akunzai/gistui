use crate::actions::{run_command, CommandPlan, CommandRunner, SystemRunner};
use crate::domain::{GistRevision, GistRevisionChangeStatus};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct GhGist {
    id: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    public: bool,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    created_at: String,
    /// Number of comments on the gist. The REST list response includes this, so the count is
    /// available without a per-gist comments fetch.
    #[serde(default)]
    comments: u32,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    owner: Option<GhCommentUser>,
    #[serde(default)]
    fork_of: Option<GhGistForkOf>,
    /// Present on full gist objects; omitted from the list response (counts default to 0).
    #[serde(default)]
    forks: Vec<serde_json::Value>,
    // The REST API returns `files` as an object keyed by filename. BTreeMap keeps
    // the order deterministic (by filename) for stable display and tests.
    #[serde(default)]
    files: BTreeMap<String, GhGistFile>,
}

#[derive(Debug, Deserialize)]
struct GhGistForkOf {
    id: String,
}

#[derive(Debug, Deserialize)]
struct GhGistFile {
    filename: String,
    #[serde(default)]
    raw_url: Option<String>,
    #[serde(default, rename = "type")]
    content_type: Option<String>,
}

/// Plan for `gh --version` (used to confirm `gh` is installed and runnable).
pub fn gh_version_plan() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec!["--version".into()],
    }
}

/// Plan for `gh auth status` (used to confirm an authenticated session).
pub fn auth_status_plan() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec!["auth".into(), "status".into()],
    }
}

pub fn check_gh_ready() -> Result<()> {
    check_gh_ready_with(&SystemRunner)
}

pub fn check_gh_ready_with(runner: &dyn CommandRunner) -> Result<()> {
    if !runner.run(&gh_version_plan())?.success {
        bail!("gh is installed but did not run successfully");
    }
    if !runner.run(&auth_status_plan())?.success {
        bail!("gh auth status failed; run gh auth login");
    }
    Ok(())
}

/// Parse a `gh api /gists…` JSON array into raw `GhGist` rows. Centralises the
/// `from_str + context` boilerplate shared by the list / comment-count / fork-count /
/// starred-id parsers.
fn parse_gh_gists(raw: &str) -> Result<Vec<GhGist>> {
    serde_json::from_str(raw).context("parse gh gist list JSON")
}

/// Escape a value for safe interpolation inside a GraphQL double-quoted string literal:
/// backslash first, then the quote (GraphQL string literals follow JSON escaping). Keeps a
/// stray `"`/`\` in an API-supplied node id from breaking out of the query. Shared by
/// `forks`' fork-flags query and `stars`' stargazer query (issue #301).
fn escape_graphql_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[derive(Debug, Deserialize)]
struct GhCommentUser {
    #[serde(default)]
    login: String,
}

/// Plan for fetching gist file bytes from a list-response `raw_url` (no auth).
pub fn raw_url_fetch_plan(url: &str) -> CommandPlan {
    CommandPlan {
        program: "curl".into(),
        args: vec!["-sL".into(), url.into()],
    }
}

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

pub fn fetch_gist_commits_json(gist_id: &str) -> Result<String> {
    fetch_gist_commits_json_with(&SystemRunner, gist_id)
}

pub fn fetch_gist_commits_json_with(runner: &dyn CommandRunner, gist_id: &str) -> Result<String> {
    run_command(runner, &gist_commits_plan(gist_id))
}

pub fn fetch_gist_revision_json(gist_id: &str, version: &str) -> Result<String> {
    fetch_gist_revision_json_with(&SystemRunner, gist_id, version)
}

pub fn fetch_gist_revision_json_with(
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
pub fn fetch_revision_file_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<RevisionFileContent> {
    let constructed = build_gist_revision_raw_url(owner_login, gist_id, version, filename);
    match fetch_gist_revision_json_with(runner, gist_id, version) {
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
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<String> {
    fetch_revision_file_text_with(&SystemRunner, gist_id, version, filename, owner_login)
}

pub fn fetch_revision_file_text_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<String> {
    match fetch_revision_file_with(runner, gist_id, version, filename, owner_login)? {
        RevisionFileContent::Present(content) => Ok(content),
        RevisionFileContent::Truncated => {
            bail!("file too large for API preview (>1 MB)")
        }
        RevisionFileContent::Absent => bail!("{filename} not present in this revision"),
    }
}

pub fn fetch_revision_file_text_optional(
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<String> {
    fetch_revision_file_text_optional_with(&SystemRunner, gist_id, version, filename, owner_login)
}

pub fn fetch_revision_file_text_optional_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<String> {
    match fetch_revision_file_with(runner, gist_id, version, filename, owner_login)? {
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

// Per-resource submodules (issue #301). Each is a private `mod` re-exported flat below, so
// existing `crate::gh::X` call sites don't need to know the submodule boundary exists.
mod comments;
pub use comments::{
    comments_total_from_probe, fetch_gist_comments_page, fetch_gist_comments_page_with,
    fetch_gist_comments_probe, fetch_gist_comments_probe_with, gist_comments_page_plan,
    gist_comments_probe_plan, last_page, parse_gist_comment_counts, parse_gist_comments_json,
    parse_link_rel, COMMENTS_PAGE_SIZE,
};

mod forks;
pub use forks::{
    apply_fork_of_ids, collect_gist_fork_counts, collect_gist_fork_counts_with,
    collect_owned_fork_of_ids, collect_owned_fork_of_ids_with, fetch_forked_gist_ids_graphql,
    fetch_forked_gist_ids_graphql_with, fetch_gist_fork_count, fetch_gist_fork_count_with,
    fetch_gist_fork_of_id, fetch_gist_fork_of_id_with, gist_detail_plan,
    gist_fork_flags_graphql_plan, gist_forks_plan, parse_forked_gist_ids_graphql,
    parse_gist_fork_counts,
};

mod gists;
pub use gists::{
    current_user_plan, fetch_current_user_login, fetch_current_user_login_with,
    fetch_gist_file_content, fetch_gist_file_content_with, fetch_gist_list_json,
    fetch_gist_list_json_with, fetch_gist_starred_list_json, fetch_gist_starred_list_json_with,
    gist_list_plan, gist_node_id_map, gist_starred_list_plan, gist_view_plan,
    merge_gist_node_id_maps, parse_gist_list_json, parse_starred_gist_ids,
};

mod stars;
pub use stars::{
    build_stargazer_graphql_query, collect_gist_star_counts, collect_gist_star_counts_with,
    gist_stargazer_graphql_plan, parse_stargazer_counts_graphql,
};

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

        let content = fetch_revision_file_with(&runner, "g1", "sha1", "f.md", "karpathy").unwrap();
        assert_eq!(
            content,
            RevisionFileContent::Present("revision body".into())
        );
        let calls = runner.calls();
        assert_eq!(calls[0], gist_revision_plan("g1", "sha1"));
        assert_eq!(calls[1], raw_url_fetch_plan(&url));
    }
}
