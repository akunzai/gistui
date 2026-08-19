use crate::actions::{CommandPlan, CommandRunner, SystemRunner};
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
    #[serde(default)]
    size: u64,
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
    collect_owned_fork_of_ids, collect_owned_fork_of_ids_with, fetch_forked_gist_ids_graphql_with,
    fetch_gist_fork_count_with, fetch_gist_fork_of_id_with, gist_detail_plan,
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

mod revisions;
pub use revisions::{
    build_gist_revision_raw_url, fetch_gist_commits_json, fetch_gist_commits_json_with,
    fetch_gist_revision_json_with, fetch_revision_file_text, fetch_revision_file_text_optional,
    fetch_revision_file_text_optional_with, fetch_revision_file_text_with,
    fetch_revision_file_with, gist_commits_plan, gist_revision_plan, parse_gist_commits_json,
    revision_file_content, RevisionFileContent,
};

mod stars;
pub use stars::{
    build_stargazer_graphql_query, collect_gist_star_counts, collect_gist_star_counts_with,
    gist_stargazer_graphql_plan, parse_stargazer_counts_graphql,
};
