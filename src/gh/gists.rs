//! Gist listing, node-id mapping, and file-content fetch (issue #301).

use super::{parse_gh_gists, raw_url_fetch_plan};
use crate::actions::{run_command, CommandPlan, CommandRunner, SystemRunner};
use crate::domain::GistFile;
use anyhow::{Context, Result};
use std::collections::HashMap;

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

pub fn parse_gist_list_json(raw: &str) -> Result<Vec<GistFile>> {
    let gists = parse_gh_gists(raw)?;
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
                raw_url: file.raw_url.clone(),
                content_type: file.content_type.clone(),
                node_id: gist.node_id.clone(),
            });
        }
    }

    Ok(files)
}

/// Unique `gist_id → node_id` pairs from flat gist rows (first wins).
pub fn gist_node_id_map(files: &[GistFile]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for file in files {
        if let Some(nid) = file.node_id.as_ref().filter(|s| !s.is_empty()) {
            map.entry(file.gist_id.clone())
                .or_insert_with(|| nid.clone());
        }
    }
    map
}

/// Merge node-id maps from owned and starred gist rows.
pub fn merge_gist_node_id_maps(
    owned: &[GistFile],
    starred: &[GistFile],
) -> HashMap<String, String> {
    let mut map = gist_node_id_map(owned);
    for (id, nid) in gist_node_id_map(starred) {
        map.entry(id).or_insert(nid);
    }
    map
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
    let gists = parse_gh_gists(raw)?;
    Ok(gists.into_iter().map(|g| g.id).collect())
}

pub fn fetch_gist_file_content(
    gist_id: &str,
    filename: &str,
    raw_url: Option<&str>,
) -> Result<String> {
    fetch_gist_file_content_with(&SystemRunner, gist_id, filename, raw_url)
}

pub fn fetch_gist_file_content_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
    filename: &str,
    raw_url: Option<&str>,
) -> Result<String> {
    match run_command(runner, &gist_view_plan(gist_id, filename)) {
        Ok(content) => Ok(content),
        Err(primary) => {
            if let Some(url) = raw_url.filter(|u| !u.is_empty()) {
                run_command(runner, &raw_url_fetch_plan(url))
                    .with_context(|| format!("{primary}; raw_url fallback also failed"))
            } else {
                Err(primary)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gist_list_into_file_rows() {
        let raw = include_str!("../../tests/fixtures/gh/gist-list.json");
        let files = parse_gist_list_json(raw).unwrap();

        assert_eq!(files.len(), 3);
        // Files within a gist are ordered deterministically by filename.
        assert_eq!(files[0].gist_id, "abc123");
        assert_eq!(files[0].filename, "settings.json");
        assert_eq!(files[0].description, "claude config");
        assert!(!files[0].public);
        assert_eq!(files[0].owner_login, "akunzai");
        assert_eq!(files[0].content_type.as_deref(), Some("application/json"));
        assert_eq!(files[1].filename, "statusline.sh");
        assert_eq!(files[1].content_type.as_deref(), Some("text/x-shellscript"));
        let notes = files.iter().find(|f| f.filename == "notes.md").unwrap();
        assert_eq!(notes.fork_of_id.as_deref(), Some("upstream99"));
    }

    #[test]
    fn parses_starred_gist_ids() {
        let raw = include_str!("../../tests/fixtures/gh/gist-starred.json");
        let ids = parse_starred_gist_ids(raw).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains("star111"));
        let files = parse_gist_list_json(raw).unwrap();
        assert_eq!(files[0].owner_login, "otherdev");
    }

    #[test]
    fn null_description_parses_as_empty_string() {
        let raw = include_str!("../../tests/fixtures/gh/gist-list.json");
        let files = parse_gist_list_json(raw).unwrap();

        let notes = files.iter().find(|f| f.filename == "notes.md").unwrap();
        assert_eq!(notes.description, "");
        assert!(notes.public);
    }

    #[test]
    fn fetch_gist_file_content_falls_back_to_raw_url() {
        use crate::actions::test_support::SeqRunner;
        use crate::actions::CommandOutput;

        let url = "https://gist.githubusercontent.com/u/id/raw/hash/file.md";
        let runner = SeqRunner::new(vec![
            CommandOutput {
                success: false,
                stdout: String::new(),
                stderr: "HTTP 502".into(),
            },
            CommandOutput {
                success: true,
                stdout: "big content".into(),
                stderr: String::new(),
            },
        ]);

        let content = fetch_gist_file_content_with(&runner, "id", "file.md", Some(url)).unwrap();
        assert_eq!(content, "big content");
        let calls = runner.calls();
        assert_eq!(calls[0], gist_view_plan("id", "file.md"));
        assert_eq!(calls[1], raw_url_fetch_plan(url));
    }
}
