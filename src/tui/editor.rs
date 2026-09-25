//! The **editor session**: handing a file to the user's external editor.
//!
//! This covers two flows. `e` on the List screen edits the local file in place
//! ([`edit_local_path`]). `e` on the upload Confirm edits a redact buffer
//! ([`edit_upload_buffer`]). A GUI editor is watched without blocking
//! ([`spawn_upload_edit_watch`]); a terminal editor gets the whole terminal
//! ([`SuspendedTerminal`]).
//!
//! The editor-choosing rules live here in one place:
//! - `$VISUAL`, then `$EDITOR`, then `notepad` on Windows or `vi` elsewhere.
//! - `$EDITOR` is split with quotes respected and no backslash escapes.
//! - On Windows, a bare program name is resolved through `PATH` and `PATHEXT`. This finds
//!   `code.cmd`, and Rust escapes arguments for batch scripts safely.
//!
//! The redact buffer holds content that has not been redacted yet. It lives in a private
//! [`ScratchDir`] that [`Jobs`] owns until the session ends, so quitting while a GUI editor
//! is still open cleans it up too.

use super::bg::{Jobs, UploadEditWatchEvent};
use super::*;
use crate::temp_dir::ScratchDir;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::{Path, PathBuf};

type Tui = Terminal<CrosstermBackend<io::Stdout>>;

/// Whether `program`'s basename matches a known GUI editor that forks and returns
/// immediately. Such an editor both needs `--wait` injected by [`parse_editor`], and, in the
/// upload-redact flow, can be watched without blocking instead of taking over the terminal.
/// Matching is on the basename, so a full path or a `.exe` / `.cmd` suffix still matches.
pub(super) fn editor_is_gui(program: &str) -> bool {
    let basename = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let base = Path::new(basename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(basename)
        .to_ascii_lowercase();
    matches!(
        base.as_str(),
        "code"
            | "code-insiders"
            | "codium"
            | "vscodium"
            | "cursor"
            | "windsurf"
            | "zed"
            | "subl"
            | "sublime_text"
    )
}

/// The editor setting: `$VISUAL`, then `$EDITOR`, then the platform default. Blank values
/// count as unset.
fn editor_setting(var: impl Fn(&str) -> Option<String>, windows: bool) -> String {
    ["VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(var)
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| if windows { "notepad" } else { "vi" }.to_string())
}

/// Split an editor setting into `(program, args)`.
///
/// Whitespace separates words except inside `"…"` or `'…'`. Backslashes are literal, so
/// Windows paths survive. If the whole setting is an existing file (an unquoted
/// `C:\Program Files\…\editor.exe`), it is taken as the program.
///
/// Known GUI editors fork and return immediately, so they get `--wait` unless the setting
/// already has it. Without that flag the terminal flow would read the buffer back before
/// the user saved, and could upload the **un-redacted** original.
///
/// Returns `None` for a blank setting.
fn parse_editor(setting: &str, is_file: impl Fn(&Path) -> bool) -> Option<(String, Vec<String>)> {
    let trimmed = setting.trim();
    let mut words = if is_file(Path::new(trimmed)) {
        vec![trimmed.to_string()]
    } else {
        split_words(trimmed)
    };
    if words.is_empty() {
        return None;
    }
    let program = words.remove(0);
    let mut args = words;
    if editor_is_gui(&program) && !args.iter().any(|a| a == "--wait" || a == "-w") {
        args.push("--wait".to_string());
    }
    Some((program, args))
}

fn split_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote = None;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// Windows program lookup: a bare name with no extension (`code`) is tried in each `PATH`
/// directory with each `PATHEXT` extension, and the first file found wins. That finds
/// `code.cmd`, which `Command` would not find by itself. A name that already has a
/// directory or an extension, or that matches nothing, is returned unchanged.
fn resolve_on_path(
    program: &str,
    dirs: &[PathBuf],
    pathext: &str,
    is_file: impl Fn(&Path) -> bool,
) -> PathBuf {
    let as_path = Path::new(program);
    if program.contains(['/', '\\']) || as_path.extension().is_some() {
        return as_path.to_path_buf();
    }
    let exts: Vec<&str> = pathext.split(';').filter(|e| !e.is_empty()).collect();
    dirs.iter()
        .flat_map(|dir| {
            exts.iter()
                .map(move |ext| dir.join(format!("{program}{}", ext.to_ascii_lowercase())))
        })
        .find(|candidate| is_file(candidate))
        .unwrap_or_else(|| as_path.to_path_buf())
}

/// The user's editor as `(program, args)`, or `None` when the setting is blank.
fn resolve_editor() -> Option<(PathBuf, Vec<String>)> {
    let windows = cfg!(windows);
    let setting = editor_setting(|k| std::env::var(k).ok(), windows);
    let (program, args) = parse_editor(&setting, |p| p.is_file())?;
    let program = if windows {
        let dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        resolve_on_path(&program, &dirs, &pathext, |p| p.is_file())
    } else {
        PathBuf::from(program)
    };
    Some((program, args))
}

/// The terminal handed over to an external program: mouse capture, raw mode, and the
/// alternate screen are all off. [`Self::resume`] takes them back. Dropping the guard without
/// resuming (an early return) also restores them, on a best-effort basis.
pub(super) struct SuspendedTerminal<'a> {
    terminal: &'a mut Tui,
    mouse: bool,
    resumed: bool,
}

