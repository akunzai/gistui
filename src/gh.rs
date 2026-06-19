use crate::actions::{run_command, CommandPlan, CommandRunner, SystemRunner};
use crate::domain::{GistComment, GistFile, GistRevision, GistRevisionChangeStatus};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

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
    owner: Option<GhCommentUser>,
    #[serde(default)]
    fork_of: Option<GhGistForkOf>,
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

/// Plan for listing every gist via the REST API.
///
/// `gh gist list` has no `--json` flag; use the REST API with `--paginate` so
/// accounts with more than 100 gists are fully retrieved. gh concatenates all
/// pages into a single JSON array, which `parse_gist_list_json` already handles.
pub fn gist_list_plan() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--paginate".into(),
            "/gists?per_page=100".into(),
        ],
    }
}

/// Plan for listing the authenticated user's starred gists.
pub fn gist_starred_list_plan() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--paginate".into(),
            "/gists/starred?per_page=100".into(),
        ],
    }
}

/// Plan for the authenticated user's login (ownership checks).
pub fn current_user_plan() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec!["api".into(), "user".into(), "--jq".into(), ".login".into()],
    }
}

/// Plan for fetching a single gist file's raw content.
pub fn gist_view_plan(gist_id: &str, filename: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "gist".into(),
            "view".into(),
            gist_id.to_string(),
            "--filename".into(),
            filename.to_string(),
            "--raw".into(),
        ],
    }
}

/// Plan for fetching a gist's comments via the REST API.
pub fn gist_comments_plan(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--paginate".into(),
            format!("/gists/{gist_id}/comments?per_page=100"),
        ],
    }
}

pub fn fetch_gist_comments_json(gist_id: &str) -> Result<String> {
    fetch_gist_comments_json_with(&SystemRunner, gist_id)
}

pub fn fetch_gist_comments_json_with(runner: &dyn CommandRunner, gist_id: &str) -> Result<String> {
    run_command(runner, &gist_comments_plan(gist_id))
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

pub fn parse_gist_list_json(raw: &str) -> Result<Vec<GistFile>> {
    let gists: Vec<GhGist> = serde_json::from_str(raw).context("parse gh gist list JSON")?;
    let mut files = Vec::new();

    for gist in gists {
        let description = gist.description.unwrap_or_default();
        let owner_login = gist
            .owner
            .map(|u| u.login)
            .filter(|l| !l.is_empty())
            .unwrap_or_default();
        let fork_of_id = gist.fork_of.map(|f| f.id);
        for file in gist.files.into_values() {
            files.push(GistFile {
                gist_id: gist.id.clone(),
                description: description.clone(),
                filename: file.filename,
                public: gist.public,
                updated_at: gist.updated_at.clone(),
                created_at: gist.created_at.clone(),
                owner_login: owner_login.clone(),
                fork_of_id: fork_of_id.clone(),
            });
        }
    }

    Ok(files)
}

/// Map each gist id to its comment count, parsed from the same gist-list JSON. The count rides
/// along in the list response, so this needs no extra `gh` call.
pub fn parse_gist_comment_counts(raw: &str) -> Result<HashMap<String, u32>> {
    let gists: Vec<GhGist> = serde_json::from_str(raw).context("parse gh gist list JSON")?;
    Ok(gists.into_iter().map(|g| (g.id, g.comments)).collect())
}

#[derive(Debug, Deserialize)]
struct GhComment {
    #[serde(default)]
    user: Option<GhCommentUser>,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Deserialize)]
struct GhCommentUser {
    #[serde(default)]
    login: String,
}

pub fn parse_gist_comments_json(raw: &str) -> Result<Vec<GistComment>> {
    let comments: Vec<GhComment> =
        serde_json::from_str(raw).context("parse gh gist comments JSON")?;
    Ok(comments
        .into_iter()
        .map(|c| GistComment {
            author: c
                .user
                .map(|u| u.login)
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "(unknown)".to_string()),
            created_at: c.created_at,
            body: c.body,
        })
        .collect())
}

