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

pub(super) fn fetch_gist_content(
    runner: &dyn crate::actions::CommandRunner,
    gist_id: &str,
    filename: &str,
    raw_url: Option<&str>,
) -> std::result::Result<String, String> {
    let content = crate::gh::fetch_gist_file_content(runner, gist_id, filename, raw_url)
        .map_err(|e| e.to_string())?;
    crate::domain::ensure_text_size(content.len() as u64)?;
    Ok(content)
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

/// Observable facts about one action job, separated from the closure that executes it
/// (issue #422). Dispatch tests can inspect this value without running `gh`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ActionJobSpec {
    pub(super) kind: ActionJobKind,
    pub(super) progress: String,
}

impl ActionJobSpec {
    pub(super) fn new(kind: ActionJobKind, progress: impl Into<String>) -> Self {
        Self {
            kind,
            progress: progress.into(),
        }
    }

    pub(super) fn gist_fetch(
        progress: impl Into<String>,
        file: crate::domain::GistFileRef,
    ) -> Self {
        Self::new(ActionJobKind::GistFetch(file), progress)
    }
}

/// Semantic action identity and the non-content payload dispatch wiring needs to expose.
/// Worker closures stay opaque; content and descriptions are deliberately not duplicated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ActionJobKind {
    GistFetch(crate::domain::GistFileRef),
    FetchComments {
        gist_id: String,
        page: Option<u32>,
    },
    AnalyzeCompact {
        gist_id: String,
    },
    Upload {
        file: crate::domain::GistFileRef,
    },
    Create {
        local_path: PathBuf,
        public: bool,
    },
    DeleteGist {
        gist_id: String,
    },
    RemoveFile {
        file: crate::domain::GistFileRef,
    },
    CompactGist {
        gist_id: String,
    },
    UpdateDescription {
        gist_id: String,
    },
    /// Every Gist revision job. Its semantic identity is owned by the workflow module
    /// (`src/tui/gist_revision.rs`, issue #430), not spelled out again here.
    Revision(super::gist_revision::RevisionJobKind),
    ToggleGistStar {
        gist_id: String,
        starring: bool,
    },
    ForkGist {
        gist_id: String,
    },
}

impl ActionJobKind {
    /// A change to a gist itself. Once its `gh` command has started it runs to completion —
    /// cutting a write short would leave GitHub in an unknown state — so it can't be cancelled
    /// and its result always applies (#478).
    fn is_gist_mutation(&self) -> bool {
        match self {
            Self::Upload { .. }
            | Self::Create { .. }
            | Self::DeleteGist { .. }
            | Self::RemoveFile { .. }
            | Self::CompactGist { .. }
            | Self::UpdateDescription { .. }
            | Self::ToggleGistStar { .. }
            | Self::ForkGist { .. }
            | Self::Revision(super::gist_revision::RevisionJobKind::ExecuteRestore { .. }) => true,
            Self::GistFetch(_)
            | Self::FetchComments { .. }
            | Self::AnalyzeCompact { .. }
            | Self::Revision(_) => false,
        }
    }
}

struct ActionJob {
    generation: u64,
    run: Box<dyn FnOnce() -> ActionApply + Send>,
}

/// Internal seam between deciding which action job to start and executing its closure.
trait ActionSpawner {
    fn spawn(&mut self, spec: ActionJobSpec, job: ActionJob) -> ActionRx;
}

struct ThreadActionSpawner;

impl ActionSpawner for ThreadActionSpawner {
    fn spawn(&mut self, _spec: ActionJobSpec, job: ActionJob) -> ActionRx {
        let ActionJob { generation, run } = job;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let apply = run();
            let _ = tx.send((generation, apply));
        });
        Some(rx)
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
pub(super) struct RecordedActionJobs(std::rc::Rc<std::cell::RefCell<Vec<ActionJobSpec>>>);

