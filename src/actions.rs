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

pub fn create_command(local_path: &Path, public: bool) -> CommandPlan {
    let mut args = vec![
        "gist".into(),
        "create".into(),
        local_path.display().to_string(),
    ];
    if public {
        args.push("--public".into());
    }
    CommandPlan {
        program: "gh".into(),
        args,
    }
}

pub fn execute_command(plan: &CommandPlan) -> Result<String> {
    let output = Command::new(&plan.program)
        .args(&plan.args)
        .output()
        .with_context(|| format!("run {} {}", plan.program, plan.args.join(" ")))?;

    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(String::from_utf8(output.stdout)?)
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
    fn create_command_defaults_to_secret() {
        let plan = create_command(PathBuf::from("/tmp/settings.json").as_path(), false);
        assert_eq!(plan.args, vec!["gist", "create", "/tmp/settings.json"]);
        assert!(!plan.args.contains(&"--public".to_string()));
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
