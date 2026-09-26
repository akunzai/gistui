//! Gist listing, node-id mapping, and file-content fetch (issue #301).

use super::{fetch_raw_text, parse_gh_gists};
use crate::actions::{run_command, CommandPlan, CommandRunner};
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

/// Plan for one gist's REST record (`GET /gists/{id}`). It carries every file's exact content,
/// unlike `gh gist view --raw`, which appends a `\n` to a file that lacks one (issue #471).
pub fn gist_get_plan(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec!["api".into(), format!("/gists/{gist_id}")],
    }
}

/// One file's content in a `GET /gists/{id}` record: the exact text, or — when the API
/// truncated it — the `raw_url` that serves it whole.
#[derive(Debug, PartialEq, Eq)]
enum GistFileBody {
    Text(String),
    Truncated { raw_url: String },
}

fn gist_file_body(raw: &str, filename: &str) -> Result<GistFileBody> {
    let gist: serde_json::Value = serde_json::from_str(raw).context("parse gist JSON")?;
    let file = gist
        .get("files")
        .and_then(|files| files.get(filename))
        .with_context(|| format!("gist has no file named {filename}"))?;
    if file.get("truncated").and_then(|t| t.as_bool()) == Some(true) {
        let raw_url = file
            .get("raw_url")
            .and_then(|u| u.as_str())
            .with_context(|| format!("{filename} is truncated and has no raw_url"))?;
        return Ok(GistFileBody::Truncated {
            raw_url: raw_url.to_string(),
        });
    }
    let content = file
        .get("content")
        .and_then(|c| c.as_str())
        .with_context(|| format!("{filename} has no content"))?;
    Ok(GistFileBody::Text(content.to_string()))
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
            // Exhaustive by design (issue #379): a new GistFile field must fail here, not default.
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
                size: file.size,
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

pub fn fetch_gist_list_json(runner: &dyn CommandRunner) -> Result<String> {
    run_command(runner, &gist_list_plan())
}

pub fn fetch_gist_starred_list_json(runner: &dyn CommandRunner) -> Result<String> {
    run_command(runner, &gist_starred_list_plan())
}

pub fn fetch_current_user_login(runner: &dyn CommandRunner) -> Result<String> {
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
    runner: &dyn CommandRunner,
    gist_id: &str,
    filename: &str,
    raw_url: Option<&str>,
) -> Result<String> {
    let body =
        run_command(runner, &gist_get_plan(gist_id)).and_then(|raw| gist_file_body(&raw, filename));
    match body {
        Ok(GistFileBody::Text(content)) => Ok(content),
        Ok(GistFileBody::Truncated { raw_url }) => fetch_raw_text(runner, &raw_url),
        Err(primary) => {
            if let Some(url) = raw_url.filter(|u| !u.is_empty()) {
                fetch_raw_text(runner, url)
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

    /// Issue #507: raw bytes that are not UTF-8 fail the fetch instead of being decoded lossily
    /// into content that would be diffed, written, or hashed as a pin baseline.
    #[test]
    fn a_raw_file_that_is_not_utf8_fails() {
        struct Latin1;
        impl CommandRunner for Latin1 {
            fn run(&self, _: &CommandPlan) -> Result<crate::actions::CommandOutput> {
                Ok(crate::actions::CommandOutput {
                    success: true,
                    stdout: r#"{"files":{"a.txt":{"truncated":true,"raw_url":"https://r/a.txt"}}}"#
                        .into(),
                    stderr: String::new(),
                })
            }
            fn fetch_raw(&self, _: &str) -> Result<Vec<u8>> {
                Ok(b"caf\xe9".to_vec())
            }
        }

        let error = fetch_gist_file_content(&Latin1, "id", "a.txt", None).unwrap_err();

        assert_eq!(error.to_string(), "https://r/a.txt is not UTF-8 text");
    }

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
        assert_eq!(files[0].size, 42);
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

        let content = fetch_gist_file_content(&runner, "id", "file.md", Some(url)).unwrap();
        assert_eq!(content, "big content");
        let calls = runner.calls();
        assert_eq!(calls[0], gist_get_plan("id"));
        assert_eq!(calls[1], crate::actions::test_support::raw_get(url));
    }

    fn ok(stdout: &str) -> crate::actions::CommandOutput {
        crate::actions::CommandOutput::ok(stdout)
    }

    /// Issue #471: the file's exact text comes from the gist record, so a file without a final
    /// newline does not gain one.
    #[test]
    fn fetch_gist_file_content_keeps_a_missing_final_newline() {
        use crate::actions::test_support::SeqRunner;

        let runner = SeqRunner::new(vec![ok(
            r#"{"files":{"a.txt":{"content":"no newline","truncated":false},"b.txt":{"content":"x\n"}}}"#,
        )]);

        let content = fetch_gist_file_content(&runner, "id", "a.txt", None).unwrap();
        assert_eq!(content, "no newline");
        assert_eq!(runner.calls(), vec![gist_get_plan("id")]);
    }

    #[test]
    fn fetch_gist_file_content_fetches_a_truncated_file_from_its_fresh_raw_url() {
        use crate::actions::test_support::SeqRunner;

        let fresh = "https://gist.githubusercontent.com/u/id/raw/new/a.txt";
        let stale = "https://gist.githubusercontent.com/u/id/raw/old/a.txt";
        let runner = SeqRunner::new(vec![
            ok(&format!(
                r#"{{"files":{{"a.txt":{{"content":"part","truncated":true,"raw_url":"{fresh}"}}}}}}"#
            )),
            ok("whole"),
        ]);

        let content = fetch_gist_file_content(&runner, "id", "a.txt", Some(stale)).unwrap();
        assert_eq!(content, "whole");
        assert_eq!(
            runner.calls(),
            vec![
                gist_get_plan("id"),
                crate::actions::test_support::raw_get(fresh)
            ]
        );
    }

    #[test]
    fn fetch_gist_file_content_falls_back_when_the_record_lacks_the_file() {
        use crate::actions::test_support::SeqRunner;

        let url = "https://gist.githubusercontent.com/u/id/raw/hash/a.txt";
        let runner = SeqRunner::new(vec![ok(r#"{"files":{}}"#), ok("from raw")]);

        let content = fetch_gist_file_content(&runner, "id", "a.txt", Some(url)).unwrap();
        assert_eq!(content, "from raw");
    }
}
