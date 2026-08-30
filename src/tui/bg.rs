//! Background workers and the **job registry** (`Jobs`) for the TUI event loop.
//! Extracted from `run_loop` (issue #225); deepened into a single spawn/absorb API
//! so call sites do not own parallel channel fields by hand (issue #243).

use super::*;
use crate::actions::SystemRunner;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::PathBuf;

pub(super) enum UploadEditWatchEvent {
    /// The temp file's mtime changed — re-read and live-update the diff.
    ContentChanged {
        gist_id: String,
        filename: String,
        content: String,
    },
    /// The editor process exited; this is the final content, and the temp file has already
    /// been deleted by the sending thread.
    EditorClosed {
        gist_id: String,
        filename: String,
        content: String,
    },
    /// Either the editor failed to start, or the final read after it closed failed. The temp
    /// file has already been cleaned up (best-effort) by the sending thread.
    ReadError {
        gist_id: String,
        filename: String,
        message: String,
    },
}

pub(super) fn revision_version_label(revision: &crate::domain::GistRevision) -> String {
    let sha = crate::domain::short_sha(&revision.version);
    let age = crate::domain::parse_rfc3339_to_unix(&revision.committed_at)
        .map(|t| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            crate::domain::humanize_age(now as i64 - t as i64)
        })
        .unwrap_or_else(|| "?".into());
    format!("{sha} ({age} ago)")
}

pub(super) fn fetch_revision_incremental_pair(
    gist_id: &str,
    child_version: &str,
    parent_version: Option<&str>,
    filename: &str,
    owner_login: &str,
) -> std::result::Result<(String, String), String> {
    let new_content = ensure_fetched_text(
        crate::gh::fetch_revision_file_text_optional(
            &SystemRunner,
            gist_id,
            child_version,
            filename,
            owner_login,
        )
        .map_err(|e| e.to_string())?,
    )?;
    let old_content = match parent_version {
        Some(parent) => ensure_fetched_text(
            crate::gh::fetch_revision_file_text_optional(
                &SystemRunner,
                gist_id,
                parent,
                filename,
                owner_login,
            )
            .map_err(|e| e.to_string())?,
        )?,
        None => String::new(),
    };
    Ok((old_content, new_content))
}

pub(super) fn fetch_revision_pair(
    gist_id: &str,
    version: &str,
    filename: &str,
    raw_url: Option<&str>,
    owner_login: &str,
    _old_label: &str,
    _new_label: &str,
) -> std::result::Result<(String, String), String> {
    let old_content = ensure_fetched_text(
        crate::gh::fetch_revision_file_text(&SystemRunner, gist_id, version, filename, owner_login)
            .map_err(|e| e.to_string())?,
    )?;
    let new_content = fetch_gist_content(gist_id, filename, raw_url)?;
    Ok((old_content, new_content))
}

pub(super) fn fetch_gist_content(
    gist_id: &str,
    filename: &str,
    raw_url: Option<&str>,
) -> std::result::Result<String, String> {
    let content = crate::gh::fetch_gist_file_content(&SystemRunner, gist_id, filename, raw_url)
        .map_err(|e| e.to_string())?;
    crate::domain::ensure_text_size(content.len() as u64)?;
    Ok(content)
}

/// Cap revision-file text the same way as live gist content (issue #222).
pub(super) fn ensure_fetched_text(content: String) -> std::result::Result<String, String> {
    crate::domain::ensure_text_size(content.len() as u64)?;
    Ok(content)
}

pub(super) fn fetch_revision_pair_for_restore(
    gist_id: &str,
    version: &str,
    filename: &str,
    raw_url: Option<&str>,
    owner_login: &str,
) -> std::result::Result<(String, String), String> {
    fetch_revision_pair(gist_id, version, filename, raw_url, owner_login, "", "")
}

pub(super) fn persist_gist_cache_from_state(state: &AppState) {
    if let Ok(path) = crate::cache::cache_path() {
        crate::cache::save_gist_cache(&path, &state.gist_catalog);
    }
}

/// Off-thread: ask GitHub for the latest release tag and classify it against the running
/// version. Network failures map to `Failed` (silent; the loop won't record the throttle).
pub(super) fn spawn_update_check(
) -> std::sync::mpsc::Receiver<crate::update_check::UpdateCheckOutcome> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome =
            crate::update_check::check(&crate::upgrade::UreqClient, env!("CARGO_PKG_VERSION"));
        let _ = tx.send(outcome);
    });
    rx
}

/// Background local-scan result, paired with the generation active when `spawn_local_scan`
/// was called (issue #409 — the generation travels with the receiver, not the channel
/// payload, so a disconnect can still be checked against it without a value to unpack).
type LocalScanRx = Option<(
    u64,
    std::sync::mpsc::Receiver<Result<Vec<LocalCandidate>, String>>,
)>;

/// Run `request` off-thread. Errors are carried through, never converted to an empty list
/// (issue #409) — an empty result must mean "the scan really found nothing," not "it failed."
fn spawn_local_scan(
    request: local_scan::ScanRequest,
) -> std::sync::mpsc::Receiver<Result<Vec<LocalCandidate>, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(request.run().map_err(|e| e.to_string()));
    });
    rx
}

/// Work that ran off-thread, ready to apply on the event-loop tick (issue #375).
/// The boxed closure captures the worker's result; [`Jobs::on_action_outcome`]
/// calls it after the generation guard.
type ActionApply = Box<dyn FnOnce(&mut AppState) -> LoopFlow + Send>;

/// Background per-action apply, stamped with the generation active at spawn time.
type ActionRx = Option<std::sync::mpsc::Receiver<(u64, ActionApply)>>;

/// Initial newest-first comment load: probe the total, then fetch the newest page.
/// Thin IO boundary (network) — not unit-tested.
pub(super) fn load_initial_comments(gist_id: &str) -> Result<crate::tui::InitialComments, String> {
    let probe =
        crate::gh::fetch_gist_comments_probe(&SystemRunner, gist_id).map_err(|e| e.to_string())?;
    let total = crate::gh::comments_total_from_probe(&probe);
    if total == 0 {
        return Ok(crate::tui::InitialComments {
            comments: Vec::new(),
            total: 0,
            oldest_page: 1,
        });
    }
    let oldest_page = crate::gh::last_page(total, crate::gh::COMMENTS_PAGE_SIZE);
    let raw = crate::gh::fetch_gist_comments_page(
        &SystemRunner,
        gist_id,
        oldest_page,
        crate::gh::COMMENTS_PAGE_SIZE,
    )
    .map_err(|e| e.to_string())?;
    let comments = crate::gh::parse_gist_comments_json(&raw).map_err(|e| e.to_string())?;
    Ok(crate::tui::InitialComments {
        comments,
        total,
        oldest_page,
    })
}