impl<'a> SuspendedTerminal<'a> {
    pub(super) fn suspend(terminal: &'a mut Tui, mouse: bool) -> Result<Self> {
        if mouse {
            execute!(terminal.backend_mut(), DisableMouseCapture)?;
        }
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        Ok(Self {
            terminal,
            mouse,
            resumed: false,
        })
    }

    pub(super) fn resume(mut self) -> Result<()> {
        self.resumed = true;
        self.restore()
    }

    fn restore(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(self.terminal.backend_mut(), EnterAlternateScreen)?;
        if self.mouse {
            execute!(self.terminal.backend_mut(), EnableMouseCapture)?;
        }
        self.terminal.clear()?;
        Ok(())
    }
}

impl Drop for SuspendedTerminal<'_> {
    fn drop(&mut self) {
        if !self.resumed {
            let _ = self.restore();
        }
    }
}

/// Run a terminal editor on `file` with the terminal handed over, and wait for it to exit.
fn run_blocking(
    terminal: &mut Tui,
    state: &AppState,
    program: &Path,
    args: &[String],
    file: &Path,
) -> Result<io::Result<std::process::ExitStatus>> {
    let suspended = SuspendedTerminal::suspend(terminal, state.settings.mouse_enabled())?;
    let status = std::process::Command::new(program)
        .args(args)
        .arg(file)
        .status();
    suspended.resume()?;
    Ok(status)
}

/// Open `path` in the user's editor, in place. A wait flag is added for known GUI editors
/// (see [`parse_editor`]).
pub(super) fn edit_local_path(terminal: &mut Tui, state: &mut AppState, path: &Path) -> Result<()> {
    let Some((program, args)) = resolve_editor() else {
        state.set_status("no editor configured (set $EDITOR)");
        return Ok(());
    };
    match run_blocking(terminal, state, &program, &args, path)? {
        Ok(_) => state.set_status(format!("Edited {}", crate::config::display_path(path))),
        Err(error) => state.set_status(format!("editor failed: {error}")),
    }
    Ok(())
}