#[cfg(test)]
impl RecordedActionJobs {
    pub(super) fn take(&self) -> Vec<ActionJobSpec> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

#[cfg(test)]
struct RecordingActionSpawner {
    started: RecordedActionJobs,
}

#[cfg(test)]
impl ActionSpawner for RecordingActionSpawner {
    fn spawn(&mut self, spec: ActionJobSpec, _job: ActionJob) -> ActionRx {
        self.started.0.borrow_mut().push(spec);
        None
    }
}

/// Runs the real worker closure on the calling thread and queues its completion for the
/// normal [`Jobs::absorb`] path (issue #430). Lets a test drive a complete workflow —
/// stage, execute, absorb, apply — with the generation guard intact and no thread.
#[cfg(test)]
struct InlineActionSpawner;

#[cfg(test)]
impl ActionSpawner for InlineActionSpawner {
    fn spawn(&mut self, _spec: ActionJobSpec, job: ActionJob) -> ActionRx {
        let ActionJob { generation, run } = job;
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send((generation, run()));
        Some(rx)
    }
}

/// Initial newest-first comment load: probe the total, then fetch the newest page.
/// Thin IO boundary (network) — not unit-tested.
pub(super) fn load_initial_comments(
    runner: &dyn crate::actions::CommandRunner,
    gist_id: &str,
) -> Result<crate::tui::InitialComments, String> {
    let probe = crate::gh::fetch_gist_comments_probe(runner, gist_id).map_err(|e| e.to_string())?;
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
        runner,
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

/// Spawn the push (upload local → gist) flow for a pin: lands in the existing
/// upload `Screen::Confirm` diff.
pub(super) fn spawn_pin_push(
    state: &mut AppState,
    jobs: &mut Jobs,
    m: &crate::domain::PinnedMapping,
    entry: crate::tui::DeferredEntry,
) {
    let local_path = m.resolve_against(&state.cwd);
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
    let target = m.resolve_against(&state.cwd);
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
    spawn_pin_diff_inner(state, jobs, m, entry, None);
}

/// [`spawn_pin_diff`], then set `status` once the diff has opened (entering a screen clears
/// the status, so it cannot be set up front).
pub(super) fn spawn_pin_diff_then(
    state: &mut AppState,
    jobs: &mut Jobs,
    m: &crate::domain::PinnedMapping,
    entry: crate::tui::DeferredEntry,
    status: &'static str,
) {
    spawn_pin_diff_inner(state, jobs, m, entry, Some(status));
}

fn spawn_pin_diff_inner(
    state: &mut AppState,
    jobs: &mut Jobs,
    m: &crate::domain::PinnedMapping,
    entry: crate::tui::DeferredEntry,
    status: Option<&'static str>,
) {
    let local_abs = m.resolve_against(&state.cwd);
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
            let flow = screens::diff::on_preview_diff(
                state,
                entry,
                result,
                Some(local_abs),
                local_label,
                gist_label,
                target,
                false,
                gist_file,
                true,
            );
            if let Some(status) = status.filter(|_| state.screen.is_diff()) {
                state.set_status(status);
            }
            flow
        },
    );
}

/// If `pair` is a pinned pair, record `baseline` for it and project the result onto `AppState`.
///
/// The in-memory check comes **first and gates the file access entirely**: a download of a
/// file nobody pinned must not read the config, and so cannot report a config problem the
/// user did not provoke. Only a pair this session believes is pinned is worth the IO.
pub(super) fn record_pin_sync(
    state: &mut AppState,
    local_abs: &std::path::Path,
    gist_id: &str,
    filename: &str,
    baseline: &crate::sync_baseline::SyncBaseline,
    direction: Option<crate::domain::SyncDirection>,
) {
    let pair = crate::pins::PinKey::new(local_abs, gist_id, filename);
    if crate::pins::find_by_resolved_path(&state.pinned, &state.cwd, pair).is_none() {
        return;
    }
    let result = state
        .config_store
        .record_sync(&state.cwd, pair, baseline, direction);
    apply_pin_sync(state, result);
}

/// Absorb a `record_sync` result.
///
/// A failure is **appended** to whatever status the surrounding action already set — the
/// caller has usually just reported "Downloaded a.txt", and that matters more than this
/// does (issue #432; same rule as `refresh_locals`). Before #432 all three failure modes
/// were discarded and the user kept a silently stale sync badge.
///
/// `NotPinned` here means the stored config disagrees with what this session believes
/// (a hand edit between the two). Nothing was persisted, so nothing is projected and
/// nothing is said.
fn apply_pin_sync(
    state: &mut AppState,
    result: anyhow::Result<(
        crate::config_store::PinChange,
        crate::config_store::SyncRecord,
    )>,
) {
    match result {
        Ok((change, crate::config_store::SyncRecord::Recorded)) => apply_pin_change(state, change),
        Ok((_, crate::config_store::SyncRecord::NotPinned)) => {}
        Err(error) => append_status(state, format!("pin sync not recorded: {error}")),
    }
}