/// Resolve a pin's absolute local path against cwd.
pub(super) fn pin_local_abs(state: &AppState, m: &crate::domain::PinnedMapping) -> PathBuf {
    if m.local_path.is_absolute() {
        m.local_path.clone()
    } else {
        state.cwd.join(&m.local_path)
    }
}

/// Spawn the push (upload local → gist) flow for a pin: lands in the existing
/// upload `Screen::Confirm` diff.
pub(super) fn spawn_pin_push(
    state: &mut AppState,
    jobs: &mut Jobs,
    m: &crate::domain::PinnedMapping,
    entry: crate::tui::DeferredEntry,
) {
    let local_path = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    // Upload Confirm returns to the screen captured by `entry`.
    let file = crate::domain::GistFileRef::id_name(gist_id, filename);
    let (file, local_label, gist_label) =
        screens::confirm::stage_upload_preview(state, local_path.clone(), file);
    jobs.spawn_gist_fetch_action(
        state,
        "Loading diff…",
        file,
        move |result, file, state| {
            screens::confirm::on_upload_preview(
                state,
                entry,
                result,
                file,
                local_path,
                local_label,
                gist_label,
            )
        },
    );
}

/// Spawn the pull (download gist → local) flow for a pin: lands in the existing
/// download `Screen::Confirm` diff when the local file exists.
pub(super) fn spawn_pin_pull(
    state: &mut AppState,
    jobs: &mut Jobs,
    m: &crate::domain::PinnedMapping,
    entry: crate::tui::DeferredEntry,
) {
    let target = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    let file = crate::domain::GistFileRef::id_name(gist_id, filename);
    let (file, local_label, gist_label) =
        screens::diff::stage_download_gist(state, target.clone(), file);
    jobs.spawn_gist_fetch_action(state, "Downloading…", file, move |result, file, state| {
        screens::diff::on_download_selected(
            state,
            entry,
            result,
            target,
            local_label,
            gist_label,
            file,
        )
    });
}

/// Spawn a read-only diff (gist vs local) for a pin, landing on `Screen::Diff`.
pub(super) fn spawn_pin_diff(
    state: &mut AppState,
    jobs: &mut Jobs,
    m: &crate::domain::PinnedMapping,
    entry: crate::tui::DeferredEntry,
) {
    let local_abs = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    let file = crate::domain::GistFileRef::id_name(gist_id, filename);
    let gist_file = file.clone();
    let (file, local_label, gist_label) =
        screens::diff::stage_preview_diff(state, Some(local_abs.clone()), file);
    let target = local_abs.clone();
    jobs.spawn_gist_fetch_action(
        state,
        "Loading diff…",
        file,
        move |result, _file, state| {
            // Pin diffs originate from the Pins screen (no focused pane); keep the
            // historical download orientation (old = local, new = gist).
            screens::diff::on_preview_diff(
                state,
                entry,
                result,
                Some(local_abs),
                local_label,
                gist_label,
                target,
                false,
                Some(gist_file),
            )
        },
    );
}

/// If `(local_abs, gist_id, filename)` is a pinned pair, record the sync result
/// (hash of `content` + `direction`) to config and update `state.pinned`.
pub(super) fn record_pin_sync(
    state: &mut AppState,
    local_abs: &std::path::Path,
    gist_id: &str,
    filename: &str,
    content: &str,
    direction: Option<crate::domain::SyncDirection>,
) {
    // Find the pin using its STORED (possibly relative) local_path form.
    let stored_local = state.pinned.iter().find_map(|m| {
        let mabs = pin_local_abs(state, m);
        (mabs == local_abs && m.gist_id == gist_id && m.gist_filename == filename)
            .then(|| m.local_path.clone())
    });
    let Some(stored_local) = stored_local else {
        return;
    };
    let hash = crate::domain::sha256_hex(content.as_bytes());
    if let Ok(path) = crate::config::config_path() {
        if let Ok(config) = crate::config::load_config(&path) {
            if let Ok(updated) = crate::actions::record_sync(
                &path,
                config,
                &stored_local,
                gist_id,
                filename,
                &hash,
                direction,
            ) {
                state.pinned = updated.pinned;
                state.mark_pin_sync_cache_dirty();
            }
        }
    }
}

/// Builds the `--- local` / `+++ gist` diff header labels showing each side's filename and
/// last-modified time, plus the gist's id.
pub(super) fn open_browser_gist(state: &mut AppState, gist_id: &str) {
    let plan = crate::actions::open_browser_command(gist_id);
    // Fire-and-forget on a detached thread: `gh gist view --web` resolves the URL and shells
    // out to the OS opener, which can stall the event loop for a perceptible window if run
    // inline. A launch failure is rare and self-evident (no browser appears), so we report
    // optimistically rather than thread the result back through a background outcome.
    std::thread::spawn(move || {
        let _ = crate::actions::execute_command(&plan);
    });
    state.set_status(format!("Opening gist {gist_id} in the browser…"));
}

pub(super) fn open_url(state: &mut AppState, url: &str, status: &str) {
    let plan = crate::actions::open_url_command(url);
    std::thread::spawn(move || {
        let _ = crate::actions::execute_command(&plan);
    });
    state.set_status(status);
}

/// Copy a gist's web URL (payload already resolved at key time — issue #244).
pub(super) fn copy_gist_url_id(state: &mut AppState, gist_id: &str) {
    let url = crate::actions::gist_web_url(gist_id);
    match crate::actions::copy_to_clipboard(&url) {
        Ok(_) => state.set_status(format!("Copied URL to clipboard: {url}")),
        Err(error) => state.set_status(format!("copy failed: {error}")),
    }
}

/// Copies the full previewed file content (the text shown on `Screen::Preview`) to the
/// system clipboard.
pub(super) fn copy_preview_content(state: &mut AppState) {
    let Some(text) = state.preview().map(|p| p.body.text.clone()) else {
        state.set_status("no content to copy");
        return;
    };
    if text.is_empty() {
        state.set_status("no content to copy");
        return;
    }
    let bytes = text.len();
    match crate::actions::copy_to_clipboard(&text) {
        Ok(_) => state.set_status(format!("Copied {bytes} bytes to clipboard")),
        Err(error) => state.set_status(format!("copy failed: {error}")),
    }
}

