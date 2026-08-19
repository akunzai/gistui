//! Background workers and the **job registry** (`Jobs`) for the TUI event loop.
//! Extracted from `run_loop` (issue #225); deepened into a single spawn/absorb API
//! so call sites do not own parallel channel fields by hand (issue #243).

use super::*;
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

pub(super) enum BgTaskOutcome {
    PreviewDiff {
        result: std::result::Result<String, String>,
        local_path: Option<PathBuf>,
        local_label: String,
        gist_label: String,
        target: PathBuf,
        // True when the local pane was focused at trigger time: frame the preview as an
        // upload (old = gist, new = local) instead of a download.
        upload_orientation: bool,
    },
    DownloadSelected {
        result: std::result::Result<String, String>,
        target: PathBuf,
        local_label: String,
        gist_label: String,
        file: crate::domain::GistFileRef,
    },
    UploadPreview {
        result: std::result::Result<String, String>,
        file: crate::domain::GistFileRef,
        local_path: PathBuf,
        local_label: String,
        gist_label: String,
    },
    UploadReplace {
        result: std::result::Result<(), String>,
        file: crate::domain::GistFileRef,
    },
    CreateGist {
        result: std::result::Result<(), String>,
        local_path: PathBuf,
        public: bool,
    },
    PreviewContent {
        result: std::result::Result<String, String>,
        file: crate::domain::GistFileRef,
        preview_title: String,
    },
    DeleteGist {
        result: std::result::Result<(), String>,
        gist_id: String,
    },
    RemoveFile {
        result: std::result::Result<(), String>,
        gist_id: String,
        filename: String,
    },
    ApplyDescription {
        result: std::result::Result<(), String>,
        gist_id: String,
    },
    CompactAnalyze {
        result: std::result::Result<usize, String>,
        gist_id: String,
        label: String,
    },
    CompactGist {
        result: std::result::Result<(), String>,
        label: String,
        count: usize,
    },
    CommentsInitialLoaded {
        gist_id: String,
        result: Result<crate::tui::InitialComments, String>,
    },
    CommentsOlderLoaded {
        gist_id: String,
        result: Result<Vec<GistComment>, String>,
    },
    RevisionsFetched {
        gist_id: String,
        result: std::result::Result<Vec<crate::domain::GistRevision>, String>,
    },
    RevisionDiff {
        result: std::result::Result<(String, String), String>,
        old_label: String,
        new_label: String,
    },
    RestoreRevisionReady {
        result: std::result::Result<(String, String), String>,
        gist_id: String,
        filename: String,
        version: String,
        version_label: String,
    },
    RestoreRevisionDone {
        result: std::result::Result<(), String>,
        gist_id: String,
        filename: String,
    },
    GistStarToggle {
        result: std::result::Result<(), String>,
        gist_id: String,
        starred: bool,
    },
    ForkGist {
        result: std::result::Result<(), String>,
        gist_id: String,
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
        crate::gh::fetch_revision_file_text_optional(gist_id, child_version, filename, owner_login)
            .map_err(|e| e.to_string())?,
    )?;
    let old_content = match parent_version {
        Some(parent) => ensure_fetched_text(
            crate::gh::fetch_revision_file_text_optional(gist_id, parent, filename, owner_login)
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
        crate::gh::fetch_revision_file_text(gist_id, version, filename, owner_login)
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
    let content = crate::gh::fetch_gist_file_content(gist_id, filename, raw_url)
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
        let result = if crate::gh::check_gh_ready().is_ok() {
            // Owned list, starred list, and current-user login are independent network
            // legs — run them concurrently so large accounts don't pay three sequential
            // round-trips on cold start (issue #223). Soft-fail each leg independently
            // (`.ok()`), matching the previous sequential behaviour.
            let (owned, starred_raw, user_login) = std::thread::scope(|s| {
                let owned_h = s.spawn(|| crate::gh::fetch_gist_list_json().ok());
                let starred_h = s.spawn(|| crate::gh::fetch_gist_starred_list_json().ok());
                let user_h = s.spawn(|| crate::gh::fetch_current_user_login().ok());
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
        let counts = crate::gh::collect_gist_star_counts(node_ids);
        let _ = tx.send(counts);
    });
    rx
}

pub(super) fn spawn_fork_metadata_fetch(
    owned_ids: std::collections::HashSet<String>,
) -> std::sync::mpsc::Receiver<ForkMetaResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let fork_of = crate::gh::collect_owned_fork_of_ids(owned_ids);
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

/// Background per-action outcome stamped with the generation active at spawn time.
type ActionRx = Option<std::sync::mpsc::Receiver<(u64, BgTaskOutcome)>>;

/// Initial newest-first comment load: probe the total, then fetch the newest page.
/// Thin IO boundary (network) — not unit-tested.
pub(super) fn load_initial_comments(gist_id: &str) -> Result<crate::tui::InitialComments, String> {
    let probe = crate::gh::fetch_gist_comments_probe(gist_id).map_err(|e| e.to_string())?;
    let total = crate::gh::comments_total_from_probe(&probe);
    if total == 0 {
        return Ok(crate::tui::InitialComments {
            comments: Vec::new(),
            total: 0,
            oldest_page: 1,
        });
    }
    let oldest_page = crate::gh::last_page(total, crate::gh::COMMENTS_PAGE_SIZE);
    let raw =
        crate::gh::fetch_gist_comments_page(gist_id, oldest_page, crate::gh::COMMENTS_PAGE_SIZE)
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
    jobs.spawn_gist_fetch_action(state, "Loading diff…", file, move |result, file| {
        BgTaskOutcome::UploadPreview {
            result,
            file,
            local_path,
            local_label,
            gist_label,
        }
    });
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
    jobs.spawn_gist_fetch_action(state, "Downloading…", file, move |result, file| {
        BgTaskOutcome::DownloadSelected {
            result,
            target,
            local_label,
            gist_label,
            file,
        }
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
    jobs.spawn_gist_fetch_action(state, "Loading diff…", file, move |result, _file| {
        BgTaskOutcome::PreviewDiff {
            result,
            local_path: Some(local_abs),
            local_label,
            gist_label,
            target,
            // Pin diffs originate from the Pins screen (no focused pane); keep the
            // historical download orientation (old = local, new = gist).
            upload_orientation: false,
        }
    });
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
    let Some(text) = state.preview().map(|p| p.text.clone()) else {
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

enum LocalScanMode {
    Flat,
    Active,
}

/// Quick re-scan used after a download/upload to make the target visible immediately.
fn refresh_locals(
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

/// Background job registry (issue #243).
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

    /// Run `work` on a background thread. Sets `bg_task_msg` and stamps the result with
    /// the current action-job generation (issue #221).
    pub(super) fn spawn_action<F>(&mut self, state: &mut AppState, msg: impl Into<String>, work: F)
    where
        F: FnOnce() -> BgTaskOutcome + Send + 'static,
    {
        let generation = state.begin_bg_task();
        state.bg_task_msg = Some(msg.into());
        let (tx, rx) = std::sync::mpsc::channel();
        self.action = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send((generation, work()));
        });
    }

    /// Spawn a background job that fetches a gist file's content, then hands the result
    /// (and the file identity back) to `wrap` to build the `BgTaskOutcome`. Collapses the
    /// `fetch_gist_content` template shared by `PreviewDiff`/`DownloadSelected`/
    /// `UploadPreview`/`PreviewContent` across `dispatch.rs` and the pin-spawn helpers
    /// below (issue #299). `wrap` gets `file` back so variants that store it (all but
    /// `PreviewDiff`) don't need a second clone.
    pub(super) fn spawn_gist_fetch_action(
        &mut self,
        state: &mut AppState,
        msg: impl Into<String>,
        file: crate::domain::GistFileRef,
        wrap: impl FnOnce(std::result::Result<String, String>, crate::domain::GistFileRef) -> BgTaskOutcome
            + Send
            + 'static,
    ) {
        self.spawn_action(state, msg, move || {
            let result = fetch_gist_content(&file.gist_id, &file.filename, file.raw_url.as_deref());
            wrap(result, file)
        });
    }

    /// Esc cancel: drop the action receiver and invalidate generation so a late completion
    /// cannot mutate state.
    pub(super) fn cancel_action(&mut self, state: &mut AppState) {
        state.invalidate_bg_task();
        self.action = None;
        state.set_status("Cancelled");
    }

    /// Replace any in-flight gist list fetch with a new one (also sets `state.loading`).
    pub(super) fn request_gist_fetch(&mut self, state: &mut AppState) {
        state.loading = true;
        self.gist = Some(spawn_gist_fetch());
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
        Ok(self.on_action_outcome(state))
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
    /// A router shell: this owns the generation guard and dispatches each variant to its own
    /// `on_<variant_snake_case>` method (issue #298) instead of inlining ~480 lines of match
    /// arms here.
    fn on_action_outcome(&mut self, state: &mut AppState) -> LoopFlow {
        if let Some(ref rx) = self.action {
            if let Ok((generation, outcome)) = rx.try_recv() {
                self.action = None;
                if state.is_current_bg_generation(generation) {
                    state.bg_task_msg = None;
                    match outcome {
                        BgTaskOutcome::PreviewDiff {
                            result,
                            local_path,
                            local_label,
                            gist_label,
                            target,
                            upload_orientation,
                        } => self.on_preview_diff(
                            state,
                            result,
                            local_path,
                            local_label,
                            gist_label,
                            target,
                            upload_orientation,
                        ),
                        BgTaskOutcome::DownloadSelected {
                            result,
                            target,
                            local_label,
                            gist_label,
                            file,
                        } => self.on_download_selected(
                            state,
                            result,
                            target,
                            local_label,
                            gist_label,
                            file,
                        ),
                        BgTaskOutcome::UploadPreview {
                            result,
                            file,
                            local_path,
                            local_label,
                            gist_label,
                        } => self.on_upload_preview(
                            state,
                            result,
                            file,
                            local_path,
                            local_label,
                            gist_label,
                        ),
                        BgTaskOutcome::UploadReplace { result, file } => {
                            self.on_upload_replace(state, result, file)
                        }
                        BgTaskOutcome::CreateGist {
                            result,
                            local_path,
                            public,
                        } => self.on_create_gist(state, result, local_path, public),
                        BgTaskOutcome::PreviewContent {
                            result,
                            file,
                            preview_title,
                        } => self.on_preview_content(state, result, file, preview_title),
                        BgTaskOutcome::DeleteGist { result, gist_id } => {
                            self.on_delete_gist(state, result, gist_id)
                        }
                        BgTaskOutcome::RemoveFile {
                            result,
                            gist_id,
                            filename,
                        } => self.on_remove_file(state, result, gist_id, filename),
                        BgTaskOutcome::ApplyDescription { result, gist_id } => {
                            self.on_apply_description(state, result, gist_id)
                        }
                        BgTaskOutcome::CompactAnalyze {
                            result,
                            gist_id,
                            label,
                        } => self.on_compact_analyze(state, result, gist_id, label),
                        BgTaskOutcome::CompactGist {
                            result,
                            label,
                            count,
                        } => self.on_compact_gist(state, result, label, count),
                        BgTaskOutcome::CommentsInitialLoaded { gist_id, result } => {
                            self.on_comments_initial_loaded(state, gist_id, result)
                        }
                        BgTaskOutcome::CommentsOlderLoaded { gist_id, result } => {
                            self.on_comments_older_loaded(state, gist_id, result)
                        }
                        BgTaskOutcome::RevisionsFetched { gist_id, result } => {
                            if let LoopFlow::SkipIteration =
                                self.on_revisions_fetched(state, gist_id, result)
                            {
                                return LoopFlow::SkipIteration;
                            }
                        }
                        BgTaskOutcome::RevisionDiff {
                            result,
                            old_label,
                            new_label,
                        } => self.on_revision_diff(state, result, old_label, new_label),
                        BgTaskOutcome::RestoreRevisionReady {
                            result,
                            gist_id,
                            filename,
                            version,
                            version_label,
                        } => {
                            if let LoopFlow::SkipIteration = self.on_restore_revision_ready(
                                state,
                                result,
                                gist_id,
                                filename,
                                version,
                                version_label,
                            ) {
                                return LoopFlow::SkipIteration;
                            }
                        }
                        BgTaskOutcome::GistStarToggle {
                            result,
                            gist_id,
                            starred,
                        } => self.on_gist_star_toggle(state, result, gist_id, starred),
                        BgTaskOutcome::ForkGist { result, gist_id } => {
                            self.on_fork_gist(state, result, gist_id)
                        }
                        BgTaskOutcome::RestoreRevisionDone {
                            result,
                            gist_id,
                            filename,
                        } => self.on_restore_revision_done(state, result, gist_id, filename),
                    }
                } // is_current_bg_generation — stale outcomes are dropped without applying
            }
        }
        LoopFlow::Proceed
    }

    /// `PreviewDiff` outcome: build the local-vs-gist preview diff and, if the two sides are
    /// byte-identical, opportunistically refresh the pin-sync cache with the content already
    /// fetched (see the inline comment below for why).
    #[allow(clippy::too_many_arguments)]
    fn on_preview_diff(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<String, String>,
        local_path: Option<PathBuf>,
        local_label: String,
        gist_label: String,
        target: PathBuf,
        upload_orientation: bool,
    ) {
        match result {
            Ok(remote) => {
                match local_path
                    .as_ref()
                    .map(|p| crate::domain::read_text_file_capped(p))
                    .transpose()
                {
                    Ok(local) => {
                        let local_content = local.unwrap_or_default();
                        let diff = preview_diff_text(
                            upload_orientation,
                            &local_label,
                            &local_content,
                            &gist_label,
                            &remote,
                            state.ignore_trailing_newline,
                        );
                        let identical = crate::diff::content_eq(
                            &local_content,
                            &remote,
                            state.ignore_trailing_newline,
                        );
                        state.enter_diff(diff, remote, local_path.unwrap_or_default(), target);
                        set_diff_identical(state, identical);
                        // A pin diff that turns out identical confirms the cached
                        // last_seen_hash is (still) accurate — refresh it for free
                        // using the content we already fetched, so the Pins list's
                        // content-hash check (AppState::compute_pin_sync_status) stays
                        // correct even if the gist changed elsewhere since the last
                        // real sync. Hash the LOCAL content's raw bytes (not the
                        // trailing-newline-normalized `identical` comparison), so
                        // this matches the raw-byte hashing compute_pin_sync_status does.
                        if identical {
                            let pin = state.diff().and_then(|d| {
                                Some((
                                    d.gist_id.clone()?,
                                    d.gist_filename.clone()?,
                                    d.local_path.clone(),
                                ))
                            });
                            if let Some((gid, fname, local_abs)) = pin {
                                record_pin_sync(
                                    state,
                                    &local_abs,
                                    &gid,
                                    &fname,
                                    &local_content,
                                    None,
                                );
                            }
                        }
                    }
                    Err(error) => state.set_status(format!("read failed: {error}")),
                }
            }
            Err(error) => state.set_status(format!("fetch failed: {error}")),
        }
    }

    /// `DownloadSelected` outcome: diff against an existing local file, or write a new one.
    fn on_download_selected(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<String, String>,
        target: PathBuf,
        local_label: String,
        gist_label: String,
        file: crate::domain::GistFileRef,
    ) {
        match result {
            Ok(remote) => {
                if target.exists() {
                    match crate::domain::read_text_file_capped(&target) {
                        Ok(local_content) => {
                            let diff = crate::diff::unified_diff(
                                &local_label,
                                &local_content,
                                &gist_label,
                                &remote,
                                state.ignore_trailing_newline,
                            );
                            let identical = crate::diff::content_eq(
                                &local_content,
                                &remote,
                                state.ignore_trailing_newline,
                            );
                            state.staged_diff_gist =
                                Some((file.gist_id.clone(), file.filename.clone()));
                            state.enter_diff(diff, remote, target.clone(), target);
                            set_diff_identical(state, identical);
                        }
                        Err(error) => state.set_status(error),
                    }
                } else {
                    match crate::actions::execute_download(
                        &target,
                        &remote,
                        crate::actions::DownloadMode::CreateNew,
                    ) {
                        Ok(()) => {
                            state.set_status(format!(
                                "Downloaded {}",
                                target
                                    .file_name()
                                    .unwrap_or(target.as_os_str())
                                    .to_string_lossy()
                            ));
                            record_pin_sync(
                                state,
                                &target,
                                &file.gist_id,
                                &file.filename,
                                &remote,
                                Some(crate::domain::SyncDirection::Download),
                            );
                            refresh_locals(state, LocalScanMode::Active, Some(&target));
                        }
                        Err(error) => state.set_status(format!("download failed: {error}")),
                    }
                }
            }
            Err(error) => state.set_status(format!("fetch failed: {error}")),
        }
    }

    /// `UploadPreview` outcome: stage the pending Upload action and open Confirm with the
    /// local-vs-gist diff.
    fn on_upload_preview(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<String, String>,
        file: crate::domain::GistFileRef,
        local_path: PathBuf,
        local_label: String,
        gist_label: String,
    ) {
        match result {
            Ok(remote) => {
                // Keep staged pin/list return; enter_confirm consumes it.
                let action = PendingAction::Upload {
                    gist_id: file.gist_id,
                    filename: file.filename,
                    local_path: local_path.clone(),
                };
                match state.init_upload_state(&local_path, Some(remote), local_label, gist_label) {
                    Ok(()) => {
                        // init_upload_state writes via update_upload_diff only when
                        // Confirm is already open; open Confirm first with empty
                        // body, then rebuild the upload diff into the payload.
                        state.enter_confirm(action, String::new());
                        state.update_upload_diff();
                    }
                    Err(error) => {
                        state.set_status(format!(
                            "cannot read {}: {error}",
                            crate::config::display_path(&local_path)
                        ));
                    }
                }
            }
            Err(error) => state.set_status(format!("fetch failed: {error}")),
        }
    }

    /// `UploadReplace` outcome: commit the pin-sync record and return to wherever the upload
    /// was initiated from, then re-fetch the gist list.
    fn on_upload_replace(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<(), String>,
        file: crate::domain::GistFileRef,
    ) {
        match result {
            Ok(_) => {
                state.gist_content_cache.remove(&file.cache_key());
                state.set_status(format!(
                    "Uploaded {} to gist {}",
                    file.filename, file.gist_id
                ));
                if let Some(local_path) = state.upload_local_path() {
                    let content = state.content_to_upload();
                    record_pin_sync(
                        state,
                        &local_path,
                        &file.gist_id,
                        &file.filename,
                        &content,
                        Some(crate::domain::SyncDirection::Upload),
                    );
                }
                // Return to wherever this upload was initiated from (List, or Pins
                // for a pin push) instead of always snapping to List.
                state.staged_diff_gist = None;
                state.leave();
                self.request_gist_fetch(state);
            }
            Err(error) => {
                state.set_status(format!("upload failed: {error}"));
                // Stay on Confirm payload if still open.
            }
        }
    }

    /// `CreateGist` outcome.
    fn on_create_gist(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<(), String>,
        local_path: PathBuf,
        public: bool,
    ) {
        match result {
            Ok(_) => {
                let visibility = if public { "public" } else { "secret" };
                state.set_status(format!(
                    "Created {} gist from {}",
                    visibility,
                    crate::config::display_path(&local_path)
                ));
                state.description_input.clear();
                state.back_to_list();
                self.request_gist_fetch(state);
            }
            Err(error) => {
                state.set_status(format!("create failed: {error}"));
                state.screen = Screen::List;
                state.description_input.clear();
            }
        }
    }

    /// `PreviewContent` outcome: cache the fetched content and open the read-only preview.
    fn on_preview_content(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<String, String>,
        file: crate::domain::GistFileRef,
        preview_title: String,
    ) {
        match result {
            Ok(content) => {
                let key = file.cache_key();
                state
                    .gist_content_cache
                    .insert(key.clone(), content.clone());
                state.enter_preview(preview_title, content, Some(key));
            }
            Err(error) => state.set_status(format!("fetch failed: {error}")),
        }
    }

    /// `DeleteGist` outcome.
    fn on_delete_gist(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<(), String>,
        gist_id: String,
    ) {
        match result {
            Ok(_) => {
                state.set_status(format!("Deleted gist {gist_id}"));
                self.request_gist_fetch(state);
            }
            Err(error) => state.set_status(format!("delete failed: {error}")),
        }
    }

    /// `RemoveFile` outcome.
    fn on_remove_file(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<(), String>,
        gist_id: String,
        filename: String,
    ) {
        match result {
            Ok(_) => {
                state
                    .gist_content_cache
                    .remove(&(gist_id.clone(), filename.clone()));
                state.set_status(format!("Removed {filename} from gist {gist_id}"));
                self.request_gist_fetch(state);
            }
            Err(error) => state.set_status(format!("remove failed: {error}")),
        }
    }

    /// `ApplyDescription` outcome.
    fn on_apply_description(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<(), String>,
        gist_id: String,
    ) {
        match result {
            Ok(_) => {
                state.set_status(format!("Updated description for gist {gist_id}"));
                self.request_gist_fetch(state);
            }
            Err(error) => state.set_status(format!("description update failed: {error}")),
        }
    }

    /// `CompactAnalyze` outcome: a single-revision gist has nothing to compact; otherwise open
    /// the Confirm warning before compacting.
    fn on_compact_analyze(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<usize, String>,
        gist_id: String,
        label: String,
    ) {
        match result {
            Ok(count) if count <= 1 => state.set_status(format!(
                "\"{label}\" already has a single revision — nothing to compact"
            )),
            Ok(count) => {
                // `pending_return` was staged at the 'c' keypress (keys.rs); `enter`
                // (inside `enter_confirm`) consumes it as the Confirm cancel path.
                state.enter_confirm(
                    PendingAction::CompactGist {
                        gist_id: gist_id.clone(),
                        label: label.clone(),
                        count,
                    },
                    format!(
                        "Compact gist {gist_id} (\"{label}\").\n\nIt has {count} revisions. Compacting clones it to a temp dir, squashes the history to a single commit, and force-pushes — the {} older revisions are gone for good.",
                        count - 1
                    ),
                );
            }
            Err(error) => state.set_status(format!("revision check failed: {error}")),
        }
    }

    /// `CompactGist` outcome.
    fn on_compact_gist(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<(), String>,
        label: String,
        count: usize,
    ) {
        match result {
            Ok(_) => {
                state.set_status(format!("Compacted \"{label}\" ({count} → 1 revision)"));
                self.request_gist_fetch(state);
            }
            Err(error) => state.set_status(format!("compact failed: {error}")),
        }
    }

    /// `CommentsInitialLoaded` outcome.
    fn on_comments_initial_loaded(
        &mut self,
        state: &mut AppState,
        gist_id: String,
        result: Result<crate::tui::InitialComments, String>,
    ) {
        state.apply_initial_comments(&gist_id, result);
    }

    /// `CommentsOlderLoaded` outcome.
    fn on_comments_older_loaded(
        &mut self,
        state: &mut AppState,
        gist_id: String,
        result: Result<Vec<GistComment>, String>,
    ) {
        state.apply_older_comments(&gist_id, result);
    }

    /// `RevisionsFetched` outcome. Returns [`LoopFlow::SkipIteration`] if the fetch belongs to
    /// a gist the Revisions screen has since navigated away from.
    fn on_revisions_fetched(
        &mut self,
        state: &mut AppState,
        gist_id: String,
        result: std::result::Result<Vec<crate::domain::GistRevision>, String>,
    ) -> LoopFlow {
        if state.revision().and_then(|r| r.gist_id.as_deref()) != Some(gist_id.as_str()) {
            return LoopFlow::SkipIteration;
        }
        match result {
            Ok(entries) => {
                let short = entries.len() <= 1;
                if let Some(rev) = state.revision_mut() {
                    rev.fetch_error = None;
                    rev.entries = Some(entries);
                }
                if short {
                    state.set_status("only one revision — nothing to restore");
                }
            }
            Err(error) => {
                if let Some(rev) = state.revision_mut() {
                    rev.entries = Some(Vec::new());
                    rev.fetch_error = Some(error);
                }
            }
        }
        LoopFlow::Proceed
    }

    /// `RevisionDiff` outcome: diff two historical revisions of the same file.
    fn on_revision_diff(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<(String, String), String>,
        old_label: String,
        new_label: String,
    ) {
        match result {
            Ok((old_content, new_content)) => {
                let diff = crate::diff::unified_diff(
                    &old_label,
                    &old_content,
                    &new_label,
                    &new_content,
                    state.ignore_trailing_newline,
                );
                let identical = old_content == new_content;
                // `enter_diff` (via `enter`) parks the live Revisions screen so Esc
                // restores list cursor/entries.
                state.enter_diff(diff, String::new(), PathBuf::new(), PathBuf::new());
                set_diff_identical(state, identical);
            }
            Err(error) => state.set_status(error),
        }
    }

    /// `RestoreRevisionReady` outcome. Returns [`LoopFlow::SkipIteration`] when the revision
    /// content matches current (nothing to restore).
    fn on_restore_revision_ready(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<(String, String), String>,
        gist_id: String,
        filename: String,
        version: String,
        version_label: String,
    ) -> LoopFlow {
        match result {
            Ok((revision_content, current_content)) => {
                if revision_content == current_content {
                    state.set_status("revision matches current — nothing to restore");
                    return LoopFlow::SkipIteration;
                }
                let old_label = format!("revision {version_label}");
                let new_label = format!("current {filename}");
                let diff = crate::diff::unified_diff(
                    &old_label,
                    &revision_content,
                    &new_label,
                    &current_content,
                    state.ignore_trailing_newline,
                );
                // `enter_confirm` (via `enter`) parks the live Revisions screen so
                // cancel restores cursor/entries.
                state.enter_confirm(
                    PendingAction::RestoreRevision {
                        gist_id,
                        filename,
                        version,
                        version_label,
                        content: revision_content,
                    },
                    diff,
                );
            }
            Err(error) => state.set_status(error),
        }
        LoopFlow::Proceed
    }

    /// `GistStarToggle` outcome.
    fn on_gist_star_toggle(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<(), String>,
        gist_id: String,
        starred: bool,
    ) {
        match result {
            Ok(()) => {
                if starred {
                    state.starred_gist_ids.insert(gist_id.clone());
                    state.set_status(format!("starred {gist_id}"));
                } else {
                    state.starred_gist_ids.remove(&gist_id);
                    state.set_status(format!("unstarred {gist_id}"));
                }
                self.request_gist_fetch(state);
            }
            Err(error) => state.set_status(format!("star toggle failed: {error}")),
        }
    }

    /// `ForkGist` outcome.
    fn on_fork_gist(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<(), String>,
        gist_id: String,
    ) {
        match result {
            Ok(()) => {
                state.set_status(format!("forked {gist_id} into your account"));
                self.request_gist_fetch(state);
            }
            Err(error) => state.set_status(format!("fork failed: {error}")),
        }
    }

    /// `RestoreRevisionDone` outcome: return to the Revisions screen and re-fetch both the
    /// gist list and the revision history for the restored gist.
    fn on_restore_revision_done(
        &mut self,
        state: &mut AppState,
        result: std::result::Result<(), String>,
        gist_id: String,
        filename: String,
    ) {
        match result {
            Ok(_) => {
                state
                    .gist_content_cache
                    .remove(&(gist_id.clone(), filename.clone()));
                state.set_status(format!(
                    "Restored {filename} from old revision (new revision created)"
                ));
                // Return to the revisions list `enter_confirm` parked when the
                // restore confirm was entered.
                state.leave();
                if !state.screen.is_revisions() {
                    state.screen = Screen::Revisions(Box::default());
                }
                let gist_id = state.revision_mut().and_then(|rev| {
                    rev.index = 0;
                    rev.entries = None;
                    rev.fetch_error = None;
                    rev.gist_id.clone()
                });
                self.request_gist_fetch(state);
                if let Some(gist_id) = gist_id {
                    self.spawn_action(state, "Loading revisions…", move || {
                        let result = crate::gh::fetch_gist_commits_json(&gist_id)
                            .map_err(|e| e.to_string())
                            .and_then(|raw| {
                                crate::gh::parse_gist_commits_json(&raw).map_err(|e| e.to_string())
                            });
                        BgTaskOutcome::RevisionsFetched { gist_id, result }
                    });
                }
            }
            Err(error) => {
                state.set_status(format!("restore failed: {error}"));
                // Stay on Confirm payload if still open.
            }
        }
    }
}

/// Set `state`'s diff-payload `identical` flag, if a diff payload is currently open. Shared by
/// the three action outcomes that build a diff and check content identity byte-for-byte:
/// [`Jobs::on_preview_diff`], [`Jobs::on_download_selected`], [`Jobs::on_revision_diff`].
fn set_diff_identical(state: &mut AppState, identical: bool) {
    if let Some(d) = state.diff_mut() {
        d.identical = identical;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn gist_file_ref(gist_id: &str, filename: &str) -> crate::domain::GistFileRef {
        crate::domain::GistFileRef::new(gist_id, filename, None)
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
            BgTaskOutcome::DeleteGist {
                result: Ok(()),
                gist_id: "g1".into(),
            },
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

    // ---- on_preview_diff -----------------------------------------------------

    #[test]
    fn on_preview_diff_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_preview_diff(
            &mut state,
            Err("boom".into()),
            None,
            "local".into(),
            "gist".into(),
            PathBuf::from("target"),
            false,
        );

        assert_eq!(state.status.as_deref(), Some("fetch failed: boom"));
    }

    #[test]
    fn on_preview_diff_ok_without_local_enters_diff() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_preview_diff(
            &mut state,
            Ok("remote body".into()),
            None,
            "local".into(),
            "gist".into(),
            PathBuf::from("target"),
            false,
        );

        let diff = state.diff().expect("expected Screen::Diff");
        assert_eq!(diff.remote_content, "remote body");
        assert!(!diff.identical);
    }

    // ---- on_download_selected -------------------------------------------------

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
    fn on_download_selected_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_download_selected(
            &mut state,
            Err("boom".into()),
            PathBuf::from("target"),
            "local".into(),
            "gist".into(),
            gist_file_ref("g1", "a.txt"),
        );

        assert_eq!(state.status.as_deref(), Some("fetch failed: boom"));
    }

    #[test]
    fn on_download_selected_ok_existing_target_enters_diff() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        std::fs::write(&target, "local body").unwrap();
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_download_selected(
            &mut state,
            Ok("remote body".into()),
            target.clone(),
            "local".into(),
            "gist".into(),
            gist_file_ref("g1", "a.txt"),
        );

        let diff = state.diff().expect("expected Screen::Diff");
        assert_eq!(diff.remote_content, "remote body");
        // `enter_diff` takes `staged_diff_gist` and moves it onto the `DiffState` itself.
        assert_eq!(diff.gist_id.as_deref(), Some("g1"));
        assert_eq!(diff.gist_filename.as_deref(), Some("a.txt"));
        assert!(state.staged_diff_gist.is_none());
    }

    // ---- on_upload_preview ------------------------------------------------------

    #[test]
    fn on_upload_preview_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_upload_preview(
            &mut state,
            Err("boom".into()),
            gist_file_ref("g1", "a.txt"),
            PathBuf::from("a.txt"),
            "local".into(),
            "gist".into(),
        );

        assert_eq!(state.status.as_deref(), Some("fetch failed: boom"));
    }

    #[test]
    fn on_upload_preview_ok_enters_confirm() {
        let dir = tempfile::tempdir().unwrap();
        let local_path = dir.path().join("a.txt");
        std::fs::write(&local_path, "local body").unwrap();
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_upload_preview(
            &mut state,
            Ok("remote body".into()),
            gist_file_ref("g1", "a.txt"),
            local_path.clone(),
            "local".into(),
            "gist".into(),
        );

        assert!(state.screen.is_confirm());
        assert!(matches!(
            state.pending_action(),
            Some(PendingAction::Upload { gist_id, filename, .. })
                if gist_id == "g1" && filename == "a.txt"
        ));
    }

    // ---- on_upload_replace / on_create_gist / on_delete_gist / on_remove_file /
    // on_apply_description / on_compact_gist / on_gist_star_toggle / on_fork_gist /
    // on_restore_revision_done — Err arms only.
    //
    // Every Ok arm of these methods ends by calling `request_gist_fetch`, which
    // spawns a real background gist fetch (`spawn_gist_fetch`): it shells out to `gh`
    // (`check_gh_ready` + list/starred/user-login fetches) on success. That is a live
    // `gh`/network call this project's tests must not make, and there is no injectable
    // seam for it (unlike `update_check_path`). `on_restore_revision_done`'s Ok arm
    // additionally spawns a second live `gh` call via `spawn_action`. Only the
    // side-effect-free Err arms are covered directly.

    #[test]
    fn on_upload_replace_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_upload_replace(&mut state, Err("boom".into()), gist_file_ref("g1", "a.txt"));

        assert_eq!(state.status.as_deref(), Some("upload failed: boom"));
    }

    #[test]
    fn on_create_gist_err_resets_screen() {
        let mut state = initial_state();
        state.description_input.set("desc");
        let mut jobs = empty_jobs();

        jobs.on_create_gist(&mut state, Err("boom".into()), PathBuf::from("a.txt"), true);

        assert_eq!(state.status.as_deref(), Some("create failed: boom"));
        assert!(matches!(state.screen, Screen::List));
        assert!(state.description_input.is_empty());
    }

    #[test]
    fn on_delete_gist_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_delete_gist(&mut state, Err("boom".into()), "g1".into());

        assert_eq!(state.status.as_deref(), Some("delete failed: boom"));
    }

    #[test]
    fn on_remove_file_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_remove_file(&mut state, Err("boom".into()), "g1".into(), "a.txt".into());

        assert_eq!(state.status.as_deref(), Some("remove failed: boom"));
    }

