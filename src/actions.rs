use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPlan {
    pub program: String,
    pub args: Vec<String>,
}

/// Capability token: target may be overwritten. Private field so callers cannot
/// forge it with a bare `true` — only [`DownloadMode::overwrite_after_user_confirm`]
/// constructs one (issue #246).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverwriteConfirmed {
    _private: (),
}

/// How [`execute_download`] may write the target path.
///
/// - [`DownloadMode::CreateNew`]: refuse if the path already exists.
/// - [`DownloadMode::Overwrite`]: allowed only with a token minted after the user
///   confirmed overwrite (diff → Confirm → `y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadMode {
    CreateNew,
    Overwrite(OverwriteConfirmed),
}

impl DownloadMode {
    /// Mint overwrite permission after the user accepts the Confirm dialog.
    /// This is the only supported producer of [`OverwriteConfirmed`].
    pub const fn overwrite_after_user_confirm() -> Self {
        Self::Overwrite(OverwriteConfirmed { _private: () })
    }
}

/// The captured result of running a [`CommandPlan`], independent of how it was
/// produced. Mirrors the fields of `std::process::Output` the app actually uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The injectable boundary for the app's external IO: every command it shells out to (`gh`,
/// `git`) and every raw gist file it downloads. Production uses [`SystemRunner`]; tests
/// supply a fake so integration tests exercise command planning, success/failure handling,
/// and output parsing without touching the network or requiring `gh`. One runner carries
/// both, so a scripted test sees commands and raw fetches in the order they happened.
///
/// It expresses two shapes: [`CommandRunner::run`] spawns a program with arguments, waits,
/// and captures stdout/stderr; [`CommandRunner::fetch_raw`] GETs a gist `raw_url` (#507 —
/// no `curl` needed). Three paths need more than that and so call `std::process::Command`
/// directly. They are the whole set — anything else belongs behind this trait:
///
/// - **Piped stdin** — [`copy_via`] writes the payload to the child's stdin and closes
///   the pipe so the clipboard tool sees EOF.
/// - **TTY handoff** — `tui::bg`'s editor launches leave the alternate screen, hand the
///   terminal to the child, and restore raw mode after it exits.
/// - **A long-lived watched child** — `tui::bg`'s upload-edit watch spawns the editor and
///   keeps polling it, rather than waiting for a single result.
pub trait CommandRunner {
    fn run(&self, plan: &CommandPlan) -> Result<CommandOutput>;
    /// The body of an HTTP GET of a gist `raw_url` — anonymous, unauthenticated. A non-2xx
    /// status is an error.
    fn fetch_raw(&self, url: &str) -> Result<Vec<u8>>;
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

