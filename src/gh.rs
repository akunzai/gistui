use crate::actions::{run_command, CommandPlan, CommandRunner, SystemRunner};
use crate::domain::GistFile;
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
    // The REST API returns `files` as an object keyed by filename. BTreeMap keeps
    // the order deterministic (by filename) for stable display and tests.
    #[serde(default)]
    files: BTreeMap<String, GhGistFile>,
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
        for file in gist.files.into_values() {
            files.push(GistFile {
                gist_id: gist.id.clone(),
                description: description.clone(),
                filename: file.filename,
                public: gist.public,
                updated_at: gist.updated_at.clone(),
                created_at: gist.created_at.clone(),
            });
        }
    }

    Ok(files)
}

pub fn fetch_gist_list_json() -> Result<String> {
    fetch_gist_list_json_with(&SystemRunner)
}

pub fn fetch_gist_list_json_with(runner: &dyn CommandRunner) -> Result<String> {
    run_command(runner, &gist_list_plan())
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
        assert_eq!(files[1].filename, "statusline.sh");
    }

    #[test]
    fn null_description_parses_as_empty_string() {
        let raw = include_str!("../tests/fixtures/gh/gist-list.json");
        let files = parse_gist_list_json(raw).unwrap();

        let notes = files.iter().find(|f| f.filename == "notes.md").unwrap();
        assert_eq!(notes.description, "");
        assert!(notes.public);
    }
}