    #[test]
    fn on_apply_description_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_apply_description(&mut state, Err("boom".into()), "g1".into());

        assert_eq!(
            state.status.as_deref(),
            Some("description update failed: boom")
        );
    }

    #[test]
    fn on_compact_gist_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_compact_gist(&mut state, Err("boom".into()), "demo".into(), 3);

        assert_eq!(state.status.as_deref(), Some("compact failed: boom"));
    }

    #[test]
    fn on_gist_star_toggle_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_gist_star_toggle(&mut state, Err("boom".into()), "g1".into(), true);

        assert_eq!(state.status.as_deref(), Some("star toggle failed: boom"));
    }

    #[test]
    fn on_fork_gist_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_fork_gist(&mut state, Err("boom".into()), "g1".into());

        assert_eq!(state.status.as_deref(), Some("fork failed: boom"));
    }

    #[test]
    fn on_restore_revision_done_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_restore_revision_done(&mut state, Err("boom".into()), "g1".into(), "a.txt".into());

        assert_eq!(state.status.as_deref(), Some("restore failed: boom"));
    }

    // ---- on_preview_content (fully safe, both arms) ---------------------------

    #[test]
    fn on_preview_content_ok_caches_and_enters_preview() {
        let mut state = initial_state();
        let file = gist_file_ref("g1", "a.txt");
        let mut jobs = empty_jobs();

        jobs.on_preview_content(&mut state, Ok("body".into()), file.clone(), "a.txt".into());

        assert_eq!(
            state.gist_content_cache.get(&file.cache_key()),
            Some(&"body".to_string())
        );
        let preview = state.preview().expect("expected Screen::Preview");
        assert_eq!(preview.text, "body");
    }

    #[test]
    fn on_preview_content_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_preview_content(
            &mut state,
            Err("boom".into()),
            gist_file_ref("g1", "a.txt"),
            "a.txt".into(),
        );

        assert_eq!(state.status.as_deref(), Some("fetch failed: boom"));
    }

    // ---- on_compact_analyze (fully safe) ---------------------------------------

    #[test]
    fn on_compact_analyze_single_revision_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_compact_analyze(&mut state, Ok(1), "g1".into(), "demo".into());

        assert_eq!(
            state.status.as_deref(),
            Some("\"demo\" already has a single revision — nothing to compact")
        );
    }

    #[test]
    fn on_compact_analyze_multi_revision_enters_confirm() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_compact_analyze(&mut state, Ok(4), "g1".into(), "demo".into());

        assert!(matches!(
            state.pending_action(),
            Some(PendingAction::CompactGist { gist_id, count, .. })
                if gist_id == "g1" && *count == 4
        ));
    }

    #[test]
    fn on_compact_analyze_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_compact_analyze(&mut state, Err("boom".into()), "g1".into(), "demo".into());

        assert_eq!(state.status.as_deref(), Some("revision check failed: boom"));
    }

    // ---- on_comments_initial_loaded / on_comments_older_loaded (fully safe) ---

    #[test]
    fn on_comments_initial_loaded_ok_dispatches_to_detail() {
        let mut state = initial_state();
        state.screen = Screen::GistDetail(Box::new(DetailState {
            gist_id: Some("g1".into()),
            comments_loading: true,
            ..Default::default()
        }));
        let mut jobs = empty_jobs();

        jobs.on_comments_initial_loaded(
            &mut state,
            "g1".into(),
            Ok(crate::tui::InitialComments {
                comments: vec![GistComment {
                    author: "alice".into(),
                    created_at: "2026-01-01T00:00:00Z".into(),
                    body: "hi".into(),
                }],
                total: 1,
                oldest_page: 1,
            }),
        );

        let detail = state.detail().expect("expected Screen::GistDetail");
        assert!(!detail.comments_loading);
        assert_eq!(detail.comments_total, Some(1));
    }

    #[test]
    fn on_comments_older_loaded_err_dispatches_to_detail() {
        let mut state = initial_state();
        state.screen = Screen::GistDetail(Box::new(DetailState {
            gist_id: Some("g1".into()),
            comments_loading_more: true,
            ..Default::default()
        }));
        let mut jobs = empty_jobs();

        jobs.on_comments_older_loaded(&mut state, "g1".into(), Err("boom".into()));

        let detail = state.detail().expect("expected Screen::GistDetail");
        assert!(!detail.comments_loading_more);
    }

    // ---- on_revisions_fetched --------------------------------------------------

    #[test]
    fn on_revisions_fetched_returns_skip_iteration_on_gist_mismatch() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some("g1".into()),
            ..Default::default()
        }));
        let mut jobs = empty_jobs();

        let flow = jobs.on_revisions_fetched(&mut state, "other-gist".into(), Ok(Vec::new()));

        assert!(matches!(flow, LoopFlow::SkipIteration));
        // Not applied — still no entries.
        assert!(state.revision().unwrap().entries.is_none());
    }

    #[test]
    fn on_revisions_fetched_ok_sets_entries() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some("g1".into()),
            ..Default::default()
        }));
        let entry = GistRevision {
            version: "abc123".into(),
            committed_at: "2026-01-01T00:00:00Z".into(),
            user: "alice".into(),
            change_status: crate::domain::GistRevisionChangeStatus {
                total: 1,
                additions: 1,
                deletions: 0,
            },
        };
        let mut jobs = empty_jobs();

        let flow =
            jobs.on_revisions_fetched(&mut state, "g1".into(), Ok(vec![entry.clone(), entry]));

        assert!(matches!(flow, LoopFlow::Proceed));
        assert_eq!(state.revision().unwrap().entries.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn on_revisions_fetched_err_sets_fetch_error() {
        let mut state = initial_state();
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some("g1".into()),
            ..Default::default()
        }));
        let mut jobs = empty_jobs();

        jobs.on_revisions_fetched(&mut state, "g1".into(), Err("boom".into()));

        let rev = state.revision().unwrap();
        assert_eq!(rev.fetch_error.as_deref(), Some("boom"));
        assert_eq!(rev.entries, Some(Vec::new()));
    }

    // ---- on_revision_diff -------------------------------------------------------

    #[test]
    fn on_revision_diff_ok_enters_diff() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_revision_diff(
            &mut state,
            Ok(("old body".into(), "new body".into())),
            "old".into(),
            "new".into(),
        );

        let diff = state.diff().expect("expected Screen::Diff");
        assert!(!diff.identical);
    }

    #[test]
    fn on_revision_diff_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_revision_diff(&mut state, Err("boom".into()), "old".into(), "new".into());

        assert_eq!(state.status.as_deref(), Some("boom"));
    }

    // ---- on_restore_revision_ready ----------------------------------------------

    #[test]
    fn on_restore_revision_ready_returns_skip_iteration_when_identical() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        let flow = jobs.on_restore_revision_ready(
            &mut state,
            Ok(("same".into(), "same".into())),
            "g1".into(),
            "a.txt".into(),
            "abc123".into(),
            "abc1234".into(),
        );

        assert!(matches!(flow, LoopFlow::SkipIteration));
        assert_eq!(
            state.status.as_deref(),
            Some("revision matches current — nothing to restore")
        );
    }

    #[test]
    fn on_restore_revision_ready_ok_enters_confirm_when_different() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        let flow = jobs.on_restore_revision_ready(
            &mut state,
            Ok(("old".into(), "new".into())),
            "g1".into(),
            "a.txt".into(),
            "abc123".into(),
            "abc1234".into(),
        );

        assert!(matches!(flow, LoopFlow::Proceed));
        assert!(matches!(
            state.pending_action(),
            Some(PendingAction::RestoreRevision { gist_id, filename, .. })
                if gist_id == "g1" && filename == "a.txt"
        ));
    }

    #[test]
    fn on_restore_revision_ready_err_sets_status() {
        let mut state = initial_state();
        let mut jobs = empty_jobs();

        jobs.on_restore_revision_ready(
            &mut state,
            Err("boom".into()),
            "g1".into(),
            "a.txt".into(),
            "abc123".into(),
            "abc1234".into(),
        );

        assert_eq!(state.status.as_deref(), Some("boom"));
    }
}
