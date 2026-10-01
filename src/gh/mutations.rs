//! `gh` plans for the writes a Gist mutation makes (issue #301 layout): upload, create,
//! delete, remove a file, edit the description, star, unstar, fork. Pure plan builders; the
//! read-side plans live beside their resource and a restore's plan in `revisions`.

use crate::actions::CommandPlan;
use crate::domain::GistFile;
use std::path::Path;

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

/// Star a gist (`PUT /gists/{id}/star`).
pub fn star_gist_command(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--method".into(),
            "PUT".into(),
            format!("/gists/{gist_id}/star"),
        ],
    }
}

/// Unstar a gist (`DELETE /gists/{id}/star`).
pub fn unstar_gist_command(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--method".into(),
            "DELETE".into(),
            format!("/gists/{gist_id}/star"),
        ],
    }
}

/// Fork a gist into the authenticated user's account (`POST /gists/{id}/forks`).
pub fn fork_gist_command(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--method".into(),
            "POST".into(),
            format!("/gists/{gist_id}/forks"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn gist_file() -> GistFile {
        GistFile {
            description: "config".into(),
            updated_at: "2026-06-08T00:00:00Z".into(),
            created_at: "2026-06-08T00:00:00Z".into(),
            ..GistFile::fixture("abc123", "settings.json")
        }
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
    fn star_gist_command_puts_star_endpoint() {
        let plan = star_gist_command("abc123");
        assert_eq!(plan.program, "gh");
        assert_eq!(
            plan.args,
            vec![
                "api".to_string(),
                "--method".to_string(),
                "PUT".to_string(),
                "/gists/abc123/star".to_string(),
            ]
        );
    }

    #[test]
    fn unstar_gist_command_deletes_star_endpoint() {
        let plan = unstar_gist_command("abc123");
        assert_eq!(
            plan.args,
            vec![
                "api".to_string(),
                "--method".to_string(),
                "DELETE".to_string(),
                "/gists/abc123/star".to_string(),
            ]
        );
    }

    #[test]
    fn fork_gist_command_posts_forks_endpoint() {
        let plan = fork_gist_command("abc123");
        assert_eq!(
            plan.args,
            vec![
                "api".to_string(),
                "--method".to_string(),
                "POST".to_string(),
                "/gists/abc123/forks".to_string(),
            ]
        );
    }
}
