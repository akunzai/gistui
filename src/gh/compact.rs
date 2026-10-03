//! Gist compaction: count a gist's revisions, then clone it, squash its history to one
//! commit, and force-push. Every command goes through the [`CommandRunner`] seam.

use crate::actions::{run_command, CommandPlan, CommandRunner};
use anyhow::{anyhow, bail, Result};
use std::path::Path;

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

/// Count `gist_id`'s revisions. Compaction's read-only preflight: one revision has nothing to
/// squash.
pub fn fetch_revision_count(runner: &dyn CommandRunner, gist_id: &str) -> Result<usize> {
    let out = run_command(runner, &gist_revision_count_command(gist_id))?;
    parse_revision_count(&out).ok_or_else(|| anyhow!("could not parse revision count"))
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
/// resolved separately (see [`compact_in_dir`]). A committer identity is forced via `-c` so
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
/// and remove the temp dir. A thin shell that owns only the two things its body actually does —
/// the scratch dir lifetime and the auth-hint mapping — and is not unit-tested. The command
/// sequence it drives lives in [`compact_in_dir`], behind the [`CommandRunner`] seam.
pub fn execute_compact_gist(runner: &dyn CommandRunner, gist_id: &str) -> Result<()> {
    // `with_temp_scratch_dir` owns create + cleanup, including on clone/compact failure (issue #275).
    // The dir is created empty; `git clone` accepts an empty existing destination.
    let safe: String = gist_id.chars().filter(|c| c.is_alphanumeric()).collect();
    let kind = format!("compact-{safe}");
    let result =
        crate::temp_dir::with_temp_scratch_dir(&kind, |dir| compact_in_dir(runner, gist_id, dir));
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

/// Clone, resolve the checked-out branch, then run [`compact_git_plans`] against it, stopping at
/// the first failure. Takes `runner` so the ordering — and the trim that feeds the branch name
/// back into the plans — is unit-tested without a real `git`.
fn compact_in_dir(runner: &dyn CommandRunner, gist_id: &str, dir: &Path) -> Result<()> {
    run_command(runner, &gist_clone_command(gist_id, dir))?;
    let branch = run_command(runner, &git_current_branch_command(dir))?
        .trim()
        .to_string();
    if branch.is_empty() {
        bail!("could not determine the gist's default branch");
    }
    for plan in compact_git_plans(dir, &branch) {
        run_command(runner, &plan)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_support::SeqRunner;
    use crate::actions::CommandOutput;

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

    fn compact_ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    #[test]
    fn compact_in_dir_clones_resolves_the_branch_then_runs_the_plans() {
        let dir = Path::new("/tmp/x");
        let runner = SeqRunner::new(vec![
            compact_ok(""),       // clone
            compact_ok("main\n"), // rev-parse --abbrev-ref HEAD
            compact_ok(""),       // the five compact plans
            compact_ok(""),
            compact_ok(""),
            compact_ok(""),
            compact_ok(""),
        ]);

        compact_in_dir(&runner, "abc123", dir).expect("compaction should succeed");

        let calls = runner.calls();
        let mut expected = vec![
            gist_clone_command("abc123", dir),
            git_current_branch_command(dir),
        ];
        // The branch arrives as `main\n`; the trim is what makes the plans push a real ref.
        expected.extend(compact_git_plans(dir, "main"));
        assert_eq!(calls, expected);
        assert_eq!(
            calls.last().unwrap().args,
            vec!["-C", "/tmp/x", "push", "--force", "origin", "main"]
        );
    }

    #[test]
    fn compact_in_dir_bails_before_touching_history_when_the_branch_is_blank() {
        let runner = SeqRunner::new(vec![compact_ok(""), compact_ok("  \n")]);

        let err = compact_in_dir(&runner, "abc123", Path::new("/tmp/x"))
            .expect_err("a blank branch should abort");

        assert!(err.to_string().contains("default branch"));
        // Nothing after the probe ran: no orphan checkout, no force-push.
        assert_eq!(runner.calls().len(), 2);
    }

    #[test]
    fn compact_in_dir_stops_at_the_first_failing_plan() {
        let runner = SeqRunner::new(vec![
            compact_ok(""),
            compact_ok("main\n"),
            compact_ok(""), // checkout --orphan
            compact_ok(""), // add -A
            CommandOutput {
                success: false,
                stdout: String::new(),
                stderr: "nothing to commit".into(),
            },
        ]);

        let err = compact_in_dir(&runner, "abc123", Path::new("/tmp/x"))
            .expect_err("a failed plan should abort");

        assert!(err.to_string().contains("nothing to commit"));
        // `branch -M` and the force-push never ran.
        assert_eq!(runner.calls().len(), 5);
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
}
