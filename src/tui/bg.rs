//! Background workers and the **job registry** (`Jobs`) for the TUI event loop.
//! Extracted from `run_loop` (issue #225); deepened into a single spawn/absorb API
//! so call sites do not own parallel channel fields by hand (issue #243).

use super::*;
use crate::actions::SystemRunner;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::PathBuf;

/// Owned-fork metadata (gist id → upstream id), or the reason fork detection failed.
pub(super) type ForkMetaResult = Result<std::collections::HashMap<String, Option<String>>, String>;

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
    persist_gist_cache_from_state_fields(
        &state.gists,
        &state.starred_gists,
        &state.starred_gist_ids,
        &state.current_user_login,
        &state.gist_comment_counts,
        &state.gist_fork_counts,
        &state.gist_star_counts,
    );
}

pub(super) fn persist_gist_cache_from_state_fields(
    owned: &[GistFile],
    starred: &[GistFile],
    starred_ids: &std::collections::HashSet<String>,
    user_login: &Option<String>,
    comment_counts: &std::collections::HashMap<String, u32>,
    fork_counts: &std::collections::HashMap<String, u32>,
    star_counts: &std::collections::HashMap<String, u32>,
) {
    if let Ok(path) = crate::cache::cache_path() {
        let cache = crate::cache::GistListCache {
            owned: owned.to_vec(),
            starred: starred.to_vec(),
            starred_ids: starred_ids.iter().cloned().collect(),
            user_login: user_login.clone(),
            comment_counts: comment_counts.clone(),
            fork_counts: fork_counts.clone(),
            star_counts: star_counts.clone(),
        };
        crate::cache::save_gist_cache(&path, &cache);
    }
}

/// Fetches the gist list on a background thread so startup does not block on `gh`.
/// Fork counts are fetched separately so the UI can render lists without waiting.
pub(super) type GistFetchResult = (
    Vec<GistFile>,
    Vec<GistFile>,
    std::collections::HashSet<String>,
    Option<String>,
    std::collections::HashMap<String, u32>,
    Option<String>,
    Option<String>,
);

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

pub(super) fn spawn_gist_fetch() -> std::sync::mpsc::Receiver<GistFetchResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = if crate::gh::check_gh_ready(&SystemRunner).is_ok() {
            // Owned list, starred list, and current-user login are independent network
            // legs — run them concurrently so large accounts don't pay three sequential
            // round-trips on cold start (issue #223). Soft-fail each leg independently
            // (`.ok()`), matching the previous sequential behaviour.
            let (owned, starred_raw, user_login) = std::thread::scope(|s| {
                let owned_h = s.spawn(|| crate::gh::fetch_gist_list_json(&SystemRunner).ok());
                let starred_h =
                    s.spawn(|| crate::gh::fetch_gist_starred_list_json(&SystemRunner).ok());
                let user_h = s.spawn(|| crate::gh::fetch_current_user_login(&SystemRunner).ok());
                (
                    owned_h.join().unwrap_or(None),
                    starred_h.join().unwrap_or(None),
                    user_h.join().unwrap_or(None),
                )
            });
            let (files, mut comment_counts) = owned
                .as_ref()
                .map(|raw| {
                    (
                        crate::gh::parse_gist_list_json(raw).unwrap_or_default(),
                        crate::gh::parse_gist_comment_counts(raw).unwrap_or_default(),
                    )
                })
                .unwrap_or_default();
            if let Some(raw) = starred_raw.as_ref() {
                if let Ok(starred_comments) = crate::gh::parse_gist_comment_counts(raw) {
                    comment_counts.extend(starred_comments);
                }
            }
            let starred = starred_raw
                .as_ref()
                .map(|raw| crate::gh::parse_gist_list_json(raw).unwrap_or_default())
                .unwrap_or_default();
            let starred_ids = starred_raw
                .as_ref()
                .and_then(|raw| crate::gh::parse_starred_gist_ids(raw).ok())
                .unwrap_or_default();
            (
                files,
                starred,
                starred_ids,
                user_login,
                comment_counts,
                owned,
                starred_raw,
            )
        } else {
            Default::default()
        };
        let _ = tx.send(result);
    });
    rx
}