/// Edit the upload draft's outgoing content in a private redact buffer.
///
/// A GUI editor opens in its own window, and its saves stream back live
/// ([`spawn_upload_edit_watch`]). A terminal editor blocks, and the buffer is read back
/// when it exits. Either way the buffer never outlives the session.
pub(super) fn edit_upload_buffer(
    terminal: &mut Tui,
    state: &mut AppState,
    jobs: &mut Jobs,
) -> Result<()> {
    let Some(draft) = state.upload_draft() else {
        return Ok(());
    };
    let (gist_id, gist_filename) = (draft.gist_id.clone(), draft.filename.clone());
    let content = draft.content(&state.settings);
    // The buffer keeps the local file's name, so the editor picks the right syntax.
    let Some(name) = draft.local_path.file_name().map(|n| n.to_os_string()) else {
        return Ok(());
    };

    let Some((program, args)) = resolve_editor() else {
        state.set_status("no editor configured (set $EDITOR)");
        return Ok(());
    };
    let scratch = match ScratchDir::create("redact") {
        Ok(dir) => dir,
        Err(e) => {
            state.set_status(format!("failed to create temp dir: {e}"));
            return Ok(());
        }
    };
    let buffer = match scratch.create_file(&name, content.as_bytes()) {
        Ok(path) => path,
        Err(e) => {
            state.set_status(format!("failed to write temp file: {e}"));
            return Ok(());
        }
    };

    if editor_is_gui(&program.to_string_lossy()) {
        let rx = spawn_upload_edit_watch(program, args, buffer, gist_id, gist_filename);
        jobs.set_upload_edit_watch(rx, scratch);
        if let Some(draft) = state.upload_draft_mut() {
            draft.watching = true;
        }
        state.set_status("Editing in external editor — diff updates live");
        return Ok(());
    }

    match run_blocking(terminal, state, &program, &args, &buffer)? {
        Ok(_) => match crate::domain::read_text_file_capped(&buffer) {
            Ok(edited) => {
                if let Some(draft) = state.upload_draft_mut() {
                    draft.edited_content = Some(edited);
                }
                state.update_upload_diff();
                state.set_status("Edited redact buffer");
            }
            Err(e) => state.set_status(format!("failed to read edited file: {e}")),
        },
        Err(error) => state.set_status(format!("editor failed: {error}")),
    }
    // `scratch` drops here, and on every early return above, which removes the buffer.
    Ok(())
}

