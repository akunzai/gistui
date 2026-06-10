use crate::config::{save_config, AppConfig};
use crate::domain::{GistFile, PinnedMapping, SyncDirection};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    Upload {
        local_path: PathBuf,
        target: GistFile,
    },
    Download {
        source: GistFile,
        local_path: PathBuf,
    },
    Create {
        local_path: PathBuf,
        filename: String,
        public: bool,
    },
    Delete {
        gist_id: String,
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedAction {
    pub action: PendingAction,
}

pub fn confirm_action(
    action: Option<PendingAction>,
    diff_previewed: bool,
) -> Option<ConfirmedAction> {
    if diff_previewed {
        action.map(|action| ConfirmedAction { action })
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPlan {
    pub program: String,
    pub args: Vec<String>,
}

/// The captured result of running a [`CommandPlan`], independent of how it was
/// produced. Mirrors the fields of `std::process::Output` the app actually uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The injectable boundary for every external command (`gh`) the app shells out
/// to. Production uses [`SystemRunner`]; tests supply a fake so integration tests
/// exercise command planning, success/failure handling, and output parsing
/// without touching the network or requiring `gh`.
pub trait CommandRunner {
    fn run(&self, plan: &CommandPlan) -> Result<CommandOutput>;
}

/// The real boundary: spawns the planned program via `std::process::Command`.
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, plan: &CommandPlan) -> Result<CommandOutput> {
        let output = Command::new(&plan.program)
            .args(&plan.args)
            .output()
            .with_context(|| format!("run {} {}", plan.program, plan.args.join(" ")))?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Runs a planned command through `runner`, returning its stdout on success or
/// the stderr as an error on a non-zero exit. This is the shared execution path
/// for both write actions (`gh gist edit/create/delete`) and the read fetches in
/// the `gh` module.
pub fn run_command(runner: &dyn CommandRunner, plan: &CommandPlan) -> Result<String> {
    let output = runner.run(plan)?;
    if !output.success {
        bail!("{}", output.stderr);
    }
    Ok(output.stdout)
}

pub fn upload_command(local_path: &Path, target: &GistFile) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "gist".into(),
            "edit".into(),
            target.gist_id.clone(),
            "--filename".into(),
            target.filename.clone(),
            local_path.display().to_string(),
        ],
    }
}

pub fn upload_add_command(local_path: &Path, gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "gist".into(),
            "edit".into(),
            gist_id.to_string(),
            "--add".into(),
            local_path.display().to_string(),
        ],
    }
}

pub fn open_browser_command(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "gist".into(),
            "view".into(),
            gist_id.to_string(),
            "--web".into(),
        ],
    }
}

pub fn open_repo_browser_command() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "repo".into(),
            "view".into(),
            "akunzai/gistui".into(),
            "--web".into(),
        ],
    }
}

pub fn create_command(local_path: &Path, public: bool, description: &str) -> CommandPlan {
    let mut args = vec![
        "gist".into(),
        "create".into(),
        local_path.display().to_string(),
    ];
    if public {
        args.push("--public".into());
    }
    if !description.is_empty() {
        args.push("--desc".into());
        args.push(description.to_string());
    }
    CommandPlan {
        program: "gh".into(),
        args,
    }
}

pub fn remove_file_command(gist_id: &str, filename: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "gist".into(),
            "edit".into(),
            gist_id.to_string(),
            "--remove".into(),
            filename.to_string(),
        ],
    }
}

/// Updates only the gist description via the REST API.
///
/// `gh gist edit --desc` cannot be used here: with no `--add`/`--remove` it still
/// drops into gh's interactive content editor ($EDITOR on a temp file), which is
/// wrong inside the TUI. The PATCH endpoint sets the description non-interactively.
/// `-f` (raw string field) keeps arbitrary description text from being type-coerced.
pub fn edit_description_command(gist_id: &str, description: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--method".into(),
            "PATCH".into(),
            format!("/gists/{gist_id}"),
            "-f".into(),
            format!("description={description}"),
        ],
    }
}