pub(super) fn spawn_fork_count_fetch(
    owned_raw: Option<String>,
    starred_raw: Option<String>,
    gist_ids: std::collections::HashSet<String>,
) -> std::sync::mpsc::Receiver<std::collections::HashMap<String, u32>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let counts = crate::gh::collect_gist_fork_counts(
            &SystemRunner,
            owned_raw.as_deref(),
            starred_raw.as_deref(),
            gist_ids,
        );
        let _ = tx.send(counts);
    });
    rx
}

pub(super) fn spawn_star_count_fetch(
    node_ids: std::collections::HashMap<String, String>,
) -> std::sync::mpsc::Receiver<std::collections::HashMap<String, u32>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let counts = crate::gh::collect_gist_star_counts(&SystemRunner, node_ids);
        let _ = tx.send(counts);
    });
    rx
}

pub(super) fn spawn_fork_metadata_fetch(
    owned_ids: std::collections::HashSet<String>,
) -> std::sync::mpsc::Receiver<ForkMetaResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let fork_of = crate::gh::collect_owned_fork_of_ids(&SystemRunner, owned_ids);
        let _ = tx.send(fork_of);
    });
    rx
}

/// Background local-scan result stamped with the generation active at spawn time.
type LocalScanRx = Option<std::sync::mpsc::Receiver<(u64, Vec<LocalCandidate>)>>;

