//! The **sync** workflow (issue #525): comparing, pulling, and pushing one Sync pair. List
//! and Pins both hand [`dispatch`] one plain-data [`SyncRequest`], built complete when the
//! user acted; nothing here reads the List selection or the Pins cursor afterwards.
//!
//! This module fetches the gist side, reads the local side, asks the Sync policy whether the
//! two are identical (confirming a pinned pair's baseline when they are), and opens the Diff,
//! the upload Confirm, or writes the download. The upload itself, once confirmed, is a Gist
//! mutation (`gist_mutation`), not this module's. Eligibility guards (what is selected, is
//! the pair pinned) stay with the keys that build the request.

use super::bg::{Jobs, LoopFlow};
use super::gist_content::GistContentStore;
use super::pin_sync::{confirm_sync_baseline, record_pin_sync};
use super::{AppState, DeferredEntry, UploadDraft};
use crate::domain::{SyncPair, SyncStatus};

/// One sync of one pair, as plain data captured when the user acted.
#[derive(Debug, PartialEq, Eq)]
pub struct SyncRequest {
    /// Where the screen this opens returns to.
    pub entry: DeferredEntry,
    pub pair: SyncPair,
    pub intent: SyncIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncIntent {
    /// Open the pair's Diff, framed local → gist when `upload_orientation`.
    Compare { upload_orientation: bool },
    /// Download the gist file: over an existing local file through its Diff, else directly.
    Pull,
    /// Upload the local file: preview a replace of the gist file, or add it when the gist
    /// lacks it. Decided when the user acted, so a later catalog refresh can't flip it (#523).
    Push { replaces: bool },
    /// Whichever of the above the pin's sync status calls for (the pair must be pinned).
    Auto,
}

impl AppState {
    /// A sync of `pair` that opens its screen over the current one.
    pub(crate) fn sync_request(&self, pair: SyncPair, intent: SyncIntent) -> super::KeyOutcome {
        super::KeyOutcome::Sync(SyncRequest {
            entry: self.defer_entry(),
            pair,
            intent,
        })
    }
}

/// Status when the two sides of an `Auto` sync both changed; the Diff it opens lets the
/// user pick a side.
const CONFLICT: &str = "both sides changed — press d or u";

pub(super) fn dispatch(jobs: &mut Jobs, state: &mut AppState, request: SyncRequest) {
    let SyncRequest {
        entry,
        pair,
        intent,
    } = request;
    match intent {
        SyncIntent::Compare { upload_orientation } => {
            compare(jobs, state, entry, pair, upload_orientation, None)
        }
        SyncIntent::Pull => pull(jobs, state, entry, pair),
        SyncIntent::Push { replaces: true } => push_replace(jobs, state, entry, pair),
        SyncIntent::Push { replaces: false } => push_add(state, entry, pair),
        SyncIntent::Auto => auto(jobs, state, entry, pair),
    }
}

/// `Auto`: resolve the pin, then compare, pull, or push as its status says.
fn auto(jobs: &mut Jobs, state: &mut AppState, entry: DeferredEntry, pair: SyncPair) {
    let key = crate::pins::PinKey::new(&pair.local, &pair.gist.gist_id, &pair.gist.filename);
    let Some(index) = crate::pins::position(&state.pinned, &state.cwd, key) else {
        // The key checks this; a pin removed since then lands here.
        state.set_status("pair is not pinned — press p to pin first");
        return;
    };
    match state.compute_pin_sync_status(index) {
        SyncStatus::Push => {
            let replaces = gist_has_file(state, &pair);
            dispatch(
                jobs,
                state,
                SyncRequest {
                    entry,
                    pair,
                    intent: SyncIntent::Push { replaces },
                },
            );
        }
        SyncStatus::Pull => pull(jobs, state, entry, pair),
        SyncStatus::InSync => state.set_status("already in sync"),
        SyncStatus::Missing => state.set_status("local file is missing — use d to pull it back"),
        SyncStatus::Unknown => {
            state.set_status("can't tell which side is newer — use u to push or d to pull")
        }
        // Both sides changed: show the diff and let the user pick d or u (issue #466).
        SyncStatus::Conflict => compare(jobs, state, entry, pair, false, Some(CONFLICT)),
    }
}

/// Whether the gist already holds the pair's gist file: an upload replaces it, else adds it.
pub(super) fn gist_has_file(state: &AppState, pair: &SyncPair) -> bool {
    state
        .gist_catalog
        .owned
        .iter()
        .any(|g| g.gist_id == pair.gist.gist_id && g.filename == pair.gist.filename)
}

/// The diff header labels for `pair`, with the local side named only when `local_shown`.
fn labels(state: &AppState, pair: &SyncPair, local_shown: bool) -> (String, String) {
    let gist = state.gist_file_for_diff(&pair.gist);
    crate::tui::render::diff_labels(local_shown.then_some(pair.local.as_path()), &gist)
}

/// What a fetch's apply needs besides the fetched content.
struct Fetched {
    entry: DeferredEntry,
    pair: SyncPair,
    local_label: String,
    gist_label: String,
}

/// Fetch the pair's gist file, then hand it to `apply`.
fn fetch(
    jobs: &mut Jobs,
    state: &mut AppState,
    progress: &'static str,
    fetched: Fetched,
    apply: impl FnOnce(&mut AppState, Fetched, Result<String, String>) -> LoopFlow + Send + 'static,
) {
    let file = GistContentStore::fetch_target(&state.gist_catalog, fetched.pair.gist.clone());
    jobs.spawn_gist_fetch_action(state, progress, file, move |result, _file, state| {
        apply(state, fetched, result)
    });
}

fn compare(
    jobs: &mut Jobs,
    state: &mut AppState,
    entry: DeferredEntry,
    pair: SyncPair,
    upload_orientation: bool,
    status: Option<&'static str>,
) {
    // The Diff shows what its `d` / `u` write (#524): the local file, when there is one.
    let (local_label, gist_label) = labels(state, &pair, pair.local.exists());
    let fetched = Fetched {
        entry,
        pair,
        local_label,
        gist_label,
    };
    fetch(
        jobs,
        state,
        "Loading diff…",
        fetched,
        move |state, fetched, result| {
            let flow = on_compare(state, fetched, result, upload_orientation);
            // Entering the Diff clears the status, so it can only be set once it has opened.
            if let Some(status) = status.filter(|_| state.screen.is_diff()) {
                state.set_status(status);
            }
            flow
        },
    );
}

fn pull(jobs: &mut Jobs, state: &mut AppState, entry: DeferredEntry, pair: SyncPair) {
    let (local_label, gist_label) = labels(state, &pair, true);
    let fetched = Fetched {
        entry,
        pair,
        local_label,
        gist_label,
    };
    fetch(jobs, state, "Downloading…", fetched, on_pull);
}

fn push_replace(jobs: &mut Jobs, state: &mut AppState, entry: DeferredEntry, pair: SyncPair) {
    let (local_label, gist_label) = labels(state, &pair, true);
    let fetched = Fetched {
        entry,
        pair,
        local_label,
        gist_label,
    };
    fetch(jobs, state, "Loading diff…", fetched, on_push);
}

/// A push of a file new to the gist: nothing to fetch, so Confirm opens right away against an
/// empty gist side.
fn push_add(state: &mut AppState, entry: DeferredEntry, pair: SyncPair) {
    let local_label = format!("local: {}", crate::config::display_path(&pair.local));
    match UploadDraft::read(
        pair.gist.gist_id,
        pair.gist.filename,
        pair.local.clone(),
        String::new(),
        local_label,
        "(new file)".to_string(),
        false,
    ) {
        Ok(draft) => state.enter_upload_confirm(draft, Some(entry)),
        Err(error) => cannot_read(state, &pair.local, error),
    }
}

fn cannot_read(state: &mut AppState, path: &std::path::Path, error: impl std::fmt::Display) {
    state.set_status(format!(
        "cannot read {}: {error}",
        crate::config::display_path(path)
    ));
}

/// `Compare` apply: diff the local file (empty when there is none yet) against the gist.
fn on_compare(
    state: &mut AppState,
    fetched: Fetched,
    result: Result<String, String>,
    upload_orientation: bool,
) -> LoopFlow {
    match result {
        Ok(remote) => {
            let local = if fetched.pair.local.exists() {
                crate::domain::read_text_file_capped(&fetched.pair.local)
            } else {
                Ok(String::new())
            };
            match local {
                Ok(local) => open_diff(state, fetched, local, remote, upload_orientation),
                Err(error) => state.set_status(format!("read failed: {error}")),
            }
        }
        Err(error) => state.set_status(format!("fetch failed: {error}")),
    }
    LoopFlow::Proceed
}

/// `Pull` apply: diff against an existing local file (its `d` overwrites it), or write a
/// new one.
fn on_pull(state: &mut AppState, fetched: Fetched, result: Result<String, String>) -> LoopFlow {
    match result {
        Ok(remote) if fetched.pair.local.exists() => {
            match crate::domain::read_text_file_capped(&fetched.pair.local) {
                Ok(local) => open_diff(state, fetched, local, remote, false),
                Err(error) => state.set_status(error),
            }
        }
        Ok(remote) => {
            let _ = write_download(
                state,
                &fetched.pair.local,
                &remote,
                crate::actions::DownloadMode::CreateNew,
                Some(&fetched.pair.gist),
            );
        }
        Err(error) => state.set_status(format!("fetch failed: {error}")),
    }
    LoopFlow::Proceed
}

/// `Push` (replace) apply: open the upload Confirm against the gist file — unless the two
/// sides are already identical under the Sync policy, when there is nothing to send: stay
/// put and confirm the pair's baseline (#493).
fn on_push(state: &mut AppState, fetched: Fetched, result: Result<String, String>) -> LoopFlow {
    let Fetched {
        entry,
        pair,
        local_label,
        gist_label,
    } = fetched;
    match result {
        Ok(remote) => match UploadDraft::read(
            pair.gist.gist_id.clone(),
            pair.gist.filename.clone(),
            pair.local.clone(),
            remote,
            local_label,
            gist_label,
            true,
        ) {
            Ok(draft)
                if state
                    .settings
                    .sync_policy()
                    .identical(&draft.original_content, &draft.remote_content) =>
            {
                state.set_status("already in sync — nothing to upload");
                confirm_sync_baseline(
                    state,
                    &pair.local,
                    &pair.gist,
                    &draft.original_content,
                    &draft.remote_content,
                );
            }
            Ok(draft) => state.enter_upload_confirm(draft, Some(entry)),
            Err(error) => cannot_read(state, &pair.local, error),
        },
        Err(error) => state.set_status(format!("fetch failed: {error}")),
    }
    LoopFlow::Proceed
}

/// Open the Diff of a pair whose two sides are in hand. An identical pair is in sync: see
/// [`confirm_sync_baseline`].
fn open_diff(
    state: &mut AppState,
    fetched: Fetched,
    local: String,
    remote: String,
    upload_orientation: bool,
) {
    let policy = state.settings.sync_policy();
    let text = policy.preview_diff(
        upload_orientation,
        &fetched.local_label,
        &local,
        &fetched.gist_label,
        &remote,
    );
    let identical = policy.identical(&local, &remote);
    let pair = fetched.pair.clone();
    state.open_deferred(
        fetched.entry,
        crate::tui::Screen::Diff(Box::new(crate::tui::DiffState {
            body: crate::tui::ScrollBody {
                text,
                ..crate::tui::ScrollBody::default()
            },
            merge: Some(crate::merge::Merge::new(
                fetched.pair.local.exists().then_some(local.clone()),
                remote.clone(),
                policy,
            )),
            merge_dimensions: None,
            identical,
            kind: crate::tui::DiffKind::Sync { pair: fetched.pair },
        })),
    );
    // After entering the Diff, which clears the status a failed record appends to.
    if identical {
        confirm_sync_baseline(state, &pair.local, &pair.gist, &local, &remote);
    }
}

// ---- settling what a pin believes after a sync (issue #526) -----------------------------

/// Sync's one follow-up to a successful push, called by
/// [`gist_mutation::on_upload_replace`](super::gist_mutation::on_upload_replace). It records
/// the pair's Sync baseline from the local file's bytes on disk (not the possibly
/// redacted/transformed bytes sent — #465), patches the uploaded file's blob sha into the
/// in-memory catalog so the pin reads in sync before the refresh this upload triggers lands
/// (#466), marks the pin-sync cache dirty through the existing projection
/// ([`apply_pin_change`]), and leaves Confirm and any stale Diff — the same landing rule a
/// download uses (#520). Content-store invalidation is the caller's: it applies to every
/// file mutation, not just a push.
pub(super) fn on_push_done(
    state: &mut AppState,
    file: &crate::domain::GistFileRef,
    local_path: &std::path::Path,
    local_content: &str,
    sent_content: &str,
) {
    let baseline = crate::sync_baseline::SyncBaseline::after_sync(
        local_content.as_bytes(),
        sent_content.as_bytes(),
    );
    patch_catalog_blob_sha(state, file, baseline.remote_blob_sha.as_deref());
    record_pin_sync(
        state,
        local_path,
        &file.gist_id,
        &file.filename,
        &baseline,
        Some(crate::domain::SyncDirection::Upload),
    );
    land_after_confirmed_sync(state);
}

/// Patch `file`'s blob sha into the in-memory catalog's `raw_url`, when both the catalog
/// holds a raw URL to patch and a sha was recorded.
pub(super) fn patch_catalog_blob_sha(
    state: &mut AppState,
    file: &crate::domain::GistFileRef,
    sha: Option<&str>,
) {
    let Some(sha) = sha else { return };
    for g in state.gist_catalog.owned.iter_mut() {
        if g.gist_id == file.gist_id && g.filename == file.filename {
            if let Some(url) = g
                .raw_url
                .as_deref()
                .and_then(|u| crate::domain::raw_url_with_blob_sha(u, sha))
            {
                g.raw_url = Some(url);
            }
        }
    }
}

// ---- the sync write path: download, landing, local rescan (moved from bg.rs) -----------

/// `d` in a sync Diff: gate an overwrite of an existing `target` behind Confirm (its `y`
/// comes back as [`download`] with the overwrite token), or write a new file at once. The
/// check runs now, when the user acted, not when the Diff was opened.
pub(super) fn request_download(state: &mut AppState, target: &std::path::Path) {
    if target.exists() {
        state.enter_confirm_from_diff(super::PendingAction::Download);
    } else {
        download(state, crate::actions::DownloadMode::CreateNew);
    }
}

pub(super) fn download(state: &mut AppState, mode: crate::actions::DownloadMode) {
    let Some((pair, content)) = state.diff().and_then(|d| match &d.kind {
        crate::tui::DiffKind::Sync { pair } => Some((pair.clone(), d.saved_gist()?.to_string())),
        crate::tui::DiffKind::Revision => None,
    }) else {
        return;
    };
    match write_download(state, &pair.local, &content, mode, Some(&pair.gist)) {
        Ok(()) => land_after_confirmed_sync(state),
        Err(()) => state.cancel_confirm_to_diff(),
    }
}

/// Land off Confirm and any stale Diff — the rule a successful download and a successful
/// push (`sync::on_push_done`) both use (issue #520): the write just made whatever comparison
/// was on screen stale, so skip past the download overwrite gate's Confirm (if any) and a
/// parked Diff to land on whatever was behind them.
pub(super) fn land_after_confirmed_sync(state: &mut AppState) {
    if state.screen.is_confirm() {
        state.leave();
    }
    if state.screen.is_diff() {
        state.leave();
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
        super::pin_sync::record_pin_sync(
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
    let request = state.local_scan_request(super::local_scan::ScanMode::from_active(
        state.local_recursive,
    ));
    let generation = state.begin_local_scan();
    match request.run() {
        Ok(candidates) => {
            state.apply_local_scan(generation, candidates, target);
        }
        Err(error) => {
            state.end_local_scan(generation);
            state.append_status(format!("local refresh failed: {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_support::SeqRunner;
    use crate::actions::CommandOutput;
    use crate::domain::{GistFile, GistFileRef, LocalCandidate, PinnedMapping};
    use crate::sync_baseline::SyncBaseline;
    use crate::tui::test_support::{idle_jobs, recording_jobs, state_with_stored_pin};
    use crate::tui::{initial_state, KeyOutcome, PendingAction, Screen};
    use crossterm::event::KeyCode;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    /// A runner whose one `gh api gists/g1` call returns `content` as gist `g1`'s `filename`.
    fn gist_serving(filename: &str, content: &str) -> Arc<SeqRunner> {
        let body = serde_json::json!({ "files": { filename: { "content": content } } });
        Arc::new(SeqRunner::new(vec![CommandOutput::ok(body.to_string())]))
    }

    /// Drive one request through the workflow: dispatch, run the fetch inline against
    /// `runner`, then apply. Only the action outcome is drained — `Jobs::absorb` would start
    /// a real gist-list refresh.
    fn run(state: &mut AppState, runner: &Arc<SeqRunner>, request: SyncRequest) {
        let mut jobs = Jobs::inline(&state.gist_catalog.clone(), runner.clone());
        dispatch(&mut jobs, state, request);
        jobs.on_action_outcome(state);
    }

    fn request(state: &AppState, pair: SyncPair, intent: SyncIntent) -> SyncRequest {
        SyncRequest {
            entry: state.defer_entry(),
            pair,
            intent,
        }
    }

    fn pair(local: &Path, filename: &str) -> SyncPair {
        SyncPair {
            local: local.to_path_buf(),
            gist: GistFileRef::id_name("g1", filename),
        }
    }

    /// `dir/a.txt` holding `local`, pinned to `g1:a.txt` (in memory and in `dir`'s config)
    /// when `pinned`.
    fn state_with_local(dir: &Path, local: &str, pinned: bool) -> (AppState, PathBuf) {
        let path = dir.join("a.txt");
        std::fs::write(&path, local).unwrap();
        let mut state = if pinned {
            state_with_stored_pin(dir, PinnedMapping::fixture(path.clone(), "g1", "a.txt"))
        } else {
            let mut state = initial_state();
            state.cwd = dir.to_path_buf();
            state
        };
        state.gist_catalog.owned = vec![GistFile::fixture("g1", "a.txt")];
        (state, path)
    }

    /// Compare, pull, and push of an existing local file, identical or not, pinned or not.
    /// Compare and pull open the pair's Diff; push opens the upload Confirm unless there is
    /// nothing to send. Wherever the sides turn out identical, a pinned pair's baseline is
    /// confirmed from the content in hand (#466, #492, #493); an unpinned one touches no
    /// config.
    #[test]
    fn each_intent_opens_its_screen_and_confirms_an_identical_pinned_pair() {
        for intent in [
            SyncIntent::Compare {
                upload_orientation: false,
            },
            SyncIntent::Pull,
            SyncIntent::Push { replaces: true },
        ] {
            for identical in [true, false] {
                for pinned in [true, false] {
                    let label = format!("{intent:?} identical={identical} pinned={pinned}");
                    let dir = tempfile::tempdir().unwrap();
                    let (mut state, path) = state_with_local(dir.path(), "a", pinned);
                    let remote = if identical { "a\n" } else { "b\n" };
                    let push = matches!(intent, SyncIntent::Push { .. });
                    let request = request(&state, pair(&path, "a.txt"), intent.clone());

                    run(&mut state, &gist_serving("a.txt", remote), request);

                    if push && identical {
                        assert_eq!(state.screen, Screen::List, "{label}");
                        assert_eq!(
                            state.status.as_deref(),
                            Some("already in sync — nothing to upload"),
                            "{label}"
                        );
                    } else if push {
                        assert!(
                            matches!(state.pending_action(), Some(PendingAction::Upload(d)) if d.replaces),
                            "{label}: {:?}",
                            state.screen
                        );
                    } else {
                        assert_eq!(state.diff_identical(), identical, "{label}");
                        assert_eq!(state.sync_pair(), Some(&pair(&path, "a.txt")), "{label}");
                    }
                    let recorded = pinned && identical;
                    assert_eq!(
                        state.pinned.first().map(|p| p.baseline.clone()),
                        pinned.then(|| if recorded {
                            SyncBaseline::after_sync(b"a", remote.as_bytes())
                        } else {
                            SyncBaseline::default()
                        }),
                        "{label}"
                    );
                }
            }
        }
    }

    /// A pull with no local file yet writes it straight away and records the pin's baseline
    /// from the bytes written.
    #[test]
    fn a_pull_without_a_local_file_writes_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        let mut state = state_with_stored_pin(
            dir.path(),
            PinnedMapping::fixture(path.clone(), "g1", "a.txt"),
        );
        let request = request(&state, pair(&path, "a.txt"), SyncIntent::Pull);

        run(&mut state, &gist_serving("a.txt", "new\n"), request);

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(
            state.pinned[0].baseline,
            SyncBaseline::after_sync(b"new\n", b"new\n")
        );
    }

    /// Issue #523: a push of a file new to the gist opens Confirm at once against an empty
    /// gist side, recorded as an add so a later refresh can't turn it into a replace.
    #[test]
    fn a_push_of_a_file_new_to_the_gist_opens_confirm_as_an_add() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, path) = state_with_local(dir.path(), "a\n", false);
        let runner = Arc::new(SeqRunner::new(vec![]));
        let request = request(
            &state,
            pair(&path, "new.txt"),
            SyncIntent::Push { replaces: false },
        );

        run(&mut state, &runner, request);

        assert!(runner.calls().is_empty());
        let draft = state.upload_draft().expect("upload Confirm");
        assert!(!draft.replaces);
        assert_eq!(draft.filename, "new.txt");
        assert_eq!(draft.gist_label, "(new file)");
    }

    #[test]
    fn a_failed_fetch_reports_and_stays_put() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, path) = state_with_local(dir.path(), "a", false);
        let runner = Arc::new(SeqRunner::new(vec![CommandOutput::err("HTTP 502")]));
        let request = request(&state, pair(&path, "a.txt"), SyncIntent::Pull);

        run(&mut state, &runner, request);

        assert_eq!(state.screen, Screen::List);
        assert!(
            state
                .status
                .as_deref()
                .is_some_and(|s| s.starts_with("fetch failed:") && s.contains("HTTP 502")),
            "{:?}",
            state.status
        );
    }

    /// Issue #465: a CRLF gist against an LF local file is identical (nothing to sync) with
    /// normalization on; with it off the difference is real, so `d` / `u` stay available.
    #[test]
    fn a_line_ending_only_difference_follows_the_setting() {
        for normalize in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let (mut state, path) = state_with_local(dir.path(), "a\nb\n", false);
            if !normalize {
                state
                    .settings
                    .adjust(crate::tui::ConfigField::NormalizeLineEndings, true);
            }
            let request = request(
                &state,
                pair(&path, "a.txt"),
                SyncIntent::Compare {
                    upload_orientation: false,
                },
            );

            run(&mut state, &gist_serving("a.txt", "a\r\nb\r\n"), request);

            let diff = state.diff().expect("expected Screen::Diff");
            assert_eq!(diff.identical, normalize, "normalize={normalize}");
            assert_eq!(
                diff.body.text.contains("line endings differ"),
                !normalize,
                "normalize={normalize}"
            );
        }
    }

    // ---- Auto -----------------------------------------------------------

    /// A pinned `/cwd/a.txt` ↔ `g1:a.txt` pair. `local_mtime` and `remote_updated_at` are the
    /// only inputs `compute_pin_sync_status` reads for a hash-less mapping, so varying them
    /// alone walks `Auto` through its arms that start no fetch.
    fn state_with_one_pin(
        cwd: PathBuf,
        local_mtime: Option<u64>,
        remote_updated_at: Option<&str>,
    ) -> AppState {
        let mut state = initial_state();
        state.cwd = cwd;
        state.pinned = vec![PinnedMapping::fixture("a.txt", "g1", "a.txt")];
        if let Some(modified) = local_mtime {
            state.locals = vec![LocalCandidate {
                path: PathBuf::from("a.txt"),
                modified: Some(modified),
            }];
        }
        if let Some(updated_at) = remote_updated_at {
            state.gist_catalog.owned = vec![GistFile {
                updated_at: updated_at.into(),
                ..GistFile::fixture("g1", "a.txt")
            }];
        }
        state
    }

    fn auto(state: &mut AppState) {
        let pair = pair(&state.cwd.join("a.txt"), "a.txt");
        let request = request(state, pair, SyncIntent::Auto);
        dispatch(&mut idle_jobs(), state, request);
    }

    #[test]
    fn auto_reports_the_statuses_that_need_no_fetch() {
        let updated_at = "2026-06-10T00:00:00Z";
        let ts = crate::domain::parse_rfc3339_to_unix(updated_at).unwrap();
        // `pin_mtimes` falls back to stat-ing the path when `locals` has no match, so a
        // missing file needs a really empty directory.
        let empty = tempfile::tempdir().unwrap();
        let cases = [
            (
                state_with_one_pin(PathBuf::from("/cwd"), Some(ts), Some(updated_at)),
                "already in sync",
            ),
            (
                state_with_one_pin(empty.path().to_path_buf(), None, Some(updated_at)),
                "local file is missing — use d to pull it back",
            ),
            (
                state_with_one_pin(PathBuf::from("/cwd"), Some(1_780_000_000), None),
                "can't tell which side is newer — use u to push or d to pull",
            ),
        ];
        for (mut state, expected) in cases {
            auto(&mut state);
            assert_eq!(state.status.as_deref(), Some(expected));
            assert!(state.bg_task_msg.is_none(), "{expected}: must not fetch");
        }
    }

    #[test]
    fn auto_of_a_pair_no_longer_pinned_says_so() {
        let mut state = state_with_one_pin(PathBuf::from("/cwd"), None, None);
        state.pinned.clear();

        auto(&mut state);

        assert_eq!(
            state.status.as_deref(),
            Some("pair is not pinned — press p to pin first")
        );
    }

    /// Issue #466: Auto on a Conflict opens the pair's Diff instead of picking a side, and
    /// says why once it has opened.
    #[test]
    fn auto_on_a_conflict_opens_the_diff() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("a.txt");
        std::fs::write(&local, "edited").unwrap();
        let mut state = initial_state();
        state.cwd = dir.path().to_path_buf();
        state.pinned = vec![PinnedMapping {
            baseline: SyncBaseline {
                local_sha256: Some(crate::domain::sha256_hex(b"synced")),
                remote_blob_sha: Some("1111111111111111111111111111111111111111".into()),
            },
            ..PinnedMapping::fixture(local.clone(), "g1", "a.txt")
        }];
        state.gist_catalog.owned = vec![GistFile {
            raw_url: Some(
                "https://gist.githubusercontent.com/u/g1/raw/2222222222222222222222222222222222222222/a.txt"
                    .into(),
            ),
            ..GistFile::fixture("g1", "a.txt")
        }];
        assert_eq!(state.compute_pin_sync_status(0), SyncStatus::Conflict);
        let (mut jobs, started) = recording_jobs();
        let first = request(&state, pair(&local, "a.txt"), SyncIntent::Auto);

        dispatch(&mut jobs, &mut state, first);
        let started = started.take();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].progress, "Loading diff…");

        let second = request(&state, pair(&local, "a.txt"), SyncIntent::Auto);
        run(&mut state, &gist_serving("a.txt", "theirs"), second);
        assert!(state.screen.is_diff());
        assert_eq!(state.status.as_deref(), Some(CONFLICT));
    }

    /// Issue #493: Auto on a pin that reads Push, whose sides turn out identical once fetched,
    /// opens no upload Confirm; the pin reads in sync.
    #[test]
    fn auto_of_an_identical_push_confirms_the_pin_without_uploading() {
        let dir = tempfile::tempdir().unwrap();
        let local_path = dir.path().join("a.txt");
        std::fs::write(&local_path, "a\n").unwrap();
        let synced = SyncBaseline::after_sync(b"a", b"a");
        let mapping = PinnedMapping {
            baseline: synced.clone(),
            ..PinnedMapping::fixture(local_path.clone(), "g1", "a.txt")
        };
        let mut state = state_with_stored_pin(dir.path(), mapping);
        state.gist_catalog.owned = vec![GistFile {
            raw_url: Some(format!(
                "https://gist.githubusercontent.com/u/g1/raw/{}/a.txt",
                synced.remote_blob_sha.as_deref().unwrap()
            )),
            ..GistFile::fixture("g1", "a.txt")
        }];
        assert_eq!(state.compute_pin_sync_status(0), SyncStatus::Push);
        state.enter(Screen::Pins(Box::default()));
        let runner = gist_serving("a.txt", "a");
        let request = request(&state, pair(&local_path, "a.txt"), SyncIntent::Auto);

        run(&mut state, &runner, request);

        assert_eq!(runner.calls(), vec![crate::gh::gist_get_plan("g1")]);
        assert!(state.screen.is_pins(), "no Confirm: {:?}", state.screen);
        assert_eq!(
            state.status.as_deref(),
            Some("already in sync — nothing to upload")
        );
        assert_eq!(state.compute_pin_sync_status(0), SyncStatus::InSync);
    }

    // ---- from the List, end to end --------------------------------------

    /// The List's key builds the request; the workflow runs it.
    fn press(state: &mut AppState, runner: &Arc<SeqRunner>, code: KeyCode) {
        let KeyOutcome::Sync(request) = state.handle_key(code) else {
            panic!("expected a sync request");
        };
        run(state, runner, request);
    }

    /// Issue #524: the List Diff of local `a.txt` against gist `b.txt` shows those two files,
    /// and `u` / `d` from it write exactly them.
    #[test]
    fn a_list_diff_writes_the_two_files_it_compares() {
        let dir = tempfile::tempdir().unwrap();
        let local_path = dir.path().join("a.txt");
        std::fs::write(&local_path, "local\n").unwrap();
        let mut state = initial_state();
        state.cwd = dir.path().to_path_buf();
        state.locals = vec![LocalCandidate {
            path: local_path.clone(),
            modified: None,
        }];
        state.gist_catalog.owned = vec![GistFile::fixture("g1", "b.txt")];
        state.focus = crate::tui::FocusPane::Gist;

        press(
            &mut state,
            &gist_serving("b.txt", "remote\n"),
            KeyCode::Enter,
        );

        assert!(state.screen.is_diff(), "on {:?}", state.screen);
        let KeyOutcome::Sync(upload) = state.handle_key(KeyCode::Char('u')) else {
            panic!("expected an upload");
        };
        assert_eq!(upload.pair, pair(&local_path, "b.txt"));
        assert_eq!(upload.intent, SyncIntent::Push { replaces: true });
        assert_eq!(
            state.handle_key(KeyCode::Char('d')),
            KeyOutcome::DownloadRequested { target: local_path }
        );
    }

    /// Issue #524: with no local file selected, the Diff is against the file its `d` would
    /// overwrite when one is already there — not against nothing.
    #[test]
    fn a_list_diff_without_a_local_selection_shows_the_file_d_would_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.txt"), "remote\n").unwrap();
        let mut state = initial_state();
        state.cwd = dir.path().to_path_buf();
        state.gist_catalog.owned = vec![GistFile::fixture("g1", "b.txt")];
        state.focus = crate::tui::FocusPane::Gist;

        press(
            &mut state,
            &gist_serving("b.txt", "remote\n"),
            KeyCode::Enter,
        );

        assert!(state.screen.is_diff(), "on {:?}", state.screen);
        assert!(state.diff_identical(), "the Diff read the file on disk");
    }

    /// Issue #494: `d` in a Diff opened from the List still records a pinned pair's
    /// baseline — the Diff carries its Sync pair whichever screen opened it.
    #[test]
    fn a_download_from_a_list_diff_records_the_pins_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, path) = state_with_local(dir.path(), "old\n", true);
        let request = request(
            &state,
            pair(&path, "a.txt"),
            SyncIntent::Compare {
                upload_orientation: false,
            },
        );
        run(&mut state, &gist_serving("a.txt", "new\n"), request);

        download(
            &mut state,
            crate::actions::DownloadMode::overwrite_after_user_confirm(),
        );

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(
            state.pinned[0].baseline,
            SyncBaseline::after_sync(b"new\n", b"new\n")
        );
    }

    /// Issue #526: once an upload succeeds, `on_push_done` is the one place that settles
    /// what the pin believes afterward — the baseline is the local file's bytes on disk (not
    /// the possibly redacted/transformed bytes sent, #465), the catalog's blob sha is patched
    /// so the pin reads in sync before the refresh lands (#466), and the Confirm the upload
    /// was run from (plus any stale Diff behind it) is left, as a download would leave it
    /// (#520).
    #[test]
    fn on_push_done_settles_the_pins_baseline_catalog_and_landing() {
        let dir = tempfile::tempdir().unwrap();
        let local_path = dir.path().join("a.txt");
        std::fs::write(&local_path, "hello").unwrap();
        let mapping = PinnedMapping::fixture(local_path.clone(), "g1", "a.txt");
        let mut state = state_with_stored_pin(dir.path(), mapping);
        let file = GistFileRef::id_name("g1", "a.txt");
        state.gist_catalog.owned = vec![GistFile {
            raw_url: Some(
                "https://gist.githubusercontent.com/u/g1/raw/1111111111111111111111111111111111111111/a.txt"
                    .into(),
            ),
            ..GistFile::fixture("g1", "a.txt")
        }];
        state.enter_upload_confirm(UploadDraft::fixture("g1", "a.txt", &local_path), None);

        // The local file is CRLF on disk; the upload sent LF.
        on_push_done(&mut state, &file, &local_path, "hello", "hello\n");

        assert_eq!(state.screen, Screen::List, "left Confirm");
        assert_eq!(
            state.pinned[0].direction,
            Some(crate::domain::SyncDirection::Upload)
        );
        assert_eq!(
            state.pinned[0].baseline.local_sha256.as_deref(),
            Some(crate::domain::sha256_hex(b"hello").as_str())
        );
        let sent_sha = crate::domain::git_blob_sha1(b"hello\n");
        assert_eq!(
            state.pinned[0].baseline.remote_blob_sha.as_deref(),
            Some(sent_sha.as_str())
        );
        assert_eq!(
            state.catalog_blob_sha("g1", "a.txt"),
            Some(sent_sha.as_str())
        );
        assert_eq!(
            state.compute_pin_sync_status(0),
            crate::domain::SyncStatus::InSync
        );
    }

    /// A push of a file nobody pinned patches no config: `record_pin_sync`'s in-memory gate
    /// keeps it from reaching `ConfigStore` at all.
    #[test]
    fn on_push_done_of_an_unpinned_pair_touches_no_config() {
        let mut state = initial_state();
        let file = GistFileRef::id_name("g1", "a.txt");
        state.gist_catalog.owned = vec![GistFile::fixture("g1", "a.txt")];

        on_push_done(
            &mut state,
            &file,
            std::path::Path::new("/tmp/a.txt"),
            "hello",
            "hello",
        );

        assert!(state.pinned.is_empty());
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

    /// A sync Diff of `dir/a.txt` (local `old`) against gist content `new`, opened by a pull.
    fn state_on_diff(dir: &Path) -> (AppState, PathBuf) {
        let (mut state, path) = state_with_local(dir, "old\n", false);
        let request = request(&state, pair(&path, "a.txt"), SyncIntent::Pull);
        run(&mut state, &gist_serving("a.txt", "new\n"), request);
        assert!(state.screen.is_diff(), "on {:?}", state.screen);
        (state, path)
    }

    #[test]
    fn d_over_an_existing_local_file_asks_before_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, path) = state_on_diff(dir.path());

        request_download(&mut state, &path);

        assert_eq!(state.pending_action(), Some(&PendingAction::Download));
        assert!(
            state.sync_pair().is_some(),
            "the Diff stays parked under Confirm"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
    }

    #[test]
    fn d_with_the_local_file_gone_writes_it_and_leaves_the_diff() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, path) = state_on_diff(dir.path());
        std::fs::remove_file(&path).unwrap();

        request_download(&mut state, &path);

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(state.screen, Screen::List);
    }
}
