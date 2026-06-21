use crate::config::{save_config, AppConfig};
use crate::domain::{GistFile, PinnedMapping, SyncDirection};
use anyhow::{anyhow, bail, Context, Result};
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

/// The public web URL for a gist id (what `gh gist view --web` opens).
pub fn gist_web_url(gist_id: &str) -> String {
    format!("https://gist.github.com/{gist_id}")
}

/// Clipboard-copy candidates for `os` (an `std::env::consts::OS` value), in
/// priority order. Each reads the text to copy from stdin. Returns empty for
/// platforms with no known tool, so callers can report a clear status.
pub fn clipboard_copy_candidates(os: &str) -> Vec<CommandPlan> {
    let specs: &[(&str, &[&str])] = match os {
        "macos" => &[("pbcopy", &[])],
        "windows" => &[("clip", &[])],
        // Linux/BSD: prefer Wayland, then fall back to the X11 tools.
        "linux" | "freebsd" | "netbsd" | "openbsd" | "dragonfly" | "solaris" | "illumos" => &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ],
        _ => &[],
    };
    specs
        .iter()
        .map(|(program, args)| CommandPlan {
            program: (*program).into(),
            args: args.iter().map(|a| (*a).to_string()).collect(),
        })
        .collect()
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

/// Asks the REST API for the number of revisions a gist has. `--jq` collapses the
/// `history` array to its length so the command's stdout is just an integer.
pub fn gist_revision_count_command(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            format!("/gists/{gist_id}"),
            "--jq".into(),
            ".history | length".into(),
        ],
    }
}

/// Parse the integer printed by [`gist_revision_count_command`].
pub fn parse_revision_count(stdout: &str) -> Option<usize> {
    stdout.trim().parse().ok()
}

/// JSON body for restoring a single file from an old gist revision via `PATCH /gists/{id}`.
pub fn restore_revision_json(filename: &str, content: &str) -> String {
    serde_json::json!({
        "files": {
            filename: { "content": content }
        }
    })
    .to_string()
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

/// `gh api --method PATCH` plan that uploads old file content as a new gist revision.
pub fn restore_revision_command(gist_id: &str, input_path: &Path) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--method".into(),
            "PATCH".into(),
            format!("/gists/{gist_id}"),
            "--input".into(),
            input_path.display().to_string(),
        ],
    }
}

/// Clones `gist_id` into `dir` as a git working copy (the gist's revisions are its commits).
///
/// Cloned over HTTPS (not `gh gist clone`, which follows the user's `git_protocol` and may
/// pick SSH) so both the clone and the later force-push authenticate through git's credential
/// helper — the `gh` token. Compaction runs while the TUI owns the terminal in raw mode, so an
/// SSH key passphrase prompt cannot be answered and fails (`incorrect passphrase supplied to
/// decrypt private key`); routing through HTTPS/`gh` token avoids SSH keys entirely.
pub fn gist_clone_command(gist_id: &str, dir: &Path) -> CommandPlan {
    CommandPlan {
        program: "git".into(),
        args: vec![
            "clone".into(),
            format!("https://gist.github.com/{gist_id}.git"),
            dir.display().to_string(),
        ],
    }
}

fn git_in(dir: &Path, args: &[&str]) -> CommandPlan {
    let mut full = vec!["-C".to_string(), dir.display().to_string()];
    full.extend(args.iter().map(|a| a.to_string()));
    CommandPlan {
        program: "git".into(),
        args: full,
    }
}