/// Whether `program`'s basename matches a known GUI editor that forks and returns
/// immediately (so it both needs `--wait` injected by `editor_command`, and — for the
/// upload-redact-buffer flow — can be watched non-blocking instead of taking over the
/// terminal). Keyed by basename so a full path or a `.exe` suffix still matches.
pub(super) fn editor_is_gui(program: &str) -> bool {
    // Extract basename handling both Unix (/) and Windows (\) separators, then strip .exe if present.
    let basename = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let base = std::path::Path::new(basename)
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

/// Split a `$VISUAL`/`$EDITOR` string into `(program, args)`, injecting a "wait" flag for
/// known GUI editors that fork and return immediately (`zed`, `code`, `cursor`, `subl`, …).
/// Without it `Command::status()` returns *before* the user saves, so the caller reads back
/// the stale, pre-edit buffer — which for the upload redact flow would silently publish the
/// **un-redacted** original. Terminal editors (`vi`, `nano`, `emacs -nw`) already block and
/// are left untouched. The file path is appended by the caller, so it always lands last.
/// Returns `None` only when the string is blank (no program).
pub(super) fn editor_command(editor: &str) -> Option<(String, Vec<String>)> {
    let mut parts = editor.split_whitespace();
    let program = parts.next()?.to_string();
    let mut args: Vec<String> = parts.map(str::to_string).collect();

    if editor_is_gui(&program) && !args.iter().any(|a| a == "--wait" || a == "-w") {
        args.push("--wait".to_string());
    }

    Some((program, args))
}

/// Opens `path` in `$VISUAL`/`$EDITOR` (default `vi`). A terminal editor needs the full
/// terminal, so the TUI leaves raw mode / the alternate screen for the duration and restores
/// afterwards. `$EDITOR` may include flags (e.g. `code --wait`); a wait flag is added
/// automatically for known GUI editors (see [`editor_command`]).
pub(super) fn edit_local_path(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    path: &std::path::Path,
) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let Some((program, args)) = editor_command(&editor) else {
        state.set_status("no editor configured (set $EDITOR)");
        return Ok(());
    };

    if state.settings.mouse_enabled() {
        execute!(terminal.backend_mut(), DisableMouseCapture)?;
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    let result = std::process::Command::new(program)
        .args(&args)
        .arg(path)
        .status();
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    if state.settings.mouse_enabled() {
        execute!(terminal.backend_mut(), EnableMouseCapture)?;
    }
    terminal.clear()?;

    match result {
        Ok(_) => state.set_status(format!("Edited {}", crate::config::display_path(path))),
        Err(error) => state.set_status(format!("editor failed: {error}")),
    }
    Ok(())
}

/// Watches `temp_file_path` while a non-blocking GUI-editor child process has it open,
/// sending a `ContentChanged` event on every detected save (polled every 500ms) and a
/// terminal `EditorClosed`/`ReadError` event once the editor exits or fails to start. Deletes
/// the temp file itself before returning — the caller never needs to clean up after this
/// thread. This is the non-blocking counterpart to the `Command::status()` call further down
/// in `edit_upload_buffer`, used only for editors `editor_is_gui` recognises.
pub(super) fn spawn_upload_edit_watch(
    program: String,
    args: Vec<String>,
    temp_file_path: PathBuf,
    gist_id: String,
    filename: String,
) -> std::sync::mpsc::Receiver<UploadEditWatchEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut child = match std::process::Command::new(&program)
            .args(&args)
            .arg(&temp_file_path)
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                let _ = tx.send(UploadEditWatchEvent::ReadError {
                    gist_id,
                    filename,
                    message: format!("editor failed to start: {e}"),
                });
                let _ = std::fs::remove_file(&temp_file_path);
                return;
            }
        };

        let mut last_modified = std::fs::metadata(&temp_file_path)
            .and_then(|m| m.modified())
            .ok();
        loop {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            if let Ok(modified) = std::fs::metadata(&temp_file_path).and_then(|m| m.modified()) {
                if Some(modified) != last_modified {
                    last_modified = Some(modified);
                    if let Ok(content) = std::fs::read_to_string(&temp_file_path) {
                        let _ = tx.send(UploadEditWatchEvent::ContentChanged {
                            gist_id: gist_id.clone(),
                            filename: filename.clone(),
                            content,
                        });
                    }
                }
            }
        }

        let final_event = match std::fs::read_to_string(&temp_file_path) {
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
        let _ = std::fs::remove_file(&temp_file_path);
    });
    rx
}

