//! Background workers, channels, and absorb of async results for the TUI event loop.
//! Extracted from `run_loop` (issue #225) so spawn/absorb stays separate from key IO dispatch.

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
        gist_id: String,
        filename: String,
    },
    UploadPreview {
        result: std::result::Result<String, String>,
        gist_id: String,
        filename: String,
        local_path: PathBuf,
        local_label: String,
        gist_label: String,
    },
    UploadReplace {
        result: std::result::Result<(), String>,
        gist_id: String,
        filename: String,
    },
    CreateGist {
        result: std::result::Result<(), String>,
        local_path: PathBuf,
        public: bool,
    },
    PreviewContent {
        result: std::result::Result<String, String>,
        key: (String, String),
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
pub(super) type LocalScanRx = Option<std::sync::mpsc::Receiver<(u64, Vec<LocalCandidate>)>>;

pub(super) fn spawn_local_scan(
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
pub(super) type BgRx = Option<std::sync::mpsc::Receiver<(u64, BgTaskOutcome)>>;

/// Run `work` on a background thread, wiring its result channel into `bg_rx` and setting
/// the in-progress `bg_task_msg` the main loop renders. The worker's returned
/// [`BgTaskOutcome`] is sent back stamped with a generation id so cancelled or
/// superseded results can be ignored (issue #221).
pub(super) fn spawn_bg<F>(state: &mut AppState, bg_rx: &mut BgRx, msg: impl Into<String>, work: F)
where
    F: FnOnce() -> BgTaskOutcome + Send + 'static,
{
    let generation = state.begin_bg_task();
    state.bg_task_msg = Some(msg.into());
    let (tx, rx) = std::sync::mpsc::channel();
    *bg_rx = Some(rx);
    std::thread::spawn(move || {
        let _ = tx.send((generation, work()));
    });
}

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

/// The pin currently selected in the Pins screen, if any.
pub(super) fn selected_pin(state: &AppState) -> Option<crate::domain::PinnedMapping> {
    state
        .selected_pin_index()
        .and_then(|i| state.pinned.get(i).cloned())
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
    bg_rx: &mut BgRx,
    m: &crate::domain::PinnedMapping,
) {
    state.diff_return = Screen::Pins;
    let local_path = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: gist_id.clone(),
        filename: filename.clone(),
        local_path: local_path.clone(),
    });
    let raw_url = state.gist_file_raw_url(&gist_id, &filename);
    let gist_file = GistFile::for_sync(gist_id.clone(), filename.clone(), raw_url.clone());
    let (local_label, gist_label) = diff_labels(Some(&local_path), &gist_file);
    spawn_bg(state, bg_rx, "Loading diff…", move || {
        let result = fetch_gist_content(&gist_id, &filename, raw_url.as_deref());
        BgTaskOutcome::UploadPreview {
            result,
            gist_id,
            filename,
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
    bg_rx: &mut BgRx,
    m: &crate::domain::PinnedMapping,
) {
    state.diff_return = Screen::Pins;
    let target = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    let raw_url = state.gist_file_raw_url(&gist_id, &filename);
    let gist_file = GistFile::for_sync(gist_id.clone(), filename.clone(), raw_url.clone());
    let (local_label, gist_label) = diff_labels(Some(&target), &gist_file);
    spawn_bg(state, bg_rx, "Downloading…", move || {
        let result = fetch_gist_content(&gist_id, &filename, raw_url.as_deref());
        BgTaskOutcome::DownloadSelected {
            result,
            target,
            local_label,
            gist_label,
            gist_id,
            filename,
        }
    });
}

/// Spawn a read-only diff (gist vs local) for a pin, landing on `Screen::Diff`.
pub(super) fn spawn_pin_diff(
    state: &mut AppState,
    bg_rx: &mut BgRx,
    m: &crate::domain::PinnedMapping,
) {
    let local_abs = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    // Record the pin's identity so that `d`/`u` in the diff screen can attribute
    // the action to this pin (record_pin_sync) and use the correct local file
    // instead of the Files-view selection (is_pin_diff_context check).
    state.download_gist_id = Some(gist_id.clone());
    state.download_gist_filename = Some(filename.clone());
    // Pull the real `updated_at` from the loaded gists so the diff header shows the
    // gist mtime (matching the Pins list) instead of "unknown".
    let updated_at = state
        .gists
        .iter()
        .find(|g| g.gist_id == gist_id && g.filename == filename)
        .map(|g| g.updated_at.clone())
        .unwrap_or_default();
    let raw_url = state.gist_file_raw_url(&gist_id, &filename);
    let gist_file = GistFile {
        updated_at,
        ..GistFile::for_sync(gist_id.clone(), filename.clone(), raw_url.clone())
    };
    let (local_label, gist_label) = diff_labels(Some(&local_abs), &gist_file);
    let target = local_abs.clone();
    spawn_bg(state, bg_rx, "Loading diff…", move || {
        let result = fetch_gist_content(&gist_id, &filename, raw_url.as_deref());
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
            }
        }
    }
}

/// Builds the `--- local` / `+++ gist` diff header labels showing each side's filename and
/// last-modified time, plus the gist's id.
pub(super) fn open_browser(state: &mut AppState) {
    let gist_id = state.context_gist_id();
    let Some(gist_id) = gist_id else {
        return;
    };
    let plan = crate::actions::open_browser_command(&gist_id);
    // Fire-and-forget on a detached thread: `gh gist view --web` resolves the URL and shells
    // out to the OS opener, which can stall the event loop for a perceptible window if run
    // inline. A launch failure is rare and self-evident (no browser appears), so we report
    // optimistically rather than thread the result back through a background outcome.
    std::thread::spawn(move || {
        let _ = crate::actions::execute_command(&plan);
    });
    state.set_status(format!("Opening gist {gist_id} in the browser…"));
}

pub(super) fn open_repo_url(state: &mut AppState) {
    let url = env!("CARGO_PKG_REPOSITORY");
    let plan = crate::actions::open_url_command(url);
    std::thread::spawn(move || {
        let _ = crate::actions::execute_command(&plan);
    });
    state.set_status("Opening GitHub repository in the browser…");
}

/// Copies the context gist's web URL to the system clipboard. On the Preview screen the
/// URL comes from the previewed file's gist; elsewhere from the current selection.
pub(super) fn copy_gist_url(state: &mut AppState) {
    let gist_id = match state.screen {
        Screen::Preview => state.preview_gist_key.as_ref().map(|(id, _)| id.clone()),
        _ => state.context_gist_id(),
    };
    let Some(gist_id) = gist_id else {
        state.set_status("no gist selected to copy");
        return;
    };
    let url = crate::actions::gist_web_url(&gist_id);
    match crate::actions::copy_to_clipboard(&url) {
        Ok(_) => state.set_status(format!("Copied URL to clipboard: {url}")),
        Err(error) => state.set_status(format!("copy failed: {error}")),
    }
}

/// Copies the full previewed file content (the text shown on `Screen::Preview`) to the
/// system clipboard.
pub(super) fn copy_preview_content(state: &mut AppState) {
    if state.diff_text.is_empty() {
        state.set_status("no content to copy");
        return;
    }
    let bytes = state.diff_text.len();
    match crate::actions::copy_to_clipboard(&state.diff_text) {
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

/// Opens the selected local file in `$VISUAL`/`$EDITOR` (default `vi`). A terminal editor
/// needs the full terminal, so the TUI leaves raw mode / the alternate screen for the
/// duration and restores afterwards. `$EDITOR` may include flags (e.g. `code --wait`); a
/// wait flag is added automatically for known GUI editors (see [`editor_command`]).
pub(super) fn edit_local(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
) -> Result<()> {
    let Some(local) = state.selected_local() else {
        return Ok(());
    };
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
        .arg(&local.path)
        .status();
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    if state.mouse_enabled {
        execute!(terminal.backend_mut(), EnableMouseCapture)?;
    }
    terminal.clear()?;

    match result {
        Ok(_) => state.set_status(format!(
            "Edited {}",
            crate::config::display_path(&local.path)
        )),
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
    channels: &mut BgChannels,
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
        }) = state.pending_action.clone()
        else {
            let _ = std::fs::remove_file(&temp_file_path);
            return Ok(());
        };
        channels.upload_edit_watch = Some(spawn_upload_edit_watch(
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

pub(super) fn download(state: &mut AppState) {
    let target = state.download_target.clone();
    let content = state.preview_remote.clone();
    let return_screen = state.diff_return;
    match crate::actions::execute_download(&target, &content, true) {
        Ok(()) => {
            state.set_status(format!(
                "Downloaded {}",
                target
                    .file_name()
                    .unwrap_or(target.as_os_str())
                    .to_string_lossy()
            ));
            if let (Some(gid), Some(fname)) = (
                state.download_gist_id.clone(),
                state.download_gist_filename.clone(),
            ) {
                record_pin_sync(
                    state,
                    &target,
                    &gid,
                    &fname,
                    &content,
                    Some(crate::domain::SyncDirection::Download),
                );
            }
            state.back_to_list();
            state.screen = return_screen;
            refresh_locals(state);
        }
        Err(error) => {
            state.set_status(format!("download failed: {error}"));
            state.screen = Screen::Diff;
        }
    }
}

/// Quick flat re-scan used after a download/upload to make the new file visible immediately.
/// Always non-recursive since downloads only write to cwd root.
pub(super) fn refresh_locals(state: &mut AppState) {
    let selected = state.selected_local().map(|c| c.path.clone());
    if let Ok(locals) = crate::local::discover_local_candidates(
        &state.cwd,
        &state.pinned,
        false,
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
                .get(state.config.index)
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

pub(super) fn pin_selected(state: &mut AppState) {
    let (Some(local), Some(gist)) = (state.selected_local(), state.selected_gist()) else {
        return;
    };
    let result = crate::config::config_path().and_then(|path| {
        let config = crate::config::load_config(&path)?;
        crate::actions::pin_mapping(&path, config, &local.path, &gist.file, None, None)
    });
    match result {
        Ok(config) => {
            state.pinned = config.pinned;
            state.skip_dirs = config.skip_dirs;
            state.scan_depth = config.scan_depth;
            state.set_status(format!(
                "Pinned {} <-> {}",
                local.path.display(),
                gist.file.filename
            ));
        }
        Err(error) => state.set_status(format!("pin failed: {error}")),
    }
}

pub(super) fn unpin_selected(state: &mut AppState) {
    let Some(local) = state.selected_local() else {
        return;
    };
    let result = crate::config::config_path().and_then(|path| {
        let config = crate::config::load_config(&path)?;
        crate::actions::unpin_mapping(&path, config, &local.path)
    });
    match result {
        Ok(config) => {
            state.pinned = config.pinned;
            state.skip_dirs = config.skip_dirs;
            state.scan_depth = config.scan_depth;
            state.set_status(format!(
                "Unpinned {}",
                crate::config::display_path(&local.path)
            ));
        }
        Err(error) => state.set_status(format!("unpin failed: {error}")),
    }
}

pub(super) fn unpin_at_pin_index(state: &mut AppState) {
    let Some(idx) = state.selected_pin_index() else {
        return;
    };
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
            state.pins.index = state
                .pins
                .index
                .min(state.visible_pin_indices().len().saturating_sub(1));
            refresh_locals(state);
            state.set_status(format!("Unpinned {label}"));
        }
        Err(error) => state.set_status(format!("unpin failed: {error}")),
    }
}

pub(super) fn upload_local_filename(local: &std::path::Path) -> Option<String> {
    local.file_name().and_then(|n| n.to_str()).map(String::from)
}

/// The background-work receivers `run_loop` drains each iteration, bundled so the
/// extracted loop steps take one `&mut BgChannels` instead of separate `&mut` parameters.
pub(super) struct BgChannels {
    pub(super) update: Option<std::sync::mpsc::Receiver<crate::update_check::UpdateCheckOutcome>>,
    pub(super) gist: Option<std::sync::mpsc::Receiver<GistFetchResult>>,
    pub(super) fork: Option<std::sync::mpsc::Receiver<std::collections::HashMap<String, u32>>>,
    pub(super) star: Option<std::sync::mpsc::Receiver<std::collections::HashMap<String, u32>>>,
    pub(super) fork_meta: Option<std::sync::mpsc::Receiver<ForkMetaResult>>,
    pub(super) local: LocalScanRx,
    /// Streams `UploadEditWatchEvent`s while a GUI editor has the upload-redact temp file
    /// open (see `spawn_upload_edit_watch`). Unlike the other fields above (one-shot
    /// results), this channel can carry multiple `ContentChanged` events before its
    /// terminal `EditorClosed`/`ReadError` — drained in a loop in `absorb_background_results`.
    pub(super) upload_edit_watch: Option<std::sync::mpsc::Receiver<UploadEditWatchEvent>>,
    pub(super) bg: BgRx,
}

pub(super) enum LoopFlow {
    Proceed,
    SkipIteration,
    Quit,
}

pub(super) fn absorb_background_results(
    state: &mut AppState,
    channels: &mut BgChannels,
    update_check_path: &Option<std::path::PathBuf>,
) -> Result<LoopFlow> {
    // Absorb the background gist list once it arrives.
    if state.loading {
        if let Some(ref rx) = channels.gist {
            if let Ok((
                gists,
                starred,
                starred_ids,
                user_login,
                comment_counts,
                owned_raw,
                starred_raw,
            )) = rx.try_recv()
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
                if count > 0 && state.gist_manager.index >= count {
                    state.gist_manager.index = count - 1;
                }
                channels.gist = None;
                let gist_ids: std::collections::HashSet<String> = state
                    .gists
                    .iter()
                    .chain(state.starred_gists.iter())
                    .map(|g| g.gist_id.clone())
                    .collect();
                channels.fork = Some(spawn_fork_count_fetch(
                    owned_raw,
                    starred_raw,
                    gist_ids.clone(),
                ));
                channels.fork_meta = Some(spawn_fork_metadata_fetch(
                    state.gists.iter().map(|g| g.gist_id.clone()).collect(),
                ));
                let node_ids =
                    crate::gh::merge_gist_node_id_maps(&state.gists, &state.starred_gists);
                channels.star = Some(spawn_star_count_fetch(node_ids));
            }
        }
    }

    if let Some(ref rx) = channels.fork {
        if let Ok(fork_counts) = rx.try_recv() {
            state.gist_fork_counts = fork_counts;
            persist_gist_cache_from_state(state);
            channels.fork = None;
        }
    }

    if let Some(ref rx) = channels.star {
        if let Ok(star_counts) = rx.try_recv() {
            state.gist_star_counts = star_counts;
            persist_gist_cache_from_state(state);
            channels.star = None;
        }
    }

    if let Some(ref rx) = channels.fork_meta {
        if let Ok(result) = rx.try_recv() {
            match result {
                Ok(fork_of) => {
                    crate::gh::apply_fork_of_ids(&mut state.gists, &fork_of);
                    persist_gist_cache_from_state(state);
                }
                Err(error) => state.set_status(format!("fork detection unavailable: {error}")),
            }
            channels.fork_meta = None;
        }
    }

    // Absorb a completed background local scan (ignore stale generations — issue #221).
    if state.local_scanning {
        if let Some(ref rx) = channels.local {
            if let Ok((generation, locals)) = rx.try_recv() {
                channels.local = None;
                if state.apply_local_scan_if_current(generation, locals) {
                    state.status = None;
                }
                // Stale: a newer scan is (or was) in flight; leave spinner/list alone.
            }
        }
    }

    // Absorb the background update-check result: show the hint and persist the throttle.
    // Failed checks are silent and not recorded, so they retry on the next launch.
    if let Some(ref rx) = channels.update {
        if let Ok(outcome) = rx.try_recv() {
            channels.update = None;
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

    // Absorb upload-edit-watch events. Unlike the other channels above (one-shot), this one
    // can carry several `ContentChanged` events before its terminal EditorClosed/ReadError —
    // drain all of them so a burst of saves doesn't lag a tick behind.
    let mut upload_watch_finished = false;
    if let Some(ref rx) = channels.upload_edit_watch {
        while let Ok(event) = rx.try_recv() {
            if matches!(
                event,
                UploadEditWatchEvent::EditorClosed { .. } | UploadEditWatchEvent::ReadError { .. }
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
        channels.upload_edit_watch = None;
    }

    // Absorb a completed background per-action task (ignore stale generations — issue #221).
    if let Some(ref rx) = channels.bg {
        if let Ok((generation, outcome)) = rx.try_recv() {
            channels.bg = None;
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
                    } => match result {
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
                                    state.enter_diff(
                                        diff,
                                        remote,
                                        local_path.unwrap_or_default(),
                                        target,
                                    );
                                    state.diff_identical = identical;
                                    // A pin diff that turns out identical confirms the cached
                                    // last_seen_hash is (still) accurate — refresh it for free
                                    // using the content we already fetched, so the Pins list's
                                    // content-hash check (AppState::pin_sync_status) stays
                                    // correct even if the gist changed elsewhere since the last
                                    // real sync. Hash the LOCAL content's raw bytes (not the
                                    // trailing-newline-normalized `identical` comparison), so
                                    // this matches the raw-byte hashing pin_sync_status does.
                                    if identical {
                                        if let (Some(gid), Some(fname)) = (
                                            state.download_gist_id.clone(),
                                            state.download_gist_filename.clone(),
                                        ) {
                                            let local_abs = state.preview_local.clone();
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
                    },
                    BgTaskOutcome::DownloadSelected {
                        result,
                        target,
                        local_label,
                        gist_label,
                        gist_id,
                        filename,
                    } => match result {
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
                                        state.download_gist_id = Some(gist_id);
                                        state.download_gist_filename = Some(filename);
                                        state.enter_diff(diff, remote, target.clone(), target);
                                        state.diff_identical = identical;
                                    }
                                    Err(error) => state.set_status(error),
                                }
                            } else {
                                match crate::actions::execute_download(&target, &remote, false) {
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
                                            &gist_id,
                                            &filename,
                                            &remote,
                                            Some(crate::domain::SyncDirection::Download),
                                        );
                                        refresh_locals(state);
                                    }
                                    Err(error) => {
                                        state.set_status(format!("download failed: {error}"))
                                    }
                                }
                            }
                        }
                        Err(error) => state.set_status(format!("fetch failed: {error}")),
                    },
                    BgTaskOutcome::UploadPreview {
                        result,
                        gist_id,
                        filename,
                        local_path,
                        local_label,
                        gist_label,
                    } => match result {
                        Ok(remote) => {
                            state.pending_action = Some(PendingAction::Upload {
                                gist_id,
                                filename,
                                local_path: local_path.clone(),
                            });
                            match state.init_upload_state(
                                &local_path,
                                Some(remote),
                                local_label,
                                gist_label,
                            ) {
                                Ok(()) => {
                                    state.diff_scroll = 0;
                                    state.diff_hscroll = 0;
                                    state.status = None;
                                    state.screen = Screen::Confirm;
                                }
                                Err(error) => {
                                    state.pending_action = None;
                                    state.set_status(format!(
                                        "cannot read {}: {error}",
                                        crate::config::display_path(&local_path)
                                    ));
                                }
                            }
                        }
                        Err(error) => state.set_status(format!("fetch failed: {error}")),
                    },
                    BgTaskOutcome::UploadReplace {
                        result,
                        gist_id,
                        filename,
                    } => match result {
                        Ok(_) => {
                            state
                                .gist_content_cache
                                .remove(&(gist_id.clone(), filename.clone()));
                            state.set_status(format!("Uploaded {} to gist {}", filename, gist_id));
                            if let Some(local_path) = state.upload_local_path() {
                                let content = state.content_to_upload();
                                record_pin_sync(
                                    state,
                                    &local_path,
                                    &gist_id,
                                    &filename,
                                    &content,
                                    Some(crate::domain::SyncDirection::Upload),
                                );
                            }
                            // Return to wherever this upload was initiated from (List, or Pins
                            // for a pin push) instead of always snapping to List.
                            let return_screen = state.diff_return;
                            state.back_to_list();
                            state.screen = return_screen;
                            state.loading = true;
                            channels.gist = Some(spawn_gist_fetch());
                        }
                        Err(error) => {
                            state.set_status(format!("upload failed: {error}"));
                            state.screen = Screen::Confirm;
                        }
                    },
                    BgTaskOutcome::CreateGist {
                        result,
                        local_path,
                        public,
                    } => match result {
                        Ok(_) => {
                            let visibility = if public { "public" } else { "secret" };
                            state.set_status(format!(
                                "Created {} gist from {}",
                                visibility,
                                crate::config::display_path(&local_path)
                            ));
                            state.description_input.clear();
                            state.back_to_list();
                            state.loading = true;
                            channels.gist = Some(spawn_gist_fetch());
                        }
                        Err(error) => {
                            state.set_status(format!("create failed: {error}"));
                            state.screen = Screen::List;
                            state.pending_action = None;
                            state.description_input.clear();
                        }
                    },
                    BgTaskOutcome::PreviewContent {
                        result,
                        key,
                        preview_title,
                    } => match result {
                        Ok(content) => {
                            state
                                .gist_content_cache
                                .insert(key.clone(), content.clone());
                            state.preview_title = preview_title;
                            state.preview_gist_key = Some(key);
                            state.diff_text = content;
                            state.diff_scroll = 0;
                            state.diff_hscroll = 0;
                            state.status = None;
                            state.screen = Screen::Preview;
                        }
                        Err(error) => state.set_status(format!("fetch failed: {error}")),
                    },
                    BgTaskOutcome::DeleteGist { result, gist_id } => match result {
                        Ok(_) => {
                            state.set_status(format!("Deleted gist {gist_id}"));
                            state.loading = true;
                            channels.gist = Some(spawn_gist_fetch());
                        }
                        Err(error) => state.set_status(format!("delete failed: {error}")),
                    },
                    BgTaskOutcome::RemoveFile {
                        result,
                        gist_id,
                        filename,
                    } => match result {
                        Ok(_) => {
                            state
                                .gist_content_cache
                                .remove(&(gist_id.clone(), filename.clone()));
                            state.set_status(format!("Removed {filename} from gist {gist_id}"));
                            state.loading = true;
                            channels.gist = Some(spawn_gist_fetch());
                        }
                        Err(error) => state.set_status(format!("remove failed: {error}")),
                    },
                    BgTaskOutcome::ApplyDescription { result, gist_id } => match result {
                        Ok(_) => {
                            state.set_status(format!("Updated description for gist {gist_id}"));
                            state.loading = true;
                            channels.gist = Some(spawn_gist_fetch());
                        }
                        Err(error) => {
                            state.set_status(format!("description update failed: {error}"))
                        }
                    },
                    BgTaskOutcome::CompactAnalyze {
                        result,
                        gist_id,
                        label,
                    } => match result {
                        Ok(count) if count <= 1 => state.set_status(format!(
                            "\"{label}\" already has a single revision — nothing to compact"
                        )),
                        Ok(count) => {
                            state.diff_text = format!(
                                "Compact gist {gist_id} (\"{label}\").\n\nIt has {count} revisions. Compacting clones it to a temp dir, squashes the history to a single commit, and force-pushes — the {} older revisions are gone for good.",
                                count - 1
                            );
                            state.diff_scroll = 0;
                            state.diff_hscroll = 0;
                            state.pending_action = Some(PendingAction::CompactGist {
                                gist_id,
                                label,
                                count,
                            });
                            state.screen = Screen::Confirm;
                        }
                        Err(error) => state.set_status(format!("revision check failed: {error}")),
                    },
                    BgTaskOutcome::CompactGist {
                        result,
                        label,
                        count,
                    } => match result {
                        Ok(_) => {
                            state.set_status(format!(
                                "Compacted \"{label}\" ({count} → 1 revision)"
                            ));
                            state.loading = true;
                            channels.gist = Some(spawn_gist_fetch());
                        }
                        Err(error) => state.set_status(format!("compact failed: {error}")),
                    },
                    BgTaskOutcome::CommentsInitialLoaded { gist_id, result } => {
                        state.apply_initial_comments(&gist_id, result);
                    }
                    BgTaskOutcome::CommentsOlderLoaded { gist_id, result } => {
                        state.apply_older_comments(&gist_id, result);
                    }
                    BgTaskOutcome::RevisionsFetched { gist_id, result } => {
                        if state.revision.gist_id.as_deref() != Some(gist_id.as_str()) {
                            return Ok(LoopFlow::SkipIteration);
                        }
                        match result {
                            Ok(entries) => {
                                state.revision.fetch_error = None;
                                state.revision.entries = Some(entries);
                                if state
                                    .revision
                                    .entries
                                    .as_ref()
                                    .is_some_and(|e| e.len() <= 1)
                                {
                                    state.set_status("only one revision — nothing to restore");
                                }
                            }
                            Err(error) => {
                                state.revision.entries = Some(Vec::new());
                                state.revision.fetch_error = Some(error);
                            }
                        }
                    }
                    BgTaskOutcome::RevisionDiff {
                        result,
                        old_label,
                        new_label,
                    } => match result {
                        Ok((old_content, new_content)) => {
                            let diff = crate::diff::unified_diff(
                                &old_label,
                                &old_content,
                                &new_label,
                                &new_content,
                                state.ignore_trailing_newline,
                            );
                            state.diff_text = diff;
                            state.diff_scroll = 0;
                            state.diff_hscroll = 0;
                            state.diff_identical = old_content == new_content;
                            state.diff_return = Screen::Revisions;
                            state.pending_action = None;
                            state.screen = Screen::Diff;
                        }
                        Err(error) => state.set_status(error),
                    },
                    BgTaskOutcome::RestoreRevisionReady {
                        result,
                        gist_id,
                        filename,
                        version,
                        version_label,
                    } => match result {
                        Ok((revision_content, current_content)) => {
                            if revision_content == current_content {
                                state.set_status("revision matches current — nothing to restore");
                                return Ok(LoopFlow::SkipIteration);
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
                            state.diff_text = diff;
                            state.diff_scroll = 0;
                            state.diff_hscroll = 0;
                            state.diff_identical = false;
                            state.pending_action = Some(PendingAction::RestoreRevision {
                                gist_id,
                                filename,
                                version,
                                version_label,
                                content: revision_content,
                            });
                            state.screen = Screen::Confirm;
                        }
                        Err(error) => state.set_status(error),
                    },
                    BgTaskOutcome::GistStarToggle {
                        result,
                        gist_id,
                        starred,
                    } => match result {
                        Ok(()) => {
                            if starred {
                                state.starred_gist_ids.insert(gist_id.clone());
                                state.set_status(format!("starred {gist_id}"));
                            } else {
                                state.starred_gist_ids.remove(&gist_id);
                                state.set_status(format!("unstarred {gist_id}"));
                            }
                            state.loading = true;
                            channels.gist = Some(spawn_gist_fetch());
                        }
                        Err(error) => state.set_status(format!("star toggle failed: {error}")),
                    },
                    BgTaskOutcome::ForkGist { result, gist_id } => match result {
                        Ok(()) => {
                            state.set_status(format!("forked {gist_id} into your account"));
                            state.loading = true;
                            channels.gist = Some(spawn_gist_fetch());
                        }
                        Err(error) => state.set_status(format!("fork failed: {error}")),
                    },
                    BgTaskOutcome::RestoreRevisionDone {
                        result,
                        gist_id,
                        filename,
                    } => match result {
                        Ok(_) => {
                            state
                                .gist_content_cache
                                .remove(&(gist_id.clone(), filename.clone()));
                            state.set_status(format!(
                                "Restored {filename} from old revision (new revision created)"
                            ));
                            state.pending_action = None;
                            state.screen = Screen::Revisions;
                            state.revision.index = 0;
                            state.revision.entries = None;
                            state.loading = true;
                            channels.gist = Some(spawn_gist_fetch());
                            if let Some(gist_id) = state.revision.gist_id.clone() {
                                spawn_bg(
                                    state,
                                    &mut channels.bg,
                                    "Loading revisions…",
                                    move || {
                                        let result = crate::gh::fetch_gist_commits_json(&gist_id)
                                            .map_err(|e| e.to_string())
                                            .and_then(|raw| {
                                                crate::gh::parse_gist_commits_json(&raw)
                                                    .map_err(|e| e.to_string())
                                            });
                                        BgTaskOutcome::RevisionsFetched { gist_id, result }
                                    },
                                );
                            }
                        }
                        Err(error) => {
                            state.set_status(format!("restore failed: {error}"));
                            state.screen = Screen::Confirm;
                        }
                    },
                }
            } // is_current_bg_generation — stale outcomes are dropped without applying
        }
    }
    Ok(LoopFlow::Proceed)
}