/// The command that reports a clone's checked-out branch (the gist's default branch).
pub fn git_current_branch_command(dir: &Path) -> CommandPlan {
    git_in(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
}

/// The ordered git steps that collapse a cloned gist working copy into a single root commit
/// and force-push it back over `branch`. Pure so the plan is unit-testable; the branch name is
/// resolved separately (see [`compact_gist_repo`]). A committer identity is forced via `-c` so
/// the commit succeeds regardless of the user's global git config.
pub fn compact_git_plans(dir: &Path, branch: &str) -> Vec<CommandPlan> {
    vec![
        git_in(dir, &["checkout", "--orphan", "__gistui_compact"]),
        git_in(dir, &["add", "-A"]),
        git_in(
            dir,
            &[
                "-c",
                "user.name=gistui",
                "-c",
                "user.email=gistui@users.noreply.github.com",
                "commit",
                "-m",
                "Compact gist history",
            ],
        ),
        git_in(dir, &["branch", "-M", branch]),
        git_in(dir, &["push", "--force", "origin", branch]),
    ]
}

/// Clone `gist_id` into a fresh temp dir, collapse its history to a single commit, force-push,
/// and remove the temp dir. The temp dir is always cleaned up, even on error. This is a thin IO
/// boundary (real `gh`/`git`); the command planning it drives is what carries the unit tests.
pub fn execute_compact_gist(gist_id: &str) -> Result<()> {
    let dir = compact_temp_dir(gist_id);
    let result = compact_in_dir(gist_id, &dir);
    let _ = fs::remove_dir_all(&dir);
    // A raw git HTTPS auth failure (no gist.github.com credential helper) is confusing; map it
    // to an actionable hint. Unrelated errors surface verbatim, and the happy path is untouched.
    result.map_err(|e| match compact_auth_hint(&e.to_string()) {
        Some(hint) => anyhow!(hint),
        None => e,
    })
}

/// Map a git failure `stderr` to an actionable hint when it looks like an HTTPS authentication
/// failure against gist.github.com — typically because `gh auth setup-git` was never run, so git
/// has no credential helper for the host. Returns `None` for unrelated errors so they surface
/// verbatim. See #71 (follow-up to #65, which routes compaction over HTTPS/the gh token).
pub fn compact_auth_hint(stderr: &str) -> Option<String> {
    let lower = stderr.to_lowercase();
    let is_auth_failure = lower.contains("could not read username")
        || lower.contains("could not read password")
        || lower.contains("authentication failed")
        || lower.contains("terminal prompts disabled");
    is_auth_failure.then(|| {
        "git could not authenticate to gist.github.com. \
         Run `gh auth setup-git` to enable gist compaction (one-time setup)."
            .to_string()
    })
}

fn compact_in_dir(gist_id: &str, dir: &Path) -> Result<()> {
    run_command(&SystemRunner, &gist_clone_command(gist_id, dir))?;
    let branch = run_command(&SystemRunner, &git_current_branch_command(dir))?
        .trim()
        .to_string();
    if branch.is_empty() {
        bail!("could not determine the gist's default branch");
    }
    for plan in compact_git_plans(dir, &branch) {
        run_command(&SystemRunner, &plan)?;
    }
    Ok(())
}

/// A unique, not-yet-existing temp path for a clone (let `git` create it, avoiding any
/// "clone into a non-empty dir" surprise). Uniqueness: gist id + pid + a high-resolution stamp.
fn compact_temp_dir(gist_id: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let safe: String = gist_id.chars().filter(|c| c.is_alphanumeric()).collect();
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "gistui-compact-{safe}-{}-{stamp}",
        std::process::id()
    ));
    dir
}

pub fn execute_command(plan: &CommandPlan) -> Result<String> {
    run_command(&SystemRunner, plan)
}