fn spawn_local_scan(
    generation: u64,
    cwd: std::path::PathBuf,
    pinned: Vec<crate::domain::PinnedMapping>,
    recursive: bool,
    skip_dirs: Vec<String>,
    max_depth: u32,
) -> std::sync::mpsc::Receiver<(u64, Vec<LocalCandidate>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let candidates = crate::local::discover_local_candidates(
            &cwd, &pinned, recursive, &skip_dirs, max_depth,
        )
        .unwrap_or_default();
        let _ = tx.send((generation, candidates));
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

/// Stage the current Pins payload as `pending_return` so the pin diff/confirm entered once this
/// (async) flow's background fetch lands restores list state, not wherever the user has since
/// navigated to.
pub(super) fn park_pins_on_diff_return(state: &mut AppState) {
    let pins = match &state.screen {
        Screen::Pins(p) => p.as_ref().clone(),
        _ => state.pins().cloned().unwrap_or_default(),
    };
    state.pending_return = Some(Screen::Pins(Box::new(pins)));
}

/// Spawn the push (upload local → gist) flow for a pin: lands in the existing
/// upload `Screen::Confirm` diff.
pub(super) fn spawn_pin_push(
    state: &mut AppState,
    jobs: &mut Jobs,
    m: &crate::domain::PinnedMapping,
) {
    park_pins_on_diff_return(state);
    let local_path = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    // Upload Confirm is opened when UploadPreview completes (staged return is Pins).
    let raw_url = state.gist_file_raw_url(&gist_id, &filename);
    let file = crate::domain::GistFileRef::new(gist_id, filename, raw_url);
    let (local_label, gist_label) =
        diff_labels(Some(&local_path), &state.gist_file_for_diff(&file));
    jobs.spawn_gist_fetch_action(
        state,
        "Loading diff…",
        file,
        move |result, file, state| {
            screens::confirm::on_upload_preview(
                state,
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
) {
    park_pins_on_diff_return(state);
    let target = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    let raw_url = state.gist_file_raw_url(&gist_id, &filename);
    let file = crate::domain::GistFileRef::new(gist_id, filename, raw_url);
    let (local_label, gist_label) = diff_labels(Some(&target), &state.gist_file_for_diff(&file));
    jobs.spawn_gist_fetch_action(state, "Downloading…", file, move |result, file, state| {
        screens::diff::on_download_selected(state, result, target, local_label, gist_label, file)
    });
}

/// Spawn a read-only diff (gist vs local) for a pin, landing on `Screen::Diff`.
pub(super) fn spawn_pin_diff(
    state: &mut AppState,
    jobs: &mut Jobs,
    m: &crate::domain::PinnedMapping,
) {
    let local_abs = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    // Stage pin identity so enter_diff copies it onto DiffState (is_pin_diff_context).
    state.staged_diff_gist = Some((gist_id.clone(), filename.clone()));
    let raw_url = state.gist_file_raw_url(&gist_id, &filename);
    let file = crate::domain::GistFileRef::new(gist_id, filename, raw_url);
    let (local_label, gist_label) = diff_labels(Some(&local_abs), &state.gist_file_for_diff(&file));
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
                result,
                Some(local_abs),
                local_label,
                gist_label,
                target,
                false,
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

    if state.mouse_enabled {
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
    if state.mouse_enabled {
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

    if state.mouse_enabled {
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
    if state.mouse_enabled {
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
            // Diff pairing identity lives on the payload; leaving drops it.
            state.staged_diff_gist = None;
            // Skip past the download overwrite gate's Confirm (if any) and its parked Diff to
            // land on whatever was behind them.
            if state.screen.is_confirm() {
                state.leave();
            }
            if state.screen.is_diff() {
                state.leave();
            }
            refresh_locals(state, LocalScanMode::Active, Some(&target));
        }
        Err(error) => {
            state.set_status(format!("download failed: {error}"));
            state.cancel_confirm_to_diff();
        }
    }
}

pub(super) enum LocalScanMode {
    Flat,
    Active,
}

/// Quick re-scan used after a download/upload to make the target visible immediately.
pub(super) fn refresh_locals(
    state: &mut AppState,
    scan_mode: LocalScanMode,
    selection_target: Option<&std::path::Path>,
) {
    let selected = selection_target
        .map(std::path::Path::to_path_buf)
        .or_else(|| state.selected_local().map(|c| c.path.clone()));
    if let Ok(locals) = crate::local::discover_local_candidates(
        &state.cwd,
        &state.pinned,
        matches!(scan_mode, LocalScanMode::Active) && state.local_recursive,
        &state.skip_dirs,
        state.scan_depth,
    ) {
        state.locals = locals;
        state.local_index = selected
            .and_then(|path| state.locals.iter().position(|c| c.path == path))
            .unwrap_or(0)
            .min(state.locals.len().saturating_sub(1));
        if state.gist_index >= state.ranked_gists().len() {
            state.gist_index = 0;
        }
    }
}

/// Persist the diff-context toggle (`diff_show_full`) to the config file, leaving the
/// configured `diff_context` radius untouched. IO boundary, called from `run_loop`.
pub(super) fn persist_theme(state: &mut AppState) {
    let result = crate::config::config_path().and_then(|path| {
        let mut config = crate::config::load_config(&path)?;
        config.theme = state.theme_choice;
        crate::config::save_config(&path, &config)?;
        Ok(())
    });
    let name = match state.theme_choice {
        crate::config::ThemeChoice::Dark => "dark",
        crate::config::ThemeChoice::Light => "light",
    };
    match result {
        Ok(()) => state.set_status(format!("Theme: {name}")),
        Err(error) => state.set_status(format!("save config failed: {error}")),
    }
}

pub(super) fn persist_diff_context(state: &mut AppState) {
    let result = crate::config::config_path().and_then(|path| {
        let mut config = crate::config::load_config(&path)?;
        config.diff_show_full = state.diff_show_full;
        crate::config::save_config(&path, &config)?;
        Ok(())
    });
    match result {
        Ok(()) if state.diff_show_full => state.set_status("Diff context: full file"),
        Ok(()) => state.set_status(format!("Diff context: {} lines", state.diff_context)),
        Err(error) => state.set_status(format!("save config failed: {error}")),
    }
}

/// Persist Settings-screen fields after a user change (issue #227). Creates config.toml
/// only when a value actually changed (opening Config never calls this).
pub(super) fn persist_settings(state: &mut AppState) {
    let result = crate::config::config_path().and_then(|path| {
        let mut config = crate::config::load_config(&path)?;
        config.theme = state.theme_choice;
        config.mouse = state.config_mouse;
        config.check_updates = state.config_check_updates;
        config.ignore_trailing_newline = state.ignore_trailing_newline;
        config.scan_depth = state.scan_depth;
        config.diff_context = state.diff_context;
        crate::config::save_config(&path, &config)?;
        Ok(())
    });
    match result {
        Ok(()) => {
            let field = ConfigField::ALL
                .get(state.config().map(|c| c.index).unwrap_or(0))
                .copied()
                .unwrap_or(ConfigField::Theme);
            state.set_status(format!(
                "{}: {}",
                field.label(),
                state.config_field_value(field)
            ));
        }
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
            state.scan_depth = config.scan_depth;
            state.mark_pin_sync_cache_dirty();
            state.set_status(format!("Pinned {} <-> {}", local_path.display(), filename));
        }
        Err(error) => state.set_status(format!("pin failed: {error}")),
    }
}

pub(super) fn unpin_path(state: &mut AppState, local_path: &std::path::Path) {
    let result = crate::config::config_path().and_then(|path| {
        let config = crate::config::load_config(&path)?;
        crate::actions::unpin_mapping(&path, config, local_path)
    });
    match result {
        Ok(config) => {
            state.pinned = config.pinned;
            state.skip_dirs = config.skip_dirs;
            state.scan_depth = config.scan_depth;
            state.mark_pin_sync_cache_dirty();
            state.set_status(format!(
                "Unpinned {}",
                crate::config::display_path(local_path)
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
        crate::actions::unpin_mapping_exact(&path, config, &mapping.local_path, &mapping.gist_id)
    });
    match result {
        Ok(config) => {
            state.pinned = config.pinned;
            state.skip_dirs = config.skip_dirs;
            state.scan_depth = config.scan_depth;
            state.mark_pin_sync_cache_dirty();
            let len = state.visible_pin_indices().len();
            if let Some(pins) = state.pins_mut() {
                pins.cursor.clamp_len(len);
            }
            refresh_locals(state, LocalScanMode::Flat, None);
            state.set_status(format!("Unpinned {label}"));
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
/// - **Local scans** ([`Jobs::request_local_scan`]): stamp `AppState::local_scan_generation`.
/// - **One-shot slots** (gist list, fork/star counts, update check, fork meta): a new
///   spawn replaces the receiver; there is no multi-generation queue for these.
pub(super) struct Jobs {
    update: Option<std::sync::mpsc::Receiver<crate::update_check::UpdateCheckOutcome>>,
    gist: Option<std::sync::mpsc::Receiver<GistFetchResult>>,
    fork: Option<std::sync::mpsc::Receiver<std::collections::HashMap<String, u32>>>,
    star: Option<std::sync::mpsc::Receiver<std::collections::HashMap<String, u32>>>,
    fork_meta: Option<std::sync::mpsc::Receiver<ForkMetaResult>>,
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
    ) -> Self {
        Self {
            update,
            gist: fetch_gists.then(spawn_gist_fetch),
            fork: None,
            star: None,
            fork_meta: None,
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
        let generation = state.begin_local_scan();
        state.set_status("Scanning files…");
        state.local_scanning = true;
        self.local = Some(spawn_local_scan(
            generation,
            state.cwd.clone(),
            state.pinned.clone(),
            state.local_recursive,
            state.skip_dirs.clone(),
            state.scan_depth,
        ));
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

    /// Poll each channel in turn and apply ready results to `state`.
    ///
    /// **Order matters**: the gist-list section spawns follow-up jobs (fork counts, fork
    /// metadata, star counts) on success, and those sections poll the very channels it just
    /// spawned in this same call — so `gist` must run before `fork`/`star`/`fork_meta`.
    fn absorb_inner(
        &mut self,
        state: &mut AppState,
        update_check_path: &Option<std::path::PathBuf>,
    ) -> Result<LoopFlow> {
        self.on_gist_list_ready(state);
        self.on_fork_counts_ready(state);
        self.on_star_counts_ready(state);
        self.on_fork_meta_ready(state);
        self.on_local_scan_ready(state);
        self.on_update_check_ready(state, update_check_path);
        self.on_upload_watch_events(state);
        let flow = self.on_action_outcome(state);
        if std::mem::take(&mut state.gist_list_stale) {
            state.loading = true;
            self.gist = Some(spawn_gist_fetch());
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
/// Only covers channels that receive a single unstamped result (`gist`, `fork`, `star`,
/// `fork_meta`, `update`). The generation-stamped channels (`local`, `action`) and the
/// multi-message `upload_edit_watch` drain have different shapes and stay hand-rolled.
fn poll_channel<T>(slot: &mut Option<std::sync::mpsc::Receiver<T>>) -> Option<T> {
    let value = slot.as_ref()?.try_recv().ok()?;
    *slot = None;
    Some(value)
}

impl Jobs {
    /// Absorb the background gist list once it arrives, and spawn the follow-up
    /// fork-count/fork-meta/star-count jobs the freshly-loaded gist list needs.
    fn on_gist_list_ready(&mut self, state: &mut AppState) {
        if state.loading {
            if let Some((
                gists,
                starred,
                starred_ids,
                user_login,
                comment_counts,
                owned_raw,
                starred_raw,
            )) = poll_channel(&mut self.gist)
            {
                persist_gist_cache_from_state_fields(
                    &gists,
                    &starred,
                    &starred_ids,
                    &user_login,
                    &comment_counts,
                    &state.gist_fork_counts,
                    &state.gist_star_counts,
                );
                state.gists = gists;
                state.starred_gists = starred;
                state.starred_gist_ids = starred_ids;
                state.current_user_login = user_login;
                state.gist_comment_counts = comment_counts;
                state.loading = false;
                if state.gist_index >= state.ranked_gists().len() {
                    state.gist_index = 0;
                }
                let count = state.visible_gist_groups().len();
                if let Some(gm) = state.gist_manager_mut() {
                    gm.cursor.clamp_len(count);
                }
                let gist_ids: std::collections::HashSet<String> = state
                    .gists
                    .iter()
                    .chain(state.starred_gists.iter())
                    .map(|g| g.gist_id.clone())
                    .collect();
                self.fork = Some(spawn_fork_count_fetch(
                    owned_raw,
                    starred_raw,
                    gist_ids.clone(),
                ));
                self.fork_meta = Some(spawn_fork_metadata_fetch(
                    state.gists.iter().map(|g| g.gist_id.clone()).collect(),
                ));
                let node_ids =
                    crate::gh::merge_gist_node_id_maps(&state.gists, &state.starred_gists);
                self.star = Some(spawn_star_count_fetch(node_ids));
            }
        }
    }

    /// Absorb background fork-count results.
    fn on_fork_counts_ready(&mut self, state: &mut AppState) {
        if let Some(fork_counts) = poll_channel(&mut self.fork) {
            state.gist_fork_counts = fork_counts;
            persist_gist_cache_from_state(state);
        }
    }

    /// Absorb background star-count results.
    fn on_star_counts_ready(&mut self, state: &mut AppState) {
        if let Some(star_counts) = poll_channel(&mut self.star) {
            state.gist_star_counts = star_counts;
            persist_gist_cache_from_state(state);
        }
    }

    /// Absorb background fork-metadata (owned-fork upstream ids) results.
    fn on_fork_meta_ready(&mut self, state: &mut AppState) {
        if let Some(result) = poll_channel(&mut self.fork_meta) {
            match result {
                Ok(fork_of) => {
                    crate::gh::apply_fork_of_ids(&mut state.gists, &fork_of);
                    persist_gist_cache_from_state(state);
                }
                Err(error) => state.set_status(format!("fork detection unavailable: {error}")),
            }
        }
    }

    /// Absorb a completed background local scan (ignore stale generations — issue #221).
    fn on_local_scan_ready(&mut self, state: &mut AppState) {
        if state.local_scanning {
            if let Some(ref rx) = self.local {
                if let Ok((generation, locals)) = rx.try_recv() {
                    self.local = None;
                    if state.apply_local_scan_if_current(generation, locals) {
                        state.status = None;
                    }
                    // Stale: a newer scan is (or was) in flight; leave spinner/list alone.
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
            gist: None,
            fork: None,
            star: None,
            fork_meta: None,
            local: None,
            upload_edit_watch: None,
            action: None,
        }
    }

    // ---- on_gist_list_ready ---------------------------------------------

    #[test]
    fn on_gist_list_ready_noop_when_not_loading() {
        let mut state = initial_state();
        state.loading = false;
        let (tx, rx) = mpsc::channel::<GistFetchResult>();
        tx.send(Default::default()).unwrap();
        let mut jobs = empty_jobs();
        jobs.gist = Some(rx);

        jobs.on_gist_list_ready(&mut state);

        // Guarded entirely by `state.loading`: the channel is never polled.
        assert!(jobs.gist.is_some());
        assert!(state.gists.is_empty());
    }

    #[test]
    fn on_gist_list_ready_noop_when_channel_empty() {
        let mut state = initial_state();
        state.loading = true;
        let (_tx, rx) = mpsc::channel::<GistFetchResult>();
        let mut jobs = empty_jobs();
        jobs.gist = Some(rx);

        jobs.on_gist_list_ready(&mut state);

        assert!(jobs.gist.is_some());
        assert!(state.loading);
        assert!(state.gists.is_empty());
    }

    // ---- on_fork_counts_ready / on_star_counts_ready ---------------------
    //
    // The "value received" branch of both also persists the gist cache to the real
    // per-OS cache directory (`persist_gist_cache_from_state` — see `bg.rs`), which has
    // no test seam (unlike `update_check_path`, it isn't threaded through as a
    // parameter). Exercising that branch here would write to the developer's real
    // `~/.cache/gistui/gists.json` (or platform equivalent) as a side effect of running
    // `cargo test`, which the project's "no live gh/network in tests" convention rules
    // out in spirit. Only the empty-channel guard is covered directly.

    #[test]
    fn on_fork_counts_ready_noop_when_channel_empty() {
        let mut state = initial_state();
        let (_tx, rx) = mpsc::channel();
        let mut jobs = empty_jobs();
        jobs.fork = Some(rx);

        jobs.on_fork_counts_ready(&mut state);

        assert!(jobs.fork.is_some());
        assert!(state.gist_fork_counts.is_empty());
    }

    #[test]
    fn on_star_counts_ready_noop_when_channel_empty() {
        let mut state = initial_state();
        let (_tx, rx) = mpsc::channel();
        let mut jobs = empty_jobs();
        jobs.star = Some(rx);

        jobs.on_star_counts_ready(&mut state);

        assert!(jobs.star.is_some());
        assert!(state.gist_star_counts.is_empty());
    }

    // ---- on_fork_meta_ready -----------------------------------------------

    #[test]
    fn on_fork_meta_ready_noop_when_channel_empty() {
        let mut state = initial_state();
        let (_tx, rx) = mpsc::channel();
        let mut jobs = empty_jobs();
        jobs.fork_meta = Some(rx);

        jobs.on_fork_meta_ready(&mut state);

        assert!(jobs.fork_meta.is_some());
    }

    #[test]
    fn on_fork_meta_ready_sets_status_on_error() {
        let mut state = initial_state();
        let (tx, rx) = mpsc::channel();
        tx.send(Err("boom".to_string())).unwrap();
        let mut jobs = empty_jobs();
        jobs.fork_meta = Some(rx);

        jobs.on_fork_meta_ready(&mut state);

        assert!(jobs.fork_meta.is_none());
        assert_eq!(
            state.status.as_deref(),
            Some("fork detection unavailable: boom")
        );
    }

    // ---- on_local_scan_ready ----------------------------------------------

    #[test]
    fn on_local_scan_ready_noop_when_not_scanning() {
        let mut state = initial_state();
        state.local_scanning = false;
        let (tx, rx) = mpsc::channel();
        tx.send((1, Vec::<LocalCandidate>::new())).unwrap();
        let mut jobs = empty_jobs();
        jobs.local = Some(rx);

        jobs.on_local_scan_ready(&mut state);

        // Guarded by `state.local_scanning`: the channel is never touched.
        assert!(jobs.local.is_some());
    }

    #[test]
    fn on_local_scan_ready_applies_current_generation() {
        let mut state = initial_state();
        let generation = state.begin_local_scan();
        state.local_scanning = true;
        state.status = Some("Scanning files…".into());
        let candidate = LocalCandidate {
            path: PathBuf::from("a.txt"),
            pinned: false,
            modified: None,
        };
        let (tx, rx) = mpsc::channel();
        tx.send((generation, vec![candidate.clone()])).unwrap();
        let mut jobs = empty_jobs();
        jobs.local = Some(rx);

        jobs.on_local_scan_ready(&mut state);

        assert!(jobs.local.is_none());
        assert!(!state.local_scanning);
        assert_eq!(state.locals, vec![candidate]);
        assert!(state.status.is_none());
    }

    #[test]
    fn on_local_scan_ready_ignores_stale_generation() {
        let mut state = initial_state();
        let stale = state.begin_local_scan();
        let _current = state.begin_local_scan();
        state.local_scanning = true;
        let (tx, rx) = mpsc::channel();
        tx.send((
            stale,
            vec![LocalCandidate {
                path: PathBuf::from("stale.txt"),
                pinned: false,
                modified: None,
            }],
        ))
        .unwrap();
        let mut jobs = empty_jobs();
        jobs.local = Some(rx);

        jobs.on_local_scan_ready(&mut state);

        // The stale result is drained off the channel but not applied — spinner/list
        // stay as they were (a newer scan is still expected).
        assert!(jobs.local.is_none());
        assert!(state.local_scanning);
        assert!(state.locals.is_empty());
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
        assert!(jobs.gist.is_none());
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
            pinned: false,
            modified: None,
        }];

        refresh_locals(&mut state, LocalScanMode::Active, Some(&target));

        assert_eq!(state.selected_local().map(|file| file.path), Some(target));
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