pub fn fetch_gist_list_json() -> Result<String> {
    fetch_gist_list_json_with(&SystemRunner)
}

pub fn fetch_gist_list_json_with(runner: &dyn CommandRunner) -> Result<String> {
    run_command(runner, &gist_list_plan())
}

pub fn fetch_gist_starred_list_json() -> Result<String> {
    fetch_gist_starred_list_json_with(&SystemRunner)
}

pub fn fetch_gist_starred_list_json_with(runner: &dyn CommandRunner) -> Result<String> {
    run_command(runner, &gist_starred_list_plan())
}

pub fn fetch_current_user_login() -> Result<String> {
    fetch_current_user_login_with(&SystemRunner)
}

pub fn fetch_current_user_login_with(runner: &dyn CommandRunner) -> Result<String> {
    let raw = run_command(runner, &current_user_plan())?;
    let login = raw.trim().trim_matches('"').to_string();
    if login.is_empty() {
        anyhow::bail!("empty user login from gh api user");
    }
    Ok(login)
}

/// Unique gist ids from a parsed gist-list JSON payload.
pub fn parse_starred_gist_ids(raw: &str) -> Result<std::collections::HashSet<String>> {
    let gists: Vec<GhGist> = serde_json::from_str(raw).context("parse gh gist list JSON")?;
    Ok(gists.into_iter().map(|g| g.id).collect())
}

pub fn fetch_gist_file_content(gist_id: &str, filename: &str) -> Result<String> {
    fetch_gist_file_content_with(&SystemRunner, gist_id, filename)
}

pub fn fetch_gist_file_content_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
    filename: &str,
) -> Result<String> {
    run_command(runner, &gist_view_plan(gist_id, filename))
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
    fn parses_gist_list_into_file_rows() {
        let raw = include_str!("../tests/fixtures/gh/gist-list.json");
        let files = parse_gist_list_json(raw).unwrap();

        assert_eq!(files.len(), 3);
        // Files within a gist are ordered deterministically by filename.
        assert_eq!(files[0].gist_id, "abc123");
        assert_eq!(files[0].filename, "settings.json");
        assert_eq!(files[0].description, "claude config");
        assert!(!files[0].public);
        assert_eq!(files[0].owner_login, "akunzai");
        assert_eq!(files[1].filename, "statusline.sh");
        let notes = files.iter().find(|f| f.filename == "notes.md").unwrap();
        assert_eq!(notes.fork_of_id.as_deref(), Some("upstream99"));
    }

    #[test]
    fn parses_starred_gist_ids() {
        let raw = include_str!("../tests/fixtures/gh/gist-starred.json");
        let ids = parse_starred_gist_ids(raw).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains("star111"));
        let files = parse_gist_list_json(raw).unwrap();
        assert_eq!(files[0].owner_login, "otherdev");
    }

    #[test]
    fn null_description_parses_as_empty_string() {
        let raw = include_str!("../tests/fixtures/gh/gist-list.json");
        let files = parse_gist_list_json(raw).unwrap();

        let notes = files.iter().find(|f| f.filename == "notes.md").unwrap();
        assert_eq!(notes.description, "");
        assert!(notes.public);
    }

    #[test]
    fn parses_gist_commits_into_revisions() {
        let raw = include_str!("../tests/fixtures/gh/gist-commits.json");
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
        let raw = include_str!("../tests/fixtures/gh/gist-revision.json");
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
    fn parses_comment_counts_defaulting_to_zero() {
        let raw = include_str!("../tests/fixtures/gh/gist-list.json");
        let counts = parse_gist_comment_counts(raw).unwrap();

        assert_eq!(counts.get("abc123").copied(), Some(2));
        // The gist with no `comments` field falls back to 0 via `#[serde(default)]`.
        assert_eq!(counts.get("def456").copied(), Some(0));
    }
}