pub(super) fn edit_upload_buffer(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    jobs: &mut Jobs,
) -> Result<()> {
    let Some(local_path) = state.upload_local_path() else {
        return Ok(());
    };
    let Some(local_filename) = local_path.file_name().and_then(|n| n.to_str()) else {
        return Ok(());
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_file_path =
        std::env::temp_dir().join(format!(".gistui_redact_{timestamp}_{local_filename}"));

    let current_content = state.content_to_upload();
    if let Err(e) = std::fs::write(&temp_file_path, &current_content) {
        state.set_status(format!("failed to write temp file: {e}"));
        return Ok(());
    }

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let Some((program, args)) = editor_command(&editor) else {
        state.set_status("no editor configured (set $EDITOR)");
        let _ = std::fs::remove_file(&temp_file_path);
        return Ok(());
    };

    // GUI editors run in their own window, so gistui doesn't need the terminal back — spawn
    // non-blocking and watch the temp file for saves instead of blocking on Command::status().
    // Terminal editors (below) still need the full terminal and stay fully blocking.
    if editor_is_gui(&program) {
        let Some(PendingAction::Upload {
            gist_id,
            filename: gist_filename,
            ..
        }) = state.pending_action().cloned()
        else {
            let _ = std::fs::remove_file(&temp_file_path);
            return Ok(());
        };
        jobs.set_upload_edit_watch(spawn_upload_edit_watch(
            program,
            args,
            temp_file_path,
            gist_id,
            gist_filename,
        ));
        state.upload.watching = true;
        state.set_status("Editing in external editor — diff updates live");
        return Ok(());
    }

    if state.settings.mouse_enabled() {
        execute!(terminal.backend_mut(), DisableMouseCapture)?;
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    let result = std::process::Command::new(program)
        .args(&args)
        .arg(&temp_file_path)
        .status();
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    if state.settings.mouse_enabled() {
        execute!(terminal.backend_mut(), EnableMouseCapture)?;
    }
    terminal.clear()?;

    match result {
        Ok(_) => match std::fs::read_to_string(&temp_file_path) {
            Ok(edited_content) => {
                state.upload.edited_content = Some(edited_content);
                state.update_upload_diff();
                state.set_status("Edited redact buffer");
            }
            Err(e) => state.set_status(format!("failed to read edited file: {e}")),
        },
        Err(error) => state.set_status(format!("editor failed: {error}")),
    }

    let _ = std::fs::remove_file(&temp_file_path);
    Ok(())
}

/// Create a scratch dir and write `body` to `filename` inside it, setting a status message
/// and returning `None` on either failure — the caller owns the early return (`ScratchDir`
/// cleanup on early failure, or ownership moving into a bg job on success, per issue #275).
/// `context` names the file for the write-failure message (e.g. "temp file"); the
/// create-dir failure message is the same regardless of caller.
pub(super) fn write_scratch_file(
    state: &mut AppState,
    label: &str,
    filename: &str,
    context: &str,
    body: &[u8],
) -> Option<(crate::temp_dir::ScratchDir, PathBuf)> {
    let scratch = match crate::temp_dir::ScratchDir::create(label) {
        Ok(dir) => dir,
        Err(e) => {
            state.set_status(format!("failed to create temp dir: {e}"));
            return None;
        }
    };
    let path = scratch.path().join(filename);
    if let Err(e) = std::fs::write(&path, body) {
        state.set_status(format!("failed to write {context}: {e}"));
        return None;
    }
    Some((scratch, path))
}

pub(super) fn download(state: &mut AppState, mode: crate::actions::DownloadMode) {
    let target = state.download_target();
    let content = state.preview_remote().to_string();
    let pin_key = state
        .diff()
        .and_then(|d| match (&d.gist_id, &d.gist_filename) {
            (Some(g), Some(f)) => Some((g.clone(), f.clone())),
            _ => None,
        });
    match crate::actions::execute_download(&target, &content, mode) {
        Ok(()) => {
            state.set_status(format!(
                "Downloaded {}",
                target
                    .file_name()
                    .unwrap_or(target.as_os_str())
                    .to_string_lossy()
            ));
            if let Some((gid, fname)) = pin_key {
                record_pin_sync(
                    state,
                    &target,
                    &gid,
                    &fname,
                    &content,
                    Some(crate::domain::SyncDirection::Download),
                );
            }
            // Skip past the download overwrite gate's Confirm (if any) and its parked Diff to
            // land on whatever was behind them.
            if state.screen.is_confirm() {
                state.leave();
            }
            if state.screen.is_diff() {
                state.leave();
            }
            refresh_locals(state, Some(&target));
        }
        Err(error) => {
            state.set_status(format!("download failed: {error}"));
            state.cancel_confirm_to_diff();
        }
    }
}

/// Synchronous local re-scan after a successful download, using the active recursive mode
/// (issue #409) so the just-downloaded file is visible immediately without waiting for an
/// interactive scan. Supersedes any scan already in flight. On failure the last-known-good
/// candidates and selection are kept, and the failure is appended to whatever status the
/// caller already set — e.g. "Downloaded a.txt; local refresh failed: …" — instead of
/// overwriting it.
pub(super) fn refresh_locals(state: &mut AppState, target: Option<&std::path::Path>) {
    let request =
        state.local_scan_request(local_scan::ScanMode::from_active(state.local_recursive));
    let generation = state.begin_local_scan();
    match request.run() {
        Ok(candidates) => {
            state.apply_local_scan(generation, candidates, target);
        }
        Err(error) => {
            state.end_local_scan(generation);
            append_status(state, format!("local refresh failed: {error}"));
        }
    }
}

/// Append a fact to the current status instead of overwriting it — so a synchronous
/// local-scan failure never erases feedback a caller already set (issue #409).
fn append_status(state: &mut AppState, message: impl Into<String>) {
    let message = message.into();
    state.status = Some(match state.status.take() {
        Some(existing) if !existing.is_empty() => format!("{existing}; {message}"),
        _ => message,
    });
}

/// Persist Settings-screen fields after a user change (issue #227). Creates config.toml
/// only when a value actually changed (opening Config never calls this).
pub(super) fn persist_settings(state: &mut AppState, success_message: String) {
    let result = crate::config::config_path().and_then(|path| {
        let mut config = crate::config::load_config(&path)?;
        state.settings.apply_to_config(&mut config);
        crate::config::save_config(&path, &config)?;
        Ok(())
    });
    match result {
        Ok(()) => state.set_status(success_message),
        Err(error) => state.set_status(format!("save config failed: {error}")),
    }
}

/// Whether [`sync_mouse_capture`] should call crossterm (false in unit tests / non-TTY).
pub(super) fn mouse_capture_applies_to_stdout() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Apply crossterm mouse capture to match `enabled` (Settings toggle must take effect
/// without restart). No-ops when stdout is not a TTY so unit tests never hang.
pub(super) fn sync_mouse_capture(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    enabled: bool,
) -> Result<()> {
    if !mouse_capture_applies_to_stdout() {
        return Ok(());
    }
    if enabled {
        execute!(terminal.backend_mut(), EnableMouseCapture)?;
    } else {
        execute!(terminal.backend_mut(), DisableMouseCapture)?;
    }
    Ok(())
}

pub(super) fn pin_paths(
    state: &mut AppState,
    local_path: &std::path::Path,
    gist_id: &str,
    filename: &str,
) {
    let gist = GistFile::for_sync(gist_id.to_string(), filename.to_string(), None);
    let result = crate::config::config_path().and_then(|path| {
        let config = crate::config::load_config(&path)?;
        crate::actions::pin_mapping(&path, config, local_path, &gist, None, None)
    });
    match result {
        Ok(config) => {
            state.pinned = config.pinned;
            state.skip_dirs = config.skip_dirs;
            state.mark_pin_sync_cache_dirty();
            state.set_status(format!("Pinned {} <-> {}", local_path.display(), filename));
        }
        Err(error) => state.set_status(format!("pin failed: {error}")),
    }
}

pub(super) fn unpin_path(
    state: &mut AppState,
    local_path: &std::path::Path,
    gist_id: &str,
    filename: &str,
) {
    let result = crate::config::config_path().and_then(|path| {
        let config = crate::config::load_config(&path)?;
        crate::actions::unpin_mapping(
            &path,
            config,
            crate::pins::PinKey::new(local_path, gist_id, filename),
        )
    });
    match result {
        Ok(config) => {
            state.pinned = config.pinned;
            state.skip_dirs = config.skip_dirs;
            state.mark_pin_sync_cache_dirty();
            state.set_status(format!(
                "Unpinned {} <-> {}",
                crate::config::display_path(local_path),
                filename
            ));
        }
        Err(error) => state.set_status(format!("unpin failed: {error}")),
    }
}

pub(super) fn unpin_at_pin_index(state: &mut AppState, idx: usize) {
    if idx >= state.pinned.len() {
        return;
    }
    let mapping = state.pinned[idx].clone();
    let label = crate::config::display_path(&mapping.local_path);
    let result = crate::config::config_path().and_then(|path| {
        let config = crate::config::load_config(&path)?;
        crate::actions::unpin_mapping(&path, config, mapping.key())
    });
    match result {
        Ok(config) => {
            state.pinned = config.pinned;
            state.skip_dirs = config.skip_dirs;
            state.mark_pin_sync_cache_dirty();
            let len = state.visible_pin_indices().len();
            if let Some(pins) = state.pins_mut() {
                pins.cursor.clamp_len(len);
            }
            // No filesystem rescan: unpin never touches the filesystem, and ranking reads
            // `PinnedMapping` directly — a forced-flat rescan here used to make the local
            // list drift back to cwd-only even while recursive mode was active (issue #409).
            state.set_status(format!("Unpinned {label} <-> {}", mapping.gist_filename));
        }
        Err(error) => state.set_status(format!("unpin failed: {error}")),
    }
}

/// Background job registry (issue #243): spawn / absorb / cancel. Apply handlers live
/// on the screen (or gist-mutation) module that owns the state they mutate (issue #383).
///
/// Call sites start work via methods on this type; the event loop only polls
/// [`Jobs::absorb`]. Receivers stay private so new job kinds extend the registry
/// in one place.
///
/// # Generation / supersession
///
/// - **Action jobs** ([`Jobs::spawn_action`] / Esc via [`Jobs::cancel_action`]): each
///   spawn stamps `AppState::bg_task_generation`. Only matching generations apply;
///   cancel bumps the generation and drops the receiver (issue #221).
/// - **Local scans** ([`Jobs::request_local_scan`]): generation/in-flight lifecycle lives on
///   `AppState`'s private `local_scan` (see `local_scan.rs`, issue #409).
/// - **Gist refreshes** own one generation across their base and enrichment jobs.
pub(super) struct Jobs {
    update: Option<std::sync::mpsc::Receiver<crate::update_check::UpdateCheckOutcome>>,
    gist_refresh: super::gist_refresh::GistRefresh,
    local: LocalScanRx,
    /// Streams `UploadEditWatchEvent`s while a GUI editor has the upload-redact temp file
    /// open (see `spawn_upload_edit_watch`). Unlike one-shot slots, this channel can carry
    /// multiple `ContentChanged` events before its terminal `EditorClosed`/`ReadError`.
    upload_edit_watch: Option<std::sync::mpsc::Receiver<UploadEditWatchEvent>>,
    action: ActionRx,
}

pub(super) enum LoopFlow {
    Proceed,
    SkipIteration,
    Quit,
}

impl Jobs {
    /// Startup registry: optional update-check receiver and initial gist list fetch.
    pub(super) fn startup(
        update: Option<std::sync::mpsc::Receiver<crate::update_check::UpdateCheckOutcome>>,
        fetch_gists: bool,
        catalog: &crate::domain::GistCatalog,
    ) -> Self {
        Self {
            update,
            gist_refresh: super::gist_refresh::GistRefresh::new(catalog, fetch_gists),
            local: None,
            upload_edit_watch: None,
            action: None,
        }
    }

    /// Run `run` on a background thread; apply `apply(value)` on the event-loop tick.
    /// Sets `bg_task_msg` and stamps the result with the current action-job generation
    /// (issue #221). `run` must not touch `AppState`; `apply` is the only place that does
    /// (issue #375).
    pub(super) fn spawn_action<T, R, A>(
        &mut self,
        state: &mut AppState,
        msg: impl Into<String>,
        run: R,
        apply: A,
    ) where
        T: Send + 'static,
        R: FnOnce() -> T + Send + 'static,
        A: FnOnce(T, &mut AppState) -> LoopFlow + Send + 'static,
    {
        let generation = state.begin_bg_task();
        state.bg_task_msg = Some(msg.into());
        let (tx, rx) = std::sync::mpsc::channel();
        self.action = Some(rx);
        std::thread::spawn(move || {
            let value = run();
            let boxed: ActionApply = Box::new(move |state| apply(value, state));
            let _ = tx.send((generation, boxed));
        });
    }

    /// Spawn a background job that fetches a gist file's content, then hands the result
    /// (and the file identity back) to `apply`. Collapses the `fetch_gist_content`
    /// template shared by preview/download/upload across `dispatch.rs` and the pin-spawn
    /// helpers below (issue #299). `apply` gets `file` back so call sites that store it
    /// (all but preview-diff) don't need a second clone.
    pub(super) fn spawn_gist_fetch_action<A>(
        &mut self,
        state: &mut AppState,
        msg: impl Into<String>,
        file: crate::domain::GistFileRef,
        apply: A,
    ) where
        A: FnOnce(
                std::result::Result<String, String>,
                crate::domain::GistFileRef,
                &mut AppState,
            ) -> LoopFlow
            + Send
            + 'static,
    {
        self.spawn_action(
            state,
            msg,
            move || {
                let result =
                    fetch_gist_content(&file.gist_id, &file.filename, file.raw_url.as_deref());
                (result, file)
            },
            move |(result, file), state| apply(result, file, state),
        );
    }

    /// Esc cancel: drop the action receiver and invalidate generation so a late completion
    /// cannot mutate state.
    pub(super) fn cancel_action(&mut self, state: &mut AppState) {
        state.invalidate_bg_task();
        self.action = None;
        state.set_status("Cancelled");
    }

    /// Start a local file scan stamped with a new generation.
    pub(super) fn request_local_scan(&mut self, state: &mut AppState) {
        let request =
            state.local_scan_request(local_scan::ScanMode::from_active(state.local_recursive));
        let generation = state.begin_local_scan();
        state.set_status(local_scan::SCANNING_STATUS);
        self.local = Some((generation, spawn_local_scan(request)));
    }

    pub(super) fn set_upload_edit_watch(
        &mut self,
        rx: std::sync::mpsc::Receiver<UploadEditWatchEvent>,
    ) {
        self.upload_edit_watch = Some(rx);
    }

    /// Poll ready job completions and apply them to `state`.
    pub(super) fn absorb(
        &mut self,
        state: &mut AppState,
        update_check_path: &Option<std::path::PathBuf>,
    ) -> Result<LoopFlow> {
        self.absorb_inner(state, update_check_path)
    }

    /// Poll each job module in turn and apply ready results to `state`.
    fn absorb_inner(
        &mut self,
        state: &mut AppState,
        update_check_path: &Option<std::path::PathBuf>,
    ) -> Result<LoopFlow> {
        self.on_gist_refresh_ready(state);
        self.on_local_scan_ready(state);
        self.on_update_check_ready(state, update_check_path);
        self.on_upload_watch_events(state);
        let flow = self.on_action_outcome(state);
        if std::mem::take(&mut state.gist_list_stale) {
            state.loading = true;
            self.gist_refresh.start(&state.gist_catalog);
        }
        if let Some(gist_id) = state.revisions_stale.take() {
            screens::revisions::request_revisions(self, state, gist_id);
        }
        Ok(flow)
    }
}

/// Receive at most one value from a one-shot job channel, clearing `slot` once a value
/// arrives. A disconnected sender is treated the same as an empty channel (no value yet):
/// `slot` is left in place and the caller tries again next tick.
///
/// Only covers channels that receive a single unstamped result (`update`). The
/// generation-stamped refresh, local, and action channels and the multi-message
/// `upload_edit_watch` drain have different shapes and stay hand-rolled.
fn poll_channel<T>(slot: &mut Option<std::sync::mpsc::Receiver<T>>) -> Option<T> {
    let value = slot.as_ref()?.try_recv().ok()?;
    *slot = None;
    Some(value)
}

impl Jobs {
    fn on_gist_refresh_ready(&mut self, state: &mut AppState) {
        for update in self.gist_refresh.poll() {
            state.gist_catalog = update.catalog;
            if update.persist {
                persist_gist_cache_from_state(state);
            }
            if let Some(status) = update.status {
                state.set_status(status);
            }
            if update.base_ready {
                state.loading = false;
                if state.gist_cursor.index >= state.ranked_gists().len() {
                    // Snapping to the top is a selection change, so the offset the old
                    // row was scrolled to goes with it (issue #415).
                    state.gist_cursor.reset();
                }
                let count = state.visible_gist_groups().len();
                if let Some(gm) = state.gist_manager_mut() {
                    gm.cursor.clamp_len(count);
                }
            }
        }
    }

    /// Absorb a completed background local scan (ignore stale generations — issue #221).
    fn on_local_scan_ready(&mut self, state: &mut AppState) {
        let Some((generation, rx)) = self.local.as_ref() else {
            return;
        };
        let generation = *generation;
        // A disconnected worker (panicked thread) is a failure too — otherwise a current
        // generation's spinner would spin forever with no way to end it (issue #409).
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("worker disconnected".to_string())
            }
        };
        self.local = None;
        match result {
            Ok(candidates) => {
                // A stale generation changes no candidates, spinner, or status.
                if state.apply_local_scan(generation, candidates, None) {
                    state.clear_scan_status();
                }
            }
            Err(error) => {
                if state.end_local_scan(generation) {
                    state.set_status(format!("local scan failed: {error}"));
                }
            }
        }
    }

    /// Absorb the background update-check result: show the hint and persist the throttle.
    /// Failed checks are silent and not recorded, so they retry on the next launch.
    fn on_update_check_ready(
        &mut self,
        state: &mut AppState,
        update_check_path: &Option<std::path::PathBuf>,
    ) {
        if let Some(outcome) = poll_channel(&mut self.update) {
            let now = crate::update_check::now_secs();
            match outcome {
                crate::update_check::UpdateCheckOutcome::Newer(version) => {
                    if let Some(ref path) = update_check_path {
                        crate::update_check::save_state(
                            path,
                            &crate::update_check::UpdateCheckState {
                                last_check: now,
                                latest_seen: version.clone(),
                            },
                        );
                    }
                    state.update_available = Some(version);
                }
                crate::update_check::UpdateCheckOutcome::UpToDate => {
                    if let Some(ref path) = update_check_path {
                        crate::update_check::save_state(
                            path,
                            &crate::update_check::UpdateCheckState {
                                last_check: now,
                                latest_seen: String::new(),
                            },
                        );
                    }
                    state.update_available = None;
                }
                crate::update_check::UpdateCheckOutcome::Failed => {}
            }
        }
    }

    /// Absorb upload-edit-watch events. Unlike the other channels above (one-shot), this one
    /// can carry several `ContentChanged` events before its terminal EditorClosed/ReadError —
    /// drain all of them so a burst of saves doesn't lag a tick behind.
    fn on_upload_watch_events(&mut self, state: &mut AppState) {
        let mut upload_watch_finished = false;
        if let Some(ref rx) = self.upload_edit_watch {
            while let Ok(event) = rx.try_recv() {
                if matches!(
                    event,
                    UploadEditWatchEvent::EditorClosed { .. }
                        | UploadEditWatchEvent::ReadError { .. }
                ) {
                    upload_watch_finished = true;
                }
                state.apply_upload_edit_event(event);
                if upload_watch_finished {
                    break;
                }
            }
        }
        if upload_watch_finished {
            self.upload_edit_watch = None;
        }
    }

    /// Absorb a completed background per-action task (ignore stale generations — issue #221).
    /// Returns [`LoopFlow::SkipIteration`] when the outcome should abort the rest of this
    /// event-loop tick (stale revision fetch / no-op restore); otherwise `LoopFlow::Proceed`.
    ///
    /// A router shell: generation guard, then the apply closure the job carried (issue #375).
    fn on_action_outcome(&mut self, state: &mut AppState) -> LoopFlow {
        let Some((generation, apply)) = self.action.as_ref().and_then(|rx| rx.try_recv().ok())
        else {
            return LoopFlow::Proceed;
        };
        self.action = None;
        if state.is_current_bg_generation(generation) {
            state.bg_task_msg = None;
            apply(state)
        } else {
            // Stale outcomes are dropped without applying.
            LoopFlow::Proceed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::GistCatalog;
    use crate::tui::gist_refresh::GistRefresh;

    use std::path::PathBuf;
    use std::sync::mpsc;

    // ---- poll_channel -------------------------------------------------

    #[test]
    fn returns_none_and_leaves_slot_when_empty() {
        let (_tx, rx) = mpsc::channel::<i32>();
        let mut slot = Some(rx);
        assert_eq!(poll_channel(&mut slot), None);
        assert!(slot.is_some());
    }

    #[test]
    fn returns_value_and_clears_slot_when_received() {
        let (tx, rx) = mpsc::channel::<i32>();
        tx.send(42).unwrap();
        let mut slot = Some(rx);
        assert_eq!(poll_channel(&mut slot), Some(42));
        assert!(slot.is_none());
    }

    #[test]
    fn treats_disconnected_sender_like_empty() {
        let (tx, rx) = mpsc::channel::<i32>();
        drop(tx);
        let mut slot = Some(rx);
        assert_eq!(poll_channel(&mut slot), None);
        assert!(slot.is_some());
    }

    #[test]
    fn returns_none_when_slot_already_empty() {
        let mut slot: Option<mpsc::Receiver<i32>> = None;
        assert_eq!(poll_channel(&mut slot), None);
    }

    // ---- test helpers ---------------------------------------------------

    /// Empty `Jobs` registry — every slot `None`. Tests populate only the slot under test.
    fn empty_jobs() -> Jobs {
        Jobs {
            update: None,
            gist_refresh: GistRefresh::new(&GistCatalog::default(), false),
            local: None,
            upload_edit_watch: None,
            action: None,
        }
    }

    // ---- on_local_scan_ready ----------------------------------------------

    #[test]
    fn on_local_scan_ready_noop_when_no_scan_is_in_flight() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_local_scan_ready(&mut state);

        assert!(jobs.local.is_none());
        assert!(!state.local_scanning());
    }

    #[test]
    fn on_local_scan_ready_applies_current_generation_and_clears_its_own_status_only() {
        let mut state = initial_state();
        let generation = state.begin_local_scan();
        state.status = Some(local_scan::SCANNING_STATUS.into());
        let candidate = LocalCandidate {
            path: PathBuf::from("a.txt"),
            modified: None,
        };
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(vec![candidate.clone()])).unwrap();
        let mut jobs = empty_jobs();
        jobs.local = Some((generation, rx));

        jobs.on_local_scan_ready(&mut state);

        assert!(jobs.local.is_none());
        assert!(!state.local_scanning());
        assert_eq!(state.locals, vec![candidate]);
        assert!(state.status.is_none());
    }

    /// Success must not clobber a status a newer action set after the scan started
    /// (issue #409).
    #[test]
    fn on_local_scan_ready_success_preserves_a_newer_status() {
        let mut state = initial_state();
        let generation = state.begin_local_scan();
        state.status = Some("a newer action's status".into());
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(Vec::<LocalCandidate>::new())).unwrap();
        let mut jobs = empty_jobs();
        jobs.local = Some((generation, rx));

        jobs.on_local_scan_ready(&mut state);

        assert_eq!(state.status.as_deref(), Some("a newer action's status"));
    }

    #[test]
    fn on_local_scan_ready_ignores_stale_generation() {
        let mut state = initial_state();
        let stale = state.begin_local_scan();
        let _current = state.begin_local_scan();
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(vec![LocalCandidate {
            path: PathBuf::from("stale.txt"),
            modified: None,
        }]))
        .unwrap();
        let mut jobs = empty_jobs();
        jobs.local = Some((stale, rx));

        jobs.on_local_scan_ready(&mut state);

        // The stale result is drained off the channel but not applied — spinner/list
        // stay as they were (a newer scan is still expected).
        assert!(jobs.local.is_none());
        assert!(state.local_scanning());
        assert!(state.locals.is_empty());
    }

    /// A current-generation failure keeps last-known-good candidates, ends the spinner, and
    /// reports the error (issue #409).
    #[test]
    fn on_local_scan_ready_current_failure_keeps_candidates_and_reports_error() {
        let mut state = initial_state();
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("kept.txt"),
            modified: None,
        }];
        let generation = state.begin_local_scan();
        let (tx, rx) = mpsc::channel();
        tx.send(Err("permission denied".to_string())).unwrap();
        let mut jobs = empty_jobs();
        jobs.local = Some((generation, rx));

        jobs.on_local_scan_ready(&mut state);

        assert!(!state.local_scanning());
        assert_eq!(state.locals.len(), 1);
        assert_eq!(state.locals[0].path, PathBuf::from("kept.txt"));
        assert_eq!(
            state.status.as_deref(),
            Some("local scan failed: permission denied")
        );
    }

    /// A disconnected worker (panicked thread) must still end the spinner instead of
    /// spinning forever (issue #409).
    #[test]
    fn on_local_scan_ready_current_disconnect_ends_the_spinner() {
        let mut state = initial_state();
        let generation = state.begin_local_scan();
        let (tx, rx) = mpsc::channel::<Result<Vec<LocalCandidate>, String>>();
        drop(tx);
        let mut jobs = empty_jobs();
        jobs.local = Some((generation, rx));

        jobs.on_local_scan_ready(&mut state);

        assert!(jobs.local.is_none());
        assert!(!state.local_scanning());
        assert!(state.status.is_some());
    }

    /// A stale generation's disconnect must not touch the current scan's spinner or status
    /// (issue #409).
    #[test]
    fn on_local_scan_ready_stale_disconnect_is_ignored() {
        let mut state = initial_state();
        let stale = state.begin_local_scan();
        let _current = state.begin_local_scan();
        let (tx, rx) = mpsc::channel::<Result<Vec<LocalCandidate>, String>>();
        drop(tx);
        let mut jobs = empty_jobs();
        jobs.local = Some((stale, rx));

        jobs.on_local_scan_ready(&mut state);

        assert!(jobs.local.is_none());
        assert!(state.local_scanning(), "current generation still in flight");
        assert!(state.status.is_none());
    }

    // ---- on_update_check_ready ---------------------------------------------

    #[test]
    fn on_update_check_ready_noop_when_channel_empty() {
        let mut state = initial_state();
        let (_tx, rx) = mpsc::channel();
        let mut jobs = empty_jobs();
        jobs.update = Some(rx);

        jobs.on_update_check_ready(&mut state, &None);

        assert!(jobs.update.is_some());
        assert!(state.update_available.is_none());
    }

    #[test]
    fn on_update_check_ready_newer_persists_and_sets_available() {
        let mut state = initial_state();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update_check.json");
        let (tx, rx) = mpsc::channel();
        tx.send(crate::update_check::UpdateCheckOutcome::Newer(
            "1.2.3".into(),
        ))
        .unwrap();
        let mut jobs = empty_jobs();
        jobs.update = Some(rx);

        jobs.on_update_check_ready(&mut state, &Some(path.clone()));

        assert_eq!(state.update_available.as_deref(), Some("1.2.3"));
        let saved = crate::update_check::load_state(&path);
        assert_eq!(saved.latest_seen, "1.2.3");
    }

    #[test]
    fn on_update_check_ready_up_to_date_clears_available_and_persists_empty() {
        let mut state = initial_state();
        state.update_available = Some("old".into());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update_check.json");
        let (tx, rx) = mpsc::channel();
        tx.send(crate::update_check::UpdateCheckOutcome::UpToDate)
            .unwrap();
        let mut jobs = empty_jobs();
        jobs.update = Some(rx);

        jobs.on_update_check_ready(&mut state, &Some(path.clone()));

        assert!(state.update_available.is_none());
        let saved = crate::update_check::load_state(&path);
        assert_eq!(saved.latest_seen, "");
    }

    #[test]
    fn on_update_check_ready_failed_is_silent() {
        let mut state = initial_state();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update_check.json");
        let (tx, rx) = mpsc::channel();
        tx.send(crate::update_check::UpdateCheckOutcome::Failed)
            .unwrap();
        let mut jobs = empty_jobs();
        jobs.update = Some(rx);

        jobs.on_update_check_ready(&mut state, &Some(path.clone()));

        assert!(state.update_available.is_none());
        assert!(!path.exists());
    }

    // ---- on_upload_watch_events ---------------------------------------------

    #[test]
    fn on_upload_watch_events_drains_until_terminal_event() {
        let mut state = initial_state();
        state.upload.watching = true;
        state.enter_confirm(
            PendingAction::Upload {
                gist_id: "g1".into(),
                filename: "a.txt".into(),
                local_path: PathBuf::from("a.txt"),
            },
            String::new(),
        );
        let (tx, rx) = mpsc::channel();
        tx.send(UploadEditWatchEvent::ContentChanged {
            gist_id: "g1".into(),
            filename: "a.txt".into(),
            content: "one".into(),
        })
        .unwrap();
        tx.send(UploadEditWatchEvent::EditorClosed {
            gist_id: "g1".into(),
            filename: "a.txt".into(),
            content: "two".into(),
        })
        .unwrap();
        let mut jobs = empty_jobs();
        jobs.upload_edit_watch = Some(rx);

        jobs.on_upload_watch_events(&mut state);

        assert!(jobs.upload_edit_watch.is_none());
        assert!(!state.upload.watching);
        assert_eq!(state.upload.edited_content.as_deref(), Some("two"));
    }

    // ---- on_action_outcome: generation guard -------------------------------

    #[test]
    fn on_action_outcome_ignores_stale_generation() {
        let mut state = initial_state();
        let stale = state.begin_bg_task();
        let _current = state.begin_bg_task();
        state.bg_task_msg = Some("Deleting gist…".into());
        let (tx, rx) = mpsc::channel();
        tx.send((
            stale,
            Box::new(|state: &mut AppState| {
                gist_mutation::on_delete_gist(state, Ok(()), "g1".into())
            }) as ActionApply,
        ))
        .unwrap();
        let mut jobs = empty_jobs();
        jobs.action = Some(rx);

        let flow = jobs.on_action_outcome(&mut state);

        assert!(matches!(flow, LoopFlow::Proceed));
        assert!(jobs.action.is_none());
        // Stale outcome dropped without applying — `bg_task_msg` untouched and no
        // follow-up gist fetch spawned.
        assert_eq!(state.bg_task_msg.as_deref(), Some("Deleting gist…"));
        assert!(!state.gist_list_stale);
        assert!(state.status.is_none());
    }

    // ---- refresh_locals -------------------------------------------------

    #[test]
    fn refresh_locals_preserves_nested_selection_in_recursive_mode() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let target = nested.join("settings.json");
        std::fs::write(&target, "body").unwrap();
        let compared = nested.join("local.json");
        std::fs::write(&compared, "local").unwrap();
        let mut state = initial_state();
        state.cwd = dir.path().to_path_buf();
        state.local_recursive = true;
        state.locals = vec![crate::domain::LocalCandidate {
            path: compared,
            modified: None,
        }];

        refresh_locals(&mut state, Some(&target));

        assert_eq!(state.selected_local().map(|file| file.path), Some(target));
    }

    /// A failed synchronous refresh keeps last-known-good candidates and appends its own
    /// failure onto whatever status the caller already set (issue #409).
    #[test]
    fn refresh_locals_failure_keeps_candidates_and_appends_to_the_existing_status() {
        let mut state = initial_state();
        // A cwd that cannot be scanned (never created) makes discovery fail.
        state.cwd = tempfile::tempdir().unwrap().path().join("does-not-exist");
        state.locals = vec![crate::domain::LocalCandidate {
            path: PathBuf::from("kept.txt"),
            modified: None,
        }];
        state.status = Some("Downloaded a.txt".into());

        refresh_locals(&mut state, None);

        assert_eq!(state.locals.len(), 1);
        assert_eq!(state.locals[0].path, PathBuf::from("kept.txt"));
        assert!(
            state
                .status
                .as_deref()
                .is_some_and(|s| s.starts_with("Downloaded a.txt; local refresh failed: ")),
            "status was {:?}",
            state.status
        );
    }

    #[test]
    fn editor_command_injects_wait_for_gui_editors() {
        for ed in ["zed", "code", "code-insiders", "cursor", "windsurf", "subl"] {
            let (program, args) = editor_command(ed).unwrap();
            assert_eq!(program, ed);
            assert!(
                args.iter().any(|a| a == "--wait" || a == "-w"),
                "expected a wait flag for GUI editor {ed:?}, got {args:?}"
            );
        }
    }

    #[test]
    fn editor_command_matches_gui_editor_by_basename() {
        // A full path or a `.exe` suffix must still be recognised as a GUI editor.
        let (program, args) = editor_command("/usr/local/bin/zed -n").unwrap();
        assert_eq!(program, "/usr/local/bin/zed");
        assert_eq!(args, vec!["-n", "--wait"]);
    }

    #[test]
    fn editor_command_leaves_terminal_editors_untouched() {
        for ed in ["vi", "vim", "nvim", "nano", "emacs", "hx"] {
            let (program, args) = editor_command(ed).unwrap();
            assert_eq!(program, ed);
            assert!(
                args.is_empty(),
                "terminal editor {ed:?} should get no injected flag, got {args:?}"
            );
        }
    }

    #[test]
    fn editor_command_keeps_an_existing_wait_flag() {
        // Don't duplicate a wait flag the user already configured (either spelling).
        let (_, args) = editor_command("code --wait").unwrap();
        assert_eq!(args, vec!["--wait"]);
        let (_, args) = editor_command("subl -w").unwrap();
        assert_eq!(args, vec!["-w"]);
    }

    #[test]
    fn editor_command_blank_is_none() {
        assert!(editor_command("").is_none());
        assert!(editor_command("   ").is_none());
    }

    #[test]
    fn editor_is_gui_matches_known_gui_editors() {
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
        ] {
            assert!(
                editor_is_gui(ed),
                "{ed} should be recognised as a GUI editor"
            );
        }
    }

    #[test]
    fn editor_is_gui_rejects_terminal_editors() {
        for ed in ["vi", "vim", "nvim", "nano", "emacs", "hx"] {
            assert!(
                !editor_is_gui(ed),
                "{ed} should not be recognised as a GUI editor"
            );
        }
    }

    #[test]
    fn editor_is_gui_matches_by_basename_from_full_path() {
        assert!(editor_is_gui("/usr/local/bin/zed"));
        assert!(editor_is_gui("C:\\Tools\\code.exe"));
    }

    #[test]
    fn mouse_capture_applies_to_stdout_matches_is_terminal() {
        // Guard used by sync_mouse_capture: must agree with std's TTY check so CI
        // (non-TTY) skips execute! and real sessions still apply capture.
        use std::io::IsTerminal;
        assert_eq!(
            mouse_capture_applies_to_stdout(),
            std::io::stdout().is_terminal()
        );
    }
}