/// Watch `buffer` while a GUI editor that was not asked to block has it open. Every
/// detected save (polled every 500ms) sends `ContentChanged`. When the editor exits or fails
/// to start, one final `EditorClosed` or `ReadError` is sent. Reads use the same size cap as
/// the draft's first read. The buffer's [`ScratchDir`] is owned by [`Jobs`], not by this
/// thread.
pub(super) fn spawn_upload_edit_watch(
    program: PathBuf,
    args: Vec<String>,
    buffer: PathBuf,
    gist_id: String,
    filename: String,
) -> std::sync::mpsc::Receiver<UploadEditWatchEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut child = match std::process::Command::new(&program)
            .args(&args)
            .arg(&buffer)
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                let _ = tx.send(UploadEditWatchEvent::ReadError {
                    gist_id,
                    filename,
                    message: format!("editor failed to start: {e}"),
                });
                return;
            }
        };

        let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mut last_modified = modified(&buffer);
        loop {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            let now = modified(&buffer);
            if now.is_some() && now != last_modified {
                last_modified = now;
                if let Ok(content) = crate::domain::read_text_file_capped(&buffer) {
                    let _ = tx.send(UploadEditWatchEvent::ContentChanged {
                        gist_id: gist_id.clone(),
                        filename: filename.clone(),
                        content,
                    });
                }
            }
        }

        let final_event = match crate::domain::read_text_file_capped(&buffer) {
            Ok(content) => UploadEditWatchEvent::EditorClosed {
                gist_id,
                filename,
                content,
            },
            Err(e) => UploadEditWatchEvent::ReadError {
                gist_id,
                filename,
                message: format!("failed to read edited file: {e}"),
            },
        };
        let _ = tx.send(final_event);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(setting: &str) -> Option<(String, Vec<String>)> {
        parse_editor(setting, |_| false)
    }

    #[test]
    fn parse_editor_injects_wait_for_gui_editors() {
        for ed in ["zed", "code", "code-insiders", "cursor", "windsurf", "subl"] {
            let (program, args) = parse(ed).unwrap();
            assert_eq!(program, ed);
            assert!(
                args.iter().any(|a| a == "--wait" || a == "-w"),
                "expected a wait flag for GUI editor {ed:?}, got {args:?}"
            );
        }
    }

    #[test]
    fn parse_editor_matches_gui_editor_by_basename() {
        let (program, args) = parse("/usr/local/bin/zed -n").unwrap();
        assert_eq!(program, "/usr/local/bin/zed");
        assert_eq!(args, vec!["-n", "--wait"]);
    }

    #[test]
    fn parse_editor_leaves_terminal_editors_untouched() {
        for ed in ["vi", "vim", "nvim", "nano", "emacs", "hx"] {
            let (program, args) = parse(ed).unwrap();
            assert_eq!(program, ed);
            assert!(args.is_empty(), "{ed:?} got {args:?}");
        }
    }

    #[test]
    fn parse_editor_keeps_an_existing_wait_flag() {
        assert_eq!(parse("code --wait").unwrap().1, vec!["--wait"]);
        assert_eq!(parse("subl -w").unwrap().1, vec!["-w"]);
    }

    #[test]
    fn parse_editor_blank_is_none() {
        assert!(parse("").is_none());
        assert!(parse("   ").is_none());
    }

    #[test]
    fn parse_editor_honours_quotes_and_keeps_backslashes() {
        let (program, args) =
            parse(r#""C:\Program Files\Notepad++\notepad++.exe" -multiInst 'two words'"#).unwrap();
        assert_eq!(program, r"C:\Program Files\Notepad++\notepad++.exe");
        assert_eq!(args, vec!["-multiInst", "two words"]);
    }

    #[test]
    fn parse_editor_takes_an_existing_unquoted_path_whole() {
        let path = r"C:\Program Files\Microsoft VS Code\bin\code.cmd";
        let (program, args) = parse_editor(path, |p| p == Path::new(path)).unwrap();
        assert_eq!(program, path);
        assert_eq!(args, vec!["--wait"], "still recognised as a GUI editor");
    }

    #[test]
    fn editor_setting_prefers_visual_then_editor_then_the_platform_default() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            editor_setting(env(&[("VISUAL", "zed"), ("EDITOR", "vim")]), false),
            "zed"
        );
        assert_eq!(
            editor_setting(env(&[("VISUAL", " "), ("EDITOR", "vim")]), false),
            "vim"
        );
        assert_eq!(editor_setting(env(&[]), false), "vi");
        assert_eq!(editor_setting(env(&[]), true), "notepad");
    }

    #[test]
    fn resolve_on_path_finds_a_cmd_shim_by_pathext() {
        let dirs = vec![PathBuf::from(r"C:\a"), PathBuf::from(r"C:\vscode\bin")];
        let target = PathBuf::from(r"C:\vscode\bin").join("code.cmd");
        let found = resolve_on_path("code", &dirs, ".COM;.EXE;.BAT;.CMD", |p| p == target);
        assert_eq!(found, target);
    }

    #[test]
    fn resolve_on_path_leaves_paths_extensions_and_misses_alone() {
        let dirs = vec![PathBuf::from(r"C:\bin")];
        let any = |_: &Path| true;
        assert_eq!(
            resolve_on_path(r"C:\tools\vim.exe", &dirs, ".EXE", any),
            PathBuf::from(r"C:\tools\vim.exe")
        );
        assert_eq!(
            resolve_on_path("vim.exe", &dirs, ".EXE", any),
            PathBuf::from("vim.exe")
        );
        assert_eq!(
            resolve_on_path("notepad", &dirs, ".EXE", |_| false),
            PathBuf::from("notepad")
        );
    }

    #[test]
    fn editor_is_gui_matches_known_gui_editors_and_rejects_terminal_ones() {
        for ed in [
            "zed",
            "code",
            "code-insiders",
            "codium",
            "vscodium",
            "cursor",
            "windsurf",
            "subl",
            "sublime_text",
            "/usr/local/bin/zed",
            r"C:\Tools\code.exe",
            r"C:\vscode\bin\code.cmd",
        ] {
            assert!(editor_is_gui(ed), "{ed} should be a GUI editor");
        }
        for ed in ["vi", "vim", "nvim", "nano", "emacs", "hx", "notepad"] {
            assert!(!editor_is_gui(ed), "{ed} should not be a GUI editor");
        }
    }

    /// The watch reads with the draft's size cap: an oversized buffer ends the session with a
    /// `ReadError`, not an unbounded read.
    #[test]
    fn watch_reports_an_oversized_buffer_instead_of_reading_it() {
        let scratch = ScratchDir::create("redact-test").unwrap();
        let big = vec![b'x'; crate::domain::MAX_TEXT_FILE_BYTES as usize + 1];
        let buffer = scratch.create_file("a.txt", &big).unwrap();
        // A program that exits at once stands in for the editor.
        let (program, args) = if cfg!(windows) {
            ("cmd", vec!["/C".to_string(), "exit".to_string()])
        } else {
            ("true", vec![])
        };
        let rx = spawn_upload_edit_watch(
            PathBuf::from(program),
            args,
            buffer,
            "g1".into(),
            "a.txt".into(),
        );
        let event = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert!(
            matches!(event, UploadEditWatchEvent::ReadError { .. }),
            "expected ReadError for an oversized buffer"
        );
    }
}