/// Project a completed persistence operation onto `AppState`. Both fields travel together
/// because "what was just read" is the correct value for both, even after a hand edit.
fn apply_pin_change(state: &mut AppState, change: crate::config_store::PinChange) {
    state.pinned = change.pinned;
    state.skip_dirs = change.skip_dirs;
    state.mark_pin_sync_cache_dirty();
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
    match scratch.create_file(filename, body) {
        Ok(path) => Some((scratch, path)),
        Err(e) => {
            state.set_status(format!("failed to write {context}: {e}"));
            None
        }
    }
}

pub(super) fn download(state: &mut AppState, mode: crate::actions::DownloadMode) {
    let target = state.download_target();
    let content = state.preview_remote().to_string();
    let pin = state
        .diff()
        .and_then(|d| match (&d.gist_id, &d.gist_filename) {
            (Some(g), Some(f)) => Some(crate::domain::GistFileRef::id_name(g, f)),
            _ => None,
        });
    match write_download(state, &target, &content, mode, pin.as_ref()) {
        Ok(()) => {
            // Skip past the download overwrite gate's Confirm (if any) and its parked Diff to
            // land on whatever was behind them.
            if state.screen.is_confirm() {
                state.leave();
            }
            if state.screen.is_diff() {
                state.leave();
            }
        }
        Err(()) => state.cancel_confirm_to_diff(),
    }
}

