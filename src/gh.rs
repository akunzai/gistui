use crate::domain::GistFile;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::process::Command;

#[derive(Debug, Deserialize)]
struct GhGist {
    id: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    public: bool,
    #[serde(default)]
    updated_at: String,
    // The REST API returns `files` as an object keyed by filename. BTreeMap keeps
    // the order deterministic (by filename) for stable display and tests.
    #[serde(default)]
    files: BTreeMap<String, GhGistFile>,
}

#[derive(Debug, Deserialize)]
struct GhGistFile {
    filename: String,
}

pub fn check_gh_ready() -> Result<()> {
    let version = Command::new("gh")
        .arg("--version")
        .output()
        .context("run gh --version")?;
    if !version.status.success() {
        bail!("gh is installed but did not run successfully");
    }

    let auth = Command::new("gh")
        .args(["auth", "status"])
        .output()
        .context("run gh auth status")?;
    if !auth.status.success() {
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
            });
        }
    }

    Ok(files)
}

pub fn fetch_gist_list_json() -> Result<String> {
    // `gh gist list` has no `--json` flag; the REST API returns the structured
    // shape parsed by `parse_gist_list_json` (files keyed by name, snake_case fields).
    let output = Command::new("gh")
        .args(["api", "/gists?per_page=100"])
        .output()
        .context("run gh api /gists")?;

    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(String::from_utf8(output.stdout)?)
}

pub fn fetch_gist_file_content(gist_id: &str, filename: &str) -> Result<String> {
    let output = Command::new("gh")
        .args(["gist", "view", gist_id, "--filename", filename, "--raw"])
        .output()
        .context("run gh gist view")?;

    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(String::from_utf8(output.stdout)?)
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