/// Copies `text` to the system clipboard by shelling out to the first available
/// platform tool (pbcopy/clip/wl-copy/xclip/xsel), piping `text` via stdin.
/// Returns the tool used on success, or an error naming the tools tried so the
/// headless / no-clipboard case surfaces as a status rather than a panic.
/// Thin IO boundary: not unit-tested (mirrors [`execute_command`]).
pub fn copy_to_clipboard(text: &str) -> Result<String> {
    let candidates = clipboard_copy_candidates(std::env::consts::OS);
    let tried = candidates
        .iter()
        .map(|p| p.program.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if candidates.is_empty() {
        bail!("no clipboard tool for this platform");
    }
    let mut last_err: Option<String> = None;
    for plan in &candidates {
        match copy_via(plan, text) {
            Ok(()) => return Ok(plan.program.clone()),
            Err(error) => last_err = Some(error.to_string()),
        }
    }
    match last_err {
        Some(error) => bail!("no clipboard tool worked (tried {tried}): {error}"),
        None => bail!("no clipboard tool found (tried {tried})"),
    }
}

fn copy_via(plan: &CommandPlan, text: &str) -> Result<()> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(&plan.program)
        .args(&plan.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn {}", plan.program))?;
    {
        let mut stdin = child.stdin.take().context("clipboard stdin unavailable")?;
        stdin.write_all(text.as_bytes())?;
        // `stdin` drops here, closing the pipe so the tool sees EOF and exits.
    }
    let status = child.wait()?;
    if !status.success() {
        bail!("{} exited with {status}", plan.program);
    }
    Ok(())
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

/// Record the result of a successful sync for the pin identified by
/// `(local_path, gist_id, gist_filename)`: set `last_seen_hash` to the agreed
/// content hash and `direction` to the direction performed, then persist.
/// No-op if the pin is not found.
pub fn record_sync(
    config_path: &Path,
    mut config: AppConfig,
    local_path: &Path,
    gist_id: &str,
    gist_filename: &str,
    hash: &str,
    direction: SyncDirection,
) -> Result<AppConfig> {
    if let Some(mapping) = config.pinned.iter_mut().find(|m| {
        m.local_path == local_path && m.gist_id == gist_id && m.gist_filename == gist_filename
    }) {
        mapping.last_seen_hash = Some(hash.to_string());
        mapping.direction = Some(direction);
        save_config(config_path, &config)?;
    }
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
            owner_login: String::new(),
            fork_of_id: None,

            raw_url: None,

            content_type: None,

            node_id: None,
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
    fn gist_web_url_builds_canonical_gist_link() {
        assert_eq!(gist_web_url("abc123"), "https://gist.github.com/abc123");
    }

    #[test]
    fn clipboard_candidates_pick_the_platform_tool() {
        assert_eq!(
            clipboard_copy_candidates("macos")
                .iter()
                .map(|p| p.program.clone())
                .collect::<Vec<_>>(),
            vec!["pbcopy"]
        );
        assert_eq!(
            clipboard_copy_candidates("windows")
                .iter()
                .map(|p| p.program.clone())
                .collect::<Vec<_>>(),
            vec!["clip"]
        );
        // Linux prefers Wayland, then falls back to the X11 tools in order.
        assert_eq!(
            clipboard_copy_candidates("linux")
                .iter()
                .map(|p| p.program.clone())
                .collect::<Vec<_>>(),
            vec!["wl-copy", "xclip", "xsel"]
        );
        let xclip = &clipboard_copy_candidates("linux")[1];
        assert_eq!(xclip.args, vec!["-selection", "clipboard"]);
    }

    #[test]
    fn clipboard_candidates_empty_for_unknown_os() {
        assert!(clipboard_copy_candidates("plan9").is_empty());
    }

    #[test]
    fn delete_command_targets_gist_delete() {
        let plan = delete_command("abc123");
        assert_eq!(plan.program, "gh");
        assert_eq!(plan.args, vec!["gist", "delete", "--yes", "abc123"]);
    }

    #[test]
    fn restore_revision_json_wraps_file_content() {
        let body = restore_revision_json("config.toml", "old line\n");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["files"]["config.toml"]["content"], "old line\n");
    }

    #[test]
    fn restore_revision_command_patches_via_input_file() {
        let plan = restore_revision_command("abc123", Path::new("/tmp/restore.json"));
        assert_eq!(plan.program, "gh");
        assert_eq!(
            plan.args,
            vec![
                "api",
                "--method",
                "PATCH",
                "/gists/abc123",
                "--input",
                "/tmp/restore.json"
            ]
        );
    }

    #[test]
    fn gist_revision_count_command_uses_history_length_jq() {
        let plan = gist_revision_count_command("abc123");
        assert_eq!(plan.program, "gh");
        assert_eq!(
            plan.args,
            vec!["api", "/gists/abc123", "--jq", ".history | length"]
        );
    }

    #[test]
    fn parse_revision_count_reads_trimmed_integer() {
        assert_eq!(parse_revision_count("12\n"), Some(12));
        assert_eq!(parse_revision_count("  1 "), Some(1));
        assert_eq!(parse_revision_count("not a number"), None);
        assert_eq!(parse_revision_count(""), None);
    }

    #[test]
    fn gist_clone_command_clones_over_https_into_dir() {
        let plan = gist_clone_command("abc123", Path::new("/tmp/x"));
        // HTTPS (not `gh gist clone`/SSH) so auth flows through the gh token credential
        // helper and compaction never hits an SSH passphrase prompt under the TUI.
        assert_eq!(plan.program, "git");
        assert_eq!(
            plan.args,
            vec!["clone", "https://gist.github.com/abc123.git", "/tmp/x"]
        );
    }

    #[test]
    fn compact_git_plans_squash_to_one_commit_and_force_push() {
        let plans = compact_git_plans(Path::new("/tmp/x"), "main");
        // Every step runs against the clone dir.
        assert!(plans
            .iter()
            .all(|p| p.program == "git" && p.args[0] == "-C" && p.args[1] == "/tmp/x"));
        let verbs: Vec<&str> = plans.iter().map(|p| p.args[2].as_str()).collect();
        assert_eq!(verbs, vec!["checkout", "add", "-c", "branch", "push"]);
        // Orphan checkout drops all parents; the final step force-pushes the rebuilt branch.
        assert_eq!(
            plans[0].args,
            vec!["-C", "/tmp/x", "checkout", "--orphan", "__gistui_compact"]
        );
        assert_eq!(
            plans.last().unwrap().args,
            vec!["-C", "/tmp/x", "push", "--force", "origin", "main"]
        );
        // The commit forces an identity so it never falls back to (possibly absent) global config.
        assert!(plans[2].args.contains(&"user.name=gistui".to_string()));
    }

    #[test]
    fn compact_auth_hint_flags_git_auth_failures() {
        // The signatures git emits when no gist.github.com credential helper is configured.
        for stderr in [
            "fatal: could not read Username for 'https://gist.github.com': terminal prompts disabled",
            "remote: Support for password authentication was removed.\nfatal: Authentication failed for 'https://gist.github.com/abc.git/'",
            "fatal: could not read Password for 'https://gist.github.com': No such device",
        ] {
            let hint = compact_auth_hint(stderr).expect("auth failure should yield a hint");
            assert!(hint.contains("gh auth setup-git"));
        }
    }

    #[test]
    fn compact_auth_hint_ignores_unrelated_errors() {
        assert_eq!(
            compact_auth_hint("could not determine the gist's default branch"),
            None
        );
        assert_eq!(
            compact_auth_hint(
                "fatal: unable to access 'https://gist.github.com/': Could not resolve host"
            ),
            None
        );
    }

    #[test]
    fn current_branch_command_reads_head() {
        let plan = git_current_branch_command(Path::new("/tmp/x"));
        assert_eq!(plan.program, "git");
        assert_eq!(
            plan.args,
            vec!["-C", "/tmp/x", "rev-parse", "--abbrev-ref", "HEAD"]
        );
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
    fn download_writes_new_file_without_confirmation() {
        // Writing a path that does not exist yet is allowed directly (no diff/confirm gate),
        // creating any missing parent directories along the way.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/dir/settings.json");
        execute_download(&path, "hello", false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn download_overwrites_existing_when_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "old").unwrap();
        execute_download(&path, "new", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
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
    fn unpin_mapping_exact_removes_only_matching_local_and_gist() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let mut config = AppConfig::default();
        // Same local path pinned to two different gists — exact unpin must remove only the
        // named gist and leave the other pin intact.
        for gid in ["g1", "g2"] {
            config.pinned.push(PinnedMapping {
                local_path: PathBuf::from("/tmp/a.txt"),
                gist_id: gid.into(),
                gist_filename: "a.txt".into(),
                direction: None,
                last_seen_hash: None,
            });
        }

        let config =
            unpin_mapping_exact(&config_path, config, Path::new("/tmp/a.txt"), "g1").unwrap();
        assert_eq!(config.pinned.len(), 1);
        assert_eq!(config.pinned[0].gist_id, "g2");
        let loaded = load_config(&config_path).unwrap();
        assert_eq!(loaded.pinned.len(), 1);
        assert_eq!(loaded.pinned[0].gist_id, "g2");
    }

    #[test]
    fn unpin_mapping_exact_no_match_leaves_pins_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let mut config = AppConfig::default();
        config.pinned.push(PinnedMapping {
            local_path: PathBuf::from("/tmp/a.txt"),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        });

        let config =
            unpin_mapping_exact(&config_path, config, Path::new("/tmp/a.txt"), "nope").unwrap();
        assert_eq!(config.pinned.len(), 1);
        assert_eq!(config.pinned[0].gist_id, "g1");
    }

    #[test]
    fn record_sync_updates_hash_and_direction() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("config.toml");
        let mut config = AppConfig::default();
        config.pinned.push(PinnedMapping {
            local_path: PathBuf::from("/tmp/a.txt"),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        });

        let updated = record_sync(
            &cfg_path,
            config,
            Path::new("/tmp/a.txt"),
            "g1",
            "a.txt",
            "deadbeef",
            SyncDirection::Upload,
        )
        .unwrap();

        let m = &updated.pinned[0];
        assert_eq!(m.last_seen_hash.as_deref(), Some("deadbeef"));
        assert_eq!(m.direction, Some(SyncDirection::Upload));
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