/// Download one gist file to `target`: write it as the Sync policy dictates, record the pin
/// baseline from the bytes actually written (when `pin` names the pair), report, and rescan
/// locals. Navigation stays with the caller. On failure the status already says why.
pub(super) fn write_download(
    state: &mut AppState,
    target: &std::path::Path,
    remote: &str,
    mode: crate::actions::DownloadMode,
    pin: Option<&crate::domain::GistFileRef>,
) -> std::result::Result<(), ()> {
    let written = match state
        .settings
        .sync_policy()
        .write_download(target, remote, mode)
    {
        Ok(written) => written,
        Err(error) => {
            state.set_status(format!("download failed: {error}"));
            return Err(());
        }
    };
    state.set_status(format!(
        "Downloaded {}",
        target
            .file_name()
            .unwrap_or(target.as_os_str())
            .to_string_lossy()
    ));
    if let Some(pin) = pin {
        record_pin_sync(
            state,
            target,
            &pin.gist_id,
            &pin.filename,
            &crate::sync_baseline::SyncBaseline::after_sync(written.as_bytes(), remote.as_bytes()),
            Some(crate::domain::SyncDirection::Download),
        );
    }
    refresh_locals(state, Some(target));
    Ok(())
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
    let result = state
        .config_store
        .save_preferences(state.settings.preferences());
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

/// One rendering of a pinned pair for the status line, so pin and unpin cannot drift apart
/// in how they name the same thing (issue #424). `display_path` abbreviates `$HOME`; the raw
/// `Display` would make the pin message disagree with the unpin message about one path.
fn pin_pair_label(local_path: &std::path::Path, filename: &str) -> String {
    format!(
        "{} <-> {}",
        crate::config::display_path(local_path),
        filename
    )
}

pub(super) fn pin_paths(
    state: &mut AppState,
    local_path: &std::path::Path,
    gist_id: &str,
    filename: &str,
) {
    let result = state
        .config_store
        .pin(crate::pins::PinKey::new(local_path, gist_id, filename));
    match result {
        Ok(change) => {
            apply_pin_change(state, change);
            state.set_status(format!("Pinned {}", pin_pair_label(local_path, filename)));
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
    let result = state
        .config_store
        .unpin(crate::pins::PinKey::new(local_path, gist_id, filename));
    apply_unpin(state, result, pin_pair_label(local_path, filename));
}

/// Absorb an unpin result. [`Unpinned::NotFound`] means the stored config no longer held
/// that pair, so the status must not claim one was removed (issue #424).
fn apply_unpin(
    state: &mut AppState,
    result: anyhow::Result<(
        crate::config_store::PinChange,
        crate::config_store::Unpinned,
    )>,
    label: String,
) {
    match result {
        Ok((change, outcome)) => {
            apply_pin_change(state, change);
            state.set_status(match outcome {
                crate::config_store::Unpinned::Removed => format!("Unpinned {label}"),
                crate::config_store::Unpinned::NotFound => format!("{label} is not pinned"),
            });
        }
        Err(error) => state.set_status(format!("unpin failed: {error}")),
    }
}

pub(super) fn unpin_at_pin_index(state: &mut AppState, idx: usize) {
    if idx >= state.pinned.len() {
        return;
    }
    // A row index is a filtered-view concept: resolve it into a `PinKey` here, so the
    // persistence interface never sees one (issue #432).
    let mapping = state.pinned[idx].clone();
    let label = pin_pair_label(&mapping.local_path, &mapping.gist_filename);
    let result = state.config_store.unpin(mapping.key());
    let ok = result.is_ok();
    apply_unpin(state, result, label);
    if ok {
        let len = state.visible_pin_indices().len();
        if let Some(pins) = state.pins_mut() {
            pins.cursor.clamp_len(len);
        }
        // No filesystem rescan: unpin never touches the filesystem, and ranking reads
        // `PinnedMapping` directly — a forced-flat rescan here used to make the local
        // list drift back to cwd-only even while recursive mode was active (issue #409).
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
    /// The redact buffer's directory for the live watch. Held here, not by the watch thread,
    /// so it is removed when the session ends and when the app quits with the editor still
    /// open (a detached thread never runs its drops).
    upload_edit_scratch: Option<crate::temp_dir::ScratchDir>,
    action: ActionRx,
    /// Whether the in-flight action may be cancelled with Esc: reads yes, gist mutations no
    /// (#478).
    action_cancellable: bool,
    action_spawner: Box<dyn ActionSpawner>,
    /// External-command boundary handed to worker closures that need one. Production
    /// injects [`SystemRunner`]; tests inject a scripted runner (issue #430). Consumed by
    /// the Gist revision and Gist mutation workflows; a few read-only jobs (compact
    /// analysis, comments) still reach [`SystemRunner`] directly.
    runner: SharedRunner,
}

/// A [`crate::actions::CommandRunner`] that can be shared with a background worker.
pub(super) type SharedRunner = std::sync::Arc<dyn crate::actions::CommandRunner + Send + Sync>;

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
        let runner: SharedRunner = std::sync::Arc::new(SystemRunner);
        Self::with_action_spawner(
            update,
            fetch_gists,
            catalog,
            Box::new(ThreadActionSpawner),
            runner.clone(),
            runner,
        )
    }

    fn with_action_spawner(
        update: Option<std::sync::mpsc::Receiver<crate::update_check::UpdateCheckOutcome>>,
        fetch_gists: bool,
        catalog: &crate::domain::GistCatalog,
        action_spawner: Box<dyn ActionSpawner>,
        runner: SharedRunner,
        refresh_runner: SharedRunner,
    ) -> Self {
        Self {
            update,
            gist_refresh: super::gist_refresh::GistRefresh::new(
                catalog,
                fetch_gists,
                refresh_runner,
            ),
            local: None,
            upload_edit_watch: None,
            upload_edit_scratch: None,
            action: None,
            action_cancellable: true,
            action_spawner,
            runner,
        }
    }

    /// The shared external-command boundary, cloned into a worker closure.
    pub(super) fn command_runner(&self) -> SharedRunner {
        self.runner.clone()
    }

    #[cfg(test)]
    pub(super) fn recording(catalog: &crate::domain::GistCatalog) -> (Self, RecordedActionJobs) {
        let started = RecordedActionJobs::default();
        let jobs = Self::with_action_spawner(
            None,
            false,
            catalog,
            Box::new(RecordingActionSpawner {
                started: started.clone(),
            }),
            crate::tui::test_support::no_runner(),
            crate::tui::test_support::no_runner(),
        );
        (jobs, started)
    }

    /// A registry that executes worker closures inline against `runner`, so a test can
    /// drive a complete workflow through spawn, absorb, and apply (issue #430).
    ///
    /// The catalog refresh an apply may start runs on its own background threads, which
    /// inline execution can't order against the scripted calls, so it gets a runner that
    /// never answers: it must not consume `runner`'s script or land in its call log (#511).
    #[cfg(test)]
    pub(super) fn inline(catalog: &crate::domain::GistCatalog, runner: SharedRunner) -> Self {
        Self::with_action_spawner(
            None,
            false,
            catalog,
            Box::new(InlineActionSpawner),
            runner,
            crate::tui::test_support::no_runner(),
        )
    }

    /// Run `run` on a background thread; apply `apply(value)` on the event-loop tick.
    /// Sets `bg_task_msg` and stamps the result with the current action-job generation
    /// (issue #221). `run` must not touch `AppState`; `apply` is the only place that does
    /// (issue #375).
    pub(super) fn spawn_action<T, R, A>(
        &mut self,
        state: &mut AppState,
        spec: ActionJobSpec,
        run: R,
        apply: A,
    ) where
        T: Send + 'static,
        R: FnOnce() -> T + Send + 'static,
        A: FnOnce(T, &mut AppState) -> LoopFlow + Send + 'static,
    {
        self.start_action(state, spec, run, apply);
    }

    fn start_action<T, R, A>(&mut self, state: &mut AppState, spec: ActionJobSpec, run: R, apply: A)
    where
        T: Send + 'static,
        R: FnOnce() -> T + Send + 'static,
        A: FnOnce(T, &mut AppState) -> LoopFlow + Send + 'static,
    {
        let generation = state.begin_bg_task();
        state.bg_task_msg = Some(spec.progress.clone());
        self.action_cancellable = !spec.kind.is_gist_mutation();
        let run = Box::new(move || {
            let value = run();
            let boxed: ActionApply = Box::new(move |state: &mut AppState| apply(value, state));
            boxed
        });
        self.action = self
            .action_spawner
            .spawn(spec, ActionJob { generation, run });
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
        let spec = ActionJobSpec::gist_fetch(msg, file.clone());
        let runner = self.command_runner();
        self.start_action(
            state,
            spec,
            move || {
                let result = fetch_gist_content(
                    runner.as_ref(),
                    &file.gist_id,
                    &file.filename,
                    file.raw_url.as_deref(),
                );
                (result, file)
            },
            move |(result, file), state| apply(result, file, state),
        );
    }

    /// Esc cancel: drop the action receiver and invalidate generation so a late completion
    /// cannot mutate state. A gist mutation is not cancellable (#478): it keeps running, its
    /// result still applies, and the status says so.
    pub(super) fn cancel_action(&mut self, state: &mut AppState) {
        if !self.action_cancellable {
            state.set_status("can't cancel — waiting for GitHub");
            return;
        }
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
        scratch: crate::temp_dir::ScratchDir,
    ) {
        self.upload_edit_watch = Some(rx);
        self.upload_edit_scratch = Some(scratch);
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
            super::gist_revision::dispatch(
                self,
                state,
                super::gist_revision::RevisionRequest::FetchHistory { gist_id },
            );
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
            self.upload_edit_scratch = None;
        }
    }

    /// Absorb a completed background per-action task (ignore stale generations — issue #221).
    /// Returns [`LoopFlow::SkipIteration`] when the outcome should abort the rest of this
    /// event-loop tick (stale revision fetch / no-op restore); otherwise `LoopFlow::Proceed`.
    ///
    /// A router shell: generation guard, then the apply closure the job carried (issue #375).
    pub(super) fn on_action_outcome(&mut self, state: &mut AppState) -> LoopFlow {
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

    // ---- unpin absorb ---------------------------------------------------

    fn change(pinned: Vec<crate::domain::PinnedMapping>) -> crate::config_store::PinChange {
        crate::config_store::PinChange {
            pinned,
            skip_dirs: vec!["node_modules".into()],
        }
    }

    #[test]
    fn apply_unpin_reports_the_pair_it_removed() {
        let mut state = crate::tui::initial_state();

        apply_unpin(
            &mut state,
            Ok((change(Vec::new()), crate::config_store::Unpinned::Removed)),
            "~/a.txt <-> a.txt".into(),
        );

        assert_eq!(state.status.as_deref(), Some("Unpinned ~/a.txt <-> a.txt"));
    }

    /// Both projected fields land, so a hand edit picked up by the load is not dropped
    /// on the floor (issue #432).
    #[test]
    fn apply_unpin_projects_both_config_fields() {
        let mut state = crate::tui::initial_state();
        let mapping = crate::domain::PinnedMapping::fixture("/b.txt", "g2", "b.txt");

        apply_unpin(
            &mut state,
            Ok((
                change(vec![mapping.clone()]),
                crate::config_store::Unpinned::Removed,
            )),
            "~/a.txt <-> a.txt".into(),
        );

        assert_eq!(state.pinned, vec![mapping]);
        assert_eq!(state.skip_dirs, vec!["node_modules".to_string()]);
    }

    /// The key is exact now, so a stored config that no longer holds the pair is reachable.
    /// Saying "Unpinned" there would be a lie (issue #424).
    #[test]
    fn apply_unpin_does_not_claim_a_removal_that_did_not_happen() {
        let mut state = crate::tui::initial_state();

        apply_unpin(
            &mut state,
            Ok((change(Vec::new()), crate::config_store::Unpinned::NotFound)),
            "~/a.txt <-> a.txt".into(),
        );

        assert_eq!(
            state.status.as_deref(),
            Some("~/a.txt <-> a.txt is not pinned")
        );
    }

    #[test]
    fn apply_unpin_surfaces_a_failure_without_touching_the_pins() {
        let mut state = crate::tui::initial_state();
        state.pinned = vec![crate::domain::PinnedMapping::fixture(
            "/a.txt", "g1", "a.txt",
        )];

        apply_unpin(
            &mut state,
            Err(anyhow::anyhow!("boom")),
            "~/a.txt <-> a.txt".into(),
        );

        assert_eq!(state.status.as_deref(), Some("unpin failed: boom"));
        assert_eq!(state.pinned.len(), 1);
    }

    /// A persistence failure must not erase the feedback the surrounding action already
    /// set. Before #432 all three failure modes were discarded entirely.
    #[test]
    fn apply_pin_sync_appends_a_failure_to_the_existing_status() {
        let mut state = crate::tui::initial_state();
        state.set_status("Downloaded a.txt");

        apply_pin_sync(&mut state, Err(anyhow::anyhow!("permission denied")));

        assert_eq!(
            state.status.as_deref(),
            Some("Downloaded a.txt; pin sync not recorded: permission denied")
        );
    }

    #[test]
    fn apply_pin_sync_reports_a_failure_on_its_own_when_nothing_was_said() {
        let mut state = crate::tui::initial_state();

        apply_pin_sync(&mut state, Err(anyhow::anyhow!("boom")));

        assert_eq!(state.status.as_deref(), Some("pin sync not recorded: boom"));
    }

    /// `NotPinned` persisted nothing, so it must project nothing and say nothing.
    #[test]
    fn apply_pin_sync_applies_nothing_when_the_pair_was_not_pinned() {
        let mut state = crate::tui::initial_state();
        state.set_status("Downloaded a.txt");
        let before = state.skip_dirs.clone();

        apply_pin_sync(
            &mut state,
            Ok((
                change(vec![crate::domain::PinnedMapping::fixture(
                    "/ignored.txt",
                    "g9",
                    "ignored.txt",
                )]),
                crate::config_store::SyncRecord::NotPinned,
            )),
        );

        assert_eq!(state.status.as_deref(), Some("Downloaded a.txt"));
        assert!(state.pinned.is_empty(), "nothing was persisted to project");
        assert_eq!(state.skip_dirs, before);
    }

    /// A pair this session does not believe is pinned must not reach the filesystem at
    /// all — otherwise a routine download of an unpinned file could report a config
    /// problem the user never provoked.
    #[test]
    fn record_pin_sync_on_an_unpinned_pair_touches_nothing() {
        let mut state = crate::tui::initial_state();
        state.cwd = PathBuf::from("/cwd");
        state.set_status("Downloaded a.txt");

        record_pin_sync(
            &mut state,
            std::path::Path::new("/cwd/a.txt"),
            "g1",
            "a.txt",
            &crate::sync_baseline::SyncBaseline::after_sync(b"body", b"body"),
            Some(crate::domain::SyncDirection::Download),
        );

        assert_eq!(state.status.as_deref(), Some("Downloaded a.txt"));
        assert!(state.pinned.is_empty());
    }

    /// `record_sync` used to project only `pinned`, unlike the pin and unpin paths.
    #[test]
    fn apply_pin_sync_projects_both_config_fields() {
        let mut state = crate::tui::initial_state();

        apply_pin_sync(
            &mut state,
            Ok((
                change(Vec::new()),
                crate::config_store::SyncRecord::Recorded,
            )),
        );

        assert_eq!(state.skip_dirs, vec!["node_modules".to_string()]);
    }

    // ---- write_scratch_file ---------------------------------------------

    /// The shared scratch preparation both upload and restore-revision depend on. A write
    /// failure keeps its own wording, cleans the directory up, and — because the caller
    /// early-returns on `None` — never reaches a spawn, so an unrelated in-flight job is
    /// neither started nor superseded (issue #430).
    #[test]
    fn write_scratch_file_reports_a_write_failure_and_leaves_nothing_behind() {
        let mut state = initial_state();
        let before = state.begin_bg_task();

        // A nested name cannot be written: the scratch directory has no subdirectories.
        let prepared = write_scratch_file(
            &mut state,
            "restore",
            "sub/restore.json",
            "restore payload",
            b"{}",
        );

        assert!(prepared.is_none());
        assert!(state
            .status
            .as_deref()
            .is_some_and(|s| s.starts_with("failed to write restore payload: ")));
        assert!(
            state.is_current_bg_generation(before),
            "a failed preparation must not supersede an in-flight job"
        );
    }

    #[test]
    fn write_scratch_file_hands_back_a_path_inside_a_live_scratch_dir() {
        let mut state = initial_state();

        let (scratch, path) = write_scratch_file(
            &mut state,
            "restore",
            "restore.json",
            "restore payload",
            b"{}",
        )
        .expect("prepared");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        assert_eq!(path.parent(), Some(scratch.path()));
        drop(scratch);
        assert!(!path.exists(), "the owner's drop cleans it up");
    }

    // ---- test helpers ---------------------------------------------------

    /// Empty `Jobs` registry — every slot `None`. Tests populate only the slot under test.
    fn empty_jobs() -> Jobs {
        Jobs {
            update: None,
            gist_refresh: GistRefresh::new(
                &GistCatalog::default(),
                false,
                crate::tui::test_support::no_runner(),
            ),
            local: None,
            upload_edit_watch: None,
            upload_edit_scratch: None,
            action: None,
            action_cancellable: true,
            action_spawner: Box::new(ThreadActionSpawner),
            runner: crate::tui::test_support::no_runner(),
        }
    }

    /// Issue #478: Esc cancels a read, but not a gist mutation — that one keeps running and
    /// its result still applies.
    #[test]
    fn cancel_action_cancels_reads_but_not_gist_mutations() {
        let catalog = GistCatalog::default();

        let mut state = initial_state();
        let (mut jobs, _started) = Jobs::recording(&catalog);
        jobs.spawn_action(
            &mut state,
            ActionJobSpec::new(
                ActionJobKind::DeleteGist {
                    gist_id: "g1".into(),
                },
                "Deleting gist…",
            ),
            || (),
            |(), _| LoopFlow::Proceed,
        );
        jobs.cancel_action(&mut state);
        assert_eq!(
            state.status.as_deref(),
            Some("can't cancel — waiting for GitHub")
        );
        assert!(state.bg_task_msg.is_some(), "still running");

        let mut state = initial_state();
        let (mut jobs, _started) = Jobs::recording(&catalog);
        jobs.spawn_action(
            &mut state,
            ActionJobSpec::new(
                ActionJobKind::AnalyzeCompact {
                    gist_id: "g1".into(),
                },
                "Counting revisions…",
            ),
            || (),
            |(), _| LoopFlow::Proceed,
        );
        jobs.cancel_action(&mut state);
        assert_eq!(state.status.as_deref(), Some("Cancelled"));
        assert!(state.bg_task_msg.is_none());
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
        state.enter_upload_confirm(
            crate::tui::UploadDraft {
                watching: true,
                ..crate::tui::UploadDraft::fixture("g1", "a.txt", "a.txt")
            },
            None,
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
        let scratch = crate::temp_dir::ScratchDir::create("redact-test").unwrap();
        let buffer_dir = scratch.path().to_path_buf();
        jobs.set_upload_edit_watch(rx, scratch);

        jobs.on_upload_watch_events(&mut state);

        assert!(jobs.upload_edit_watch.is_none());
        assert!(
            !buffer_dir.exists(),
            "the redact buffer goes when the session ends"
        );
        assert!(!state.upload_draft().unwrap().watching);
        assert_eq!(
            state.upload_draft().unwrap().edited_content.as_deref(),
            Some("two")
        );
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