    fn fetch_raw(&self, url: &str) -> Result<Vec<u8>> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .user_agent("gistui")
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build()
            .into();
        let limit = crate::domain::MAX_TEXT_FILE_BYTES;
        agent
            .get(url)
            .call()
            .and_then(|mut response| response.body_mut().with_config().limit(limit).read_to_vec())
            .map_err(|e| match e {
                ureq::Error::BodyExceedsLimit(_) => anyhow!(
                    "file too large for preview/diff (over the {} MiB limit)",
                    limit / (1024 * 1024)
                ),
                e => anyhow!("fetch {url}: {e}"),
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

/// Shared sequential fake runner for unit tests (issue #245). Feeds scripted
/// [`CommandOutput`]s in order and records every [`CommandPlan`] for assertions.
#[cfg(test)]
pub mod test_support {
    use super::{CommandOutput, CommandPlan, CommandRunner};
    use anyhow::{anyhow, Result};
    use std::sync::Mutex;

    /// One recorded call: the plan, plus the body of any `--input <path>` file, read at
    /// call time. The restore-revision payload lives in a scratch directory the worker
    /// deletes when it finishes, so that file is only observable while the command runs
    /// (issue #430).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RecordedCall {
        pub plan: CommandPlan,
        pub input_body: Option<String>,
    }

    #[derive(Default)]
    struct SeqState {
        outputs: Vec<CommandOutput>,
        calls: Vec<RecordedCall>,
        next: usize,
    }

    /// Scripted runner. Guarded by a `Mutex` rather than a `RefCell` so an `Arc<SeqRunner>`
    /// can be injected into [`crate::tui`]'s job registry and shared with the worker
    /// closures that run off the event-loop thread (issue #430).
    pub struct SeqRunner {
        state: Mutex<SeqState>,
    }

    impl SeqRunner {
        pub fn new(outputs: Vec<CommandOutput>) -> Self {
            Self {
                state: Mutex::new(SeqState {
                    outputs,
                    ..SeqState::default()
                }),
            }
        }

        pub fn calls(&self) -> Vec<CommandPlan> {
            self.recorded().into_iter().map(|c| c.plan).collect()
        }

        pub fn recorded(&self) -> Vec<RecordedCall> {
            self.state.lock().expect("SeqRunner poisoned").calls.clone()
        }
    }

    /// The path following `--input`, if the plan has one.
    pub fn input_path(plan: &CommandPlan) -> Option<&String> {
        let i = plan.args.iter().position(|a| a == "--input")?;
        plan.args.get(i + 1)
    }

    impl CommandOutput {
        /// Scripted success — the common case, where only stdout matters.
        pub fn ok(stdout: impl Into<String>) -> Self {
            Self {
                success: true,
                stdout: stdout.into(),
                stderr: String::new(),
            }
        }

        /// Scripted failure; `stderr` becomes the error `run_command` reports.
        pub fn err(stderr: impl Into<String>) -> Self {
            Self {
                success: false,
                stdout: String::new(),
                stderr: stderr.into(),
            }
        }
    }

    impl CommandRunner for SeqRunner {
        fn run(&self, plan: &CommandPlan) -> Result<CommandOutput> {
            let input_body = input_path(plan).and_then(|p| std::fs::read_to_string(p).ok());
            let mut state = self.state.lock().expect("SeqRunner poisoned");
            state.calls.push(RecordedCall {
                plan: plan.clone(),
                input_body,
            });
            let i = state.next;
            state.next = i + 1;
            state
                .outputs
                .get(i)
                .cloned()
                .ok_or_else(|| anyhow!("no output for call {i}"))
        }

        /// Recorded as [`raw_get`] in the same sequence as commands; a scripted success's
        /// stdout is the body, a scripted failure's stderr the error.
        fn fetch_raw(&self, url: &str) -> Result<Vec<u8>> {
            let output = self.run(&raw_get(url))?;
            if output.success {
                Ok(output.stdout.into_bytes())
            } else {
                Err(anyhow!("{}", output.stderr))
            }
        }
    }

    /// How a [`CommandRunner::fetch_raw`] of `url` appears in a [`SeqRunner`]'s calls.
    pub fn raw_get(url: &str) -> CommandPlan {
        CommandPlan {
            program: "GET".into(),
            args: vec![url.into()],
        }
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

pub fn open_url_command(url: &str) -> CommandPlan {
    open_url_command_for_os(url, std::env::consts::OS)
}

fn open_url_command_for_os(url: &str, os: &str) -> CommandPlan {
    match os {
        "macos" => CommandPlan {
            program: "open".into(),
            args: vec![url.to_string()],
        },
        "windows" => CommandPlan {
            program: "cmd".into(),
            args: vec!["/c".into(), "start".into(), "".into(), url.to_string()],
        },
        _ => CommandPlan {
            program: "xdg-open".into(),
            args: vec![url.to_string()],
        },
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

/// Runs a planned command with the real process boundary ([`SystemRunner`]).
/// Prefer this over spelling `run_command(&SystemRunner, …)` at every write path.
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

/// Write `content` to `local_path` exactly as given. The Sync policy decides those bytes
/// ([`crate::sync_content::SyncPolicy::write_download`]).
///
/// Existing targets require [`DownloadMode::Overwrite`] (user confirmed after diff).
/// New paths use [`DownloadMode::CreateNew`] with no confirm token.
pub fn execute_download(local_path: &Path, content: &str, mode: DownloadMode) -> Result<()> {
    match mode {
        DownloadMode::CreateNew if local_path.exists() => {
            bail!(
                "refusing to overwrite {} without confirmation",
                local_path.display()
            );
        }
        DownloadMode::CreateNew | DownloadMode::Overwrite(_) => {}
    }
    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(local_path, content).with_context(|| format!("write {}", local_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_browser_command_targets_gist_web_view() {
        let plan = open_browser_command("abc123");
        assert_eq!(plan.program, "gh");
        assert_eq!(plan.args, vec!["gist", "view", "abc123", "--web"]);
    }

    #[test]
    fn open_url_command_picks_platform_opener() {
        let macos_plan = open_url_command_for_os("https://example.com", "macos");
        assert_eq!(macos_plan.program, "open");
        assert_eq!(macos_plan.args, vec!["https://example.com".to_string()]);

        let windows_plan = open_url_command_for_os("https://example.com", "windows");
        assert_eq!(windows_plan.program, "cmd");
        assert_eq!(
            windows_plan.args,
            vec![
                "/c".to_string(),
                "start".to_string(),
                "".to_string(),
                "https://example.com".to_string()
            ]
        );

        let linux_plan = open_url_command_for_os("https://example.com", "linux");
        assert_eq!(linux_plan.program, "xdg-open");
        assert_eq!(linux_plan.args, vec!["https://example.com".to_string()]);
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
    fn download_refuses_create_new_when_target_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "old").unwrap();

        let err = execute_download(&path, "new", DownloadMode::CreateNew).unwrap_err();
        assert!(err.to_string().contains("refusing to overwrite"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
    }

    #[test]
    fn download_writes_new_file_without_confirmation() {
        // Writing a path that does not exist yet is allowed directly (no diff/confirm gate),
        // creating any missing parent directories along the way.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/dir/settings.json");
        execute_download(&path, "hello", DownloadMode::CreateNew).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn download_overwrites_existing_when_token_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "old").unwrap();
        execute_download(&path, "new", DownloadMode::overwrite_after_user_confirm()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn overwrite_confirmed_is_not_a_public_bool_field() {
        // Structural: DownloadMode::Overwrite requires OverwriteConfirmed; the only
        // public mint is overwrite_after_user_confirm (no `true` flag).
        let mode = DownloadMode::overwrite_after_user_confirm();
        assert!(matches!(mode, DownloadMode::Overwrite(_)));
        assert!(matches!(DownloadMode::CreateNew, DownloadMode::CreateNew));
    }
}