pub fn delete_command(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "gist".into(),
            "delete".into(),
            "--yes".into(),
            gist_id.to_string(),
        ],
    }
}

pub fn execute_command(plan: &CommandPlan) -> Result<String> {
    run_command(&SystemRunner, plan)
}

pub fn execute_download(local_path: &Path, content: &str, overwrite_confirmed: bool) -> Result<()> {
    if local_path.exists() && !overwrite_confirmed {
        bail!(
            "refusing to overwrite {} without confirmation",
            local_path.display()
        );
    }
    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(local_path, content).with_context(|| format!("write {}", local_path.display()))
}

pub fn pin_mapping(
    config_path: &Path,
    mut config: AppConfig,
    local_path: &Path,
    target: &GistFile,
    direction: Option<SyncDirection>,
    last_seen_hash: Option<String>,
) -> Result<AppConfig> {
    config
        .pinned
        .retain(|mapping| mapping.local_path != local_path);
    config.pinned.push(PinnedMapping {
        local_path: local_path.to_path_buf(),
        gist_id: target.gist_id.clone(),
        gist_filename: target.filename.clone(),
        direction,
        last_seen_hash,
    });
    save_config(config_path, &config)?;
    Ok(config)
}

pub fn unpin_mapping(
    config_path: &Path,
    mut config: AppConfig,
    local_path: &Path,
) -> Result<AppConfig> {
    config
        .pinned
        .retain(|mapping| mapping.local_path != local_path);
    save_config(config_path, &config)?;
    Ok(config)
}

/// Removes exactly the first entry whose `local_path` and `gist_id` both match.
/// Used by the pins view to unpin a single row without affecting sibling entries.
pub fn unpin_mapping_exact(
    config_path: &Path,
    mut config: AppConfig,
    local_path: &Path,
    gist_id: &str,
) -> Result<AppConfig> {
    let mut removed = false;
    config.pinned.retain(|m| {
        if !removed && m.local_path == local_path && m.gist_id == gist_id {
            removed = true;
            false
        } else {
            true
        }
    });
    save_config(config_path, &config)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{load_config, AppConfig};

    fn gist_file() -> GistFile {
        GistFile {
            gist_id: "abc123".into(),
            description: "config".into(),
            filename: "settings.json".into(),
            public: false,
            updated_at: "2026-06-08T00:00:00Z".into(),
            created_at: "2026-06-08T00:00:00Z".into(),
        }
    }

    #[test]
    fn cannot_confirm_without_diff_preview() {
        let action = PendingAction::Upload {
            local_path: PathBuf::from("/tmp/settings.json"),
            target: gist_file(),
        };

        assert_eq!(confirm_action(Some(action), false), None);
    }

    #[test]
    fn can_confirm_after_diff_preview() {
        let action = PendingAction::Download {
            source: gist_file(),
            local_path: PathBuf::from("/tmp/settings.json"),
        };

        assert!(confirm_action(Some(action), true).is_some());
    }

    #[test]
    fn no_action_yields_none_even_after_preview() {
        assert_eq!(confirm_action(None, true), None);
    }

    #[test]
    fn upload_command_replaces_specific_gist_file() {
        let target = gist_file();
        let plan = upload_command(PathBuf::from("/tmp/settings.json").as_path(), &target);

        assert_eq!(plan.program, "gh");
        assert_eq!(
            plan.args,
            vec![
                "gist",
                "edit",
                "abc123",
                "--filename",
                "settings.json",
                "/tmp/settings.json"
            ]
        );
    }

    #[test]
    fn upload_add_command_adds_local_file_to_gist() {
        let plan = upload_add_command(PathBuf::from("/tmp/config.toml").as_path(), "abc123");
        assert_eq!(plan.program, "gh");
        assert_eq!(
            plan.args,
            vec!["gist", "edit", "abc123", "--add", "/tmp/config.toml"]
        );
    }

    #[test]
    fn open_browser_command_targets_gist_web_view() {
        let plan = open_browser_command("abc123");
        assert_eq!(plan.program, "gh");
        assert_eq!(plan.args, vec!["gist", "view", "abc123", "--web"]);
    }

    #[test]
    fn open_repo_browser_command_targets_repo_view() {
        let plan = open_repo_browser_command();
        assert_eq!(plan.program, "gh");
        assert_eq!(plan.args, vec!["repo", "view", "akunzai/gistui", "--web"]);
    }

    #[test]
    fn delete_command_targets_gist_delete() {
        let plan = delete_command("abc123");
        assert_eq!(plan.program, "gh");
        assert_eq!(plan.args, vec!["gist", "delete", "--yes", "abc123"]);
    }

    #[test]
    fn remove_file_command_removes_single_file() {
        let plan = remove_file_command("abc123", "notes.md");
        assert_eq!(plan.program, "gh");
        assert_eq!(
            plan.args,
            vec!["gist", "edit", "abc123", "--remove", "notes.md"]
        );
    }

    #[test]
    fn edit_description_command_patches_via_rest_api() {
        // Must NOT use `gh gist edit --desc`, which opens an interactive editor.
        let plan = edit_description_command("abc123", "new desc");
        assert_eq!(plan.program, "gh");
        assert_eq!(
            plan.args,
            vec![
                "api",
                "--method",
                "PATCH",
                "/gists/abc123",
                "-f",
                "description=new desc"
            ]
        );
    }

    #[test]
    fn create_command_defaults_to_secret() {
        let plan = create_command(PathBuf::from("/tmp/settings.json").as_path(), false, "");
        assert_eq!(plan.args, vec!["gist", "create", "/tmp/settings.json"]);
        assert!(!plan.args.contains(&"--public".to_string()));
    }

    #[test]
    fn create_command_includes_public_and_description() {
        let plan = create_command(PathBuf::from("/tmp/notes.md").as_path(), true, "my notes");
        assert_eq!(
            plan.args,
            vec![
                "gist",
                "create",
                "/tmp/notes.md",
                "--public",
                "--desc",
                "my notes"
            ]
        );
    }

    #[test]
    fn create_command_omits_desc_flag_when_description_empty() {
        let plan = create_command(PathBuf::from("/tmp/notes.md").as_path(), false, "");
        assert!(!plan.args.contains(&"--desc".to_string()));
    }

    #[test]
    fn download_refuses_unconfirmed_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "old").unwrap();

        let err = execute_download(&path, "new", false).unwrap_err();
        assert!(err.to_string().contains("refusing to overwrite"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
    }

    #[test]
    fn unpin_mapping_removes_mapping_for_local_path() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let local_path = dir.path().join("settings.json");
        let target = gist_file();

        let config = pin_mapping(
            &config_path,
            AppConfig::default(),
            &local_path,
            &target,
            None,
            None,
        )
        .unwrap();
        assert_eq!(config.pinned.len(), 1);

        let config = unpin_mapping(&config_path, config, &local_path).unwrap();
        assert!(config.pinned.is_empty());
        let loaded = load_config(&config_path).unwrap();
        assert!(loaded.pinned.is_empty());
    }

    #[test]
    fn pin_mapping_replaces_existing_local_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let local_path = dir.path().join("settings.json");
        let target = gist_file();

        let config = pin_mapping(
            &config_path,
            AppConfig::default(),
            &local_path,
            &target,
            Some(crate::domain::SyncDirection::Upload),
            Some("hash".into()),
        )
        .unwrap();

        assert_eq!(config.pinned.len(), 1);
        let loaded = load_config(&config_path).unwrap();
        assert_eq!(loaded.pinned[0].local_path, local_path);
        assert_eq!(loaded.pinned[0].gist_id, "abc123");
    }
}
