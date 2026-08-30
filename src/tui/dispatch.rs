//! `KeyOutcome` → IO side effects for the TUI event loop.
//! Extracted from `run_loop` (issue #225). Outcomes carry payloads (issue #244) so this
//! layer does not re-resolve list/detail selection.

use super::bg::*;
use super::*;
use crate::actions::SystemRunner;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

pub(super) fn dispatch_outcome(
    outcome: KeyOutcome,
    state: &mut AppState,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    jobs: &mut Jobs,
) -> Result<LoopFlow> {
    match outcome {
        KeyOutcome::EditUpload => {
            edit_upload_buffer(terminal, state, jobs)?;
        }
        KeyOutcome::EditLocal { path } => edit_local_path(terminal, state, &path)?,
        KeyOutcome::PersistSettings {
            effect,
            success_message,
        } => {
            persist_settings(state, success_message);
            if effect == Some(SettingsEffect::SyncMouseCapture) {
                sync_mouse_capture(terminal, state.settings.mouse_enabled())?;
            }
        }
        terminal_free => return Ok(route_outcome(terminal_free, state, jobs)),
    }
    Ok(LoopFlow::Proceed)
}

/// Every `KeyOutcome` that does not need a `Terminal` (issue #421). `dispatch_outcome`
/// above keeps the ones that do and delegates the rest here, so this layer is reachable
/// from a unit test with nothing but an `AppState` and a `Jobs`.
///
/// The seam stops at the spawn call. `Jobs::spawn_action` starts its thread inline
/// (`@src/tui/bg.rs`), so the arms that spawn still cannot be asserted on from a test —
/// only the ones that resolve entirely in `AppState`. Tracked as issue #422.
fn route_outcome(outcome: KeyOutcome, state: &mut AppState, jobs: &mut Jobs) -> LoopFlow {
    match outcome {
        KeyOutcome::Quit => return LoopFlow::Quit,
        KeyOutcome::PreviewDiff {
            entry,
            local_path,
            file,
            target,
            upload_orientation,
        } => {
            let (file, local_label, gist_label) =
                screens::diff::stage_preview_diff(state, local_path.clone(), file);

            jobs.spawn_gist_fetch_action(
                state,
                "Loading diff…",
                file,
                move |result, _file, state| {
                    screens::diff::on_preview_diff(
                        state,
                        entry,
                        result,
                        local_path,
                        local_label,
                        gist_label,
                        target,
                        upload_orientation,
                        None,
                    )
                },
            );
        }
        KeyOutcome::Download { mode } => download(state, mode),
        KeyOutcome::DownloadRequested { target } => {
            if target.exists() {
                state.enter_confirm_from_diff(PendingAction::Download);
            } else {
                download(state, crate::actions::DownloadMode::CreateNew);
            }
        }
        KeyOutcome::DownloadGist {
            entry,
            file,
            target,
        } => {
            let (file, local_label, gist_label) =
                screens::diff::stage_download_gist(state, target.clone(), file);

            jobs.spawn_gist_fetch_action(
                state,
                "Downloading…",
                file,
                move |result, file, state| {
                    screens::diff::on_download_selected(
                        state,
                        entry,
                        result,
                        target,
                        local_label,
                        gist_label,
                        file,
                    )
                },
            );
        }
        KeyOutcome::OpenGistDetail { gist_id } => {
            state.enter(Screen::GistDetail(Box::new(DetailState {
                gist_id: Some(gist_id),
                focus: DetailFocus::Files,
                file_cursor: 0,
                scroll: 0,
                ..DetailState::default()
            })));
            state.reset_comment_pagination();
        }
        KeyOutcome::FetchComments { gist_id } => {
            let Some(fetch_id) = screens::detail::stage_fetch_comments(state, gist_id) else {
                return LoopFlow::Proceed;
            };
            jobs.spawn_action(
                state,
                "Loading comments…",
                move || {
                    let result = load_initial_comments(&fetch_id);
                    (result, fetch_id)
                },
                move |(result, fetch_id), state| {
                    screens::detail::on_comments_initial_loaded(state, fetch_id, result)
                },
            );
        }
        KeyOutcome::LoadOlderComments { gist_id, page } => {
            let Some(fetch_id) = screens::detail::stage_load_older_comments(state, gist_id, page)
            else {
                return LoopFlow::Proceed;
            };
            jobs.spawn_action(
                state,
                "Loading older comments…",
                move || {
                    let result = crate::gh::fetch_gist_comments_page(
                        &SystemRunner,
                        &fetch_id,
                        page,
                        crate::gh::COMMENTS_PAGE_SIZE,
                    )
                    .map_err(|e| e.to_string())
                    .and_then(|raw| {
                        crate::gh::parse_gist_comments_json(&raw).map_err(|e| e.to_string())
                    });
                    (result, fetch_id)
                },
                move |(result, fetch_id), state| {
                    screens::detail::on_comments_older_loaded(state, fetch_id, result)
                },
            );
        }
        KeyOutcome::CompactGist {
            entry,
            gist_id,
            label,
        } => {
            jobs.spawn_action(
                state,
                "Checking revisions…",
                move || {
                    let result = crate::actions::execute_command(
                        &crate::actions::gist_revision_count_command(&gist_id),
                    )
                    .map_err(|e| e.to_string())
                    .and_then(|out| {
                        crate::actions::parse_revision_count(&out)
                            .ok_or_else(|| "could not parse revision count".to_string())
                    });
                    (result, gist_id, label)
                },
                move |(result, gist_id, label), state| {
                    screens::confirm::on_compact_analyze(state, entry, result, gist_id, label)
                },
            );
        }
        KeyOutcome::Pin {
            local_path,
            gist_id,
            filename,
        } => pin_paths(state, &local_path, &gist_id, &filename),
        KeyOutcome::Unpin {
            local_path,
            gist_id,
            filename,
        } => unpin_path(state, &local_path, &gist_id, &filename),
        KeyOutcome::UploadAdd {
            local_path,
            gist_id,
            filename,
        } => {
            let action = PendingAction::Upload {
                gist_id,
                filename: filename.clone(),
                local_path: local_path.clone(),
            };
            let local_label = format!("local: {}", crate::config::display_path(&local_path));
            let gist_label = "(new file)".to_string();
            match state.init_upload_state(&local_path, Some(String::new()), local_label, gist_label)
            {
                Ok(()) => {
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
        KeyOutcome::UploadPreview {
            entry,
            local_path,
            file,
        } => {
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
        KeyOutcome::Upload => {
            let Some(PendingAction::Upload {
                gist_id,
                filename,
                local_path: _,
            }) = state.pending_action().cloned()
            else {
                return LoopFlow::Proceed;
            };

            let upload_content = state.content_to_upload();

            // ScratchDir owns cleanup: `write_scratch_file` drops it on early failure; on
            // success ownership moves into the bg job and drops after execute (issue #275).
            let Some((scratch, temp_file_path)) = write_scratch_file(
                state,
                "upload",
                &filename,
                "temp file",
                upload_content.as_bytes(),
            ) else {
                return LoopFlow::Proceed;
            };

            let has_same_name = state
                .gist_catalog
                .owned
                .iter()
                .any(|g| g.gist_id == gist_id && g.filename == filename);

            let file = crate::domain::GistFileRef::id_name(gist_id, filename);
            let plan = if has_same_name {
                crate::actions::upload_command(&temp_file_path, &file.to_gist_file())
            } else {
                crate::actions::upload_add_command(&temp_file_path, &file.gist_id)
            };

            state.leave();
            jobs.spawn_action(
                state,
                "Uploading…",
                move || {
                    let result = crate::actions::execute_command(&plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    drop(scratch);
                    result
                },
                move |result, state| gist_mutation::on_upload_replace(state, result, file),
            );
        }
        KeyOutcome::Create(public) => {
            let Some(PendingAction::Create { local_path }) = state.pending_action().cloned() else {
                return LoopFlow::Proceed;
            };
            let description = state.description_input.to_string();
            let plan = crate::actions::create_command(&local_path, public, &description);

            jobs.spawn_action(
                state,
                "Creating gist…",
                move || {
                    crate::actions::execute_command(&plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                },
                move |result, state| {
                    gist_mutation::on_create_gist(state, result, local_path, public)
                },
            );
        }
        KeyOutcome::PreviewContent { entry, file } => {
            if let Some((file, preview_title)) =
                screens::preview::stage_preview_content(state, file)
            {
                jobs.spawn_gist_fetch_action(
                    state,
                    "Loading preview…",
                    file,
                    move |result, file, state| {
                        screens::preview::on_preview_content(
                            state,
                            entry,
                            result,
                            file,
                            preview_title,
                        )
                    },
                );
            }
        }
        KeyOutcome::RefreshPreview { entry, file } => {
            let (file, preview_title) = screens::preview::stage_refresh_preview(state, file);
            jobs.spawn_gist_fetch_action(
                state,
                "Loading preview…",
                file,
                move |result, file, state| {
                    screens::preview::on_preview_content(state, entry, result, file, preview_title)
                },
            );
        }
        KeyOutcome::OpenBrowser { gist_id } => open_browser_gist(state, &gist_id),
        KeyOutcome::OpenRepoUrl { url } => {
            open_url(state, &url, "Opening GitHub repository in the browser…")
        }
        KeyOutcome::CopyGistUrl { gist_id } => copy_gist_url_id(state, &gist_id),
        KeyOutcome::CopyPreviewContent => copy_preview_content(state),
        KeyOutcome::ExecuteDelete => {
            let Some(PendingAction::Delete { gist_id, .. }) = state.pending_action().cloned()
            else {
                return LoopFlow::Proceed;
            };
            let plan = crate::actions::delete_command(&gist_id);
            state.cancel_confirm_after_delete();

            jobs.spawn_action(
                state,
                "Deleting gist…",
                move || {
                    crate::actions::execute_command(&plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                },
                move |result, state| gist_mutation::on_delete_gist(state, result, gist_id),
            );
        }
        KeyOutcome::ExecuteRemoveFile => {
            let Some(PendingAction::RemoveFile {
                gist_id, filename, ..
            }) = state.pending_action().cloned()
            else {
                return LoopFlow::Proceed;
            };
            let plan = crate::actions::remove_file_command(&gist_id, &filename);
            state.back_to_list();

            jobs.spawn_action(
                state,
                "Removing file…",
                move || {
                    crate::actions::execute_command(&plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                },
                move |result, state| {
                    gist_mutation::on_remove_file(state, result, gist_id, filename)
                },
            );
        }
        KeyOutcome::ExecuteCompactGist => {
            let Some(PendingAction::CompactGist {
                gist_id,
                label,
                count,
            }) = state.pending_action().cloned()
            else {
                return LoopFlow::Proceed;
            };
            state.cancel_confirm();

            jobs.spawn_action(
                state,
                "Compacting revisions…",
                move || {
                    crate::actions::execute_compact_gist(&SystemRunner, &gist_id)
                        .map_err(|e| e.to_string())
                },
                move |result, state| gist_mutation::on_compact_gist(state, result, label, count),
            );
        }
        KeyOutcome::ApplyDescription {
            gist_id,
            description,
        } => {
            let plan = crate::actions::edit_description_command(&gist_id, &description);
            state.editing_description = false;
            state.description_input.clear();

            jobs.spawn_action(
                state,
                "Updating description…",
                move || {
                    crate::actions::execute_command(&plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                },
                move |result, state| gist_mutation::on_apply_description(state, result, gist_id),
            );
        }
        KeyOutcome::RefreshLocals => {
            jobs.request_local_scan(state);
        }
        KeyOutcome::UnpinAtPin { index } => unpin_at_pin_index(state, index),
        KeyOutcome::SyncSelectedPair {
            entry,
            local_path,
            gist_id,
            filename,
        } => {
            let local_abs = state.cwd.join(&local_path);
            let idx = state.pinned.iter().position(|m| {
                pin_local_abs(state, m) == local_abs
                    && m.gist_id == gist_id
                    && m.gist_filename == filename
            });
            let Some(idx) = idx else {
                state.set_status("pair is not pinned — press p to pin first");
                return LoopFlow::Proceed;
            };
            let m = state.pinned[idx].clone();
            let status = state.compute_pin_sync_status(idx);
            apply_sync_status(state, jobs, &m, status, entry);
        }
        KeyOutcome::SyncPinPush { entry, index } => {
            if let Some(m) = state.pinned.get(index).cloned() {
                spawn_pin_push(state, jobs, &m, entry);
            }
        }
        KeyOutcome::SyncPinPull { entry, index } => {
            if let Some(m) = state.pinned.get(index).cloned() {
                spawn_pin_pull(state, jobs, &m, entry);
            }
        }
        KeyOutcome::SyncPinAuto { entry, index } => {
            let Some(m) = state.pinned.get(index).cloned() else {
                return LoopFlow::Proceed;
            };
            let status = state.compute_pin_sync_status(index);
            apply_sync_status(state, jobs, &m, status, entry);
        }
        KeyOutcome::PreviewPinDiff { entry, index } => {
            if let Some(m) = state.pinned.get(index).cloned() {
                spawn_pin_diff(state, jobs, &m, entry);
            }
        }
        KeyOutcome::FetchRevisions { gist_id } => {
            screens::revisions::request_revisions(jobs, state, gist_id);
        }
        KeyOutcome::RevisionDiffIncremental {
            entry,
            gist_id,
            filename,
            child_version,
            parent_version,
            old_label,
            new_label,
            owner_login,
        } => {
            jobs.spawn_action(
                state,
                "Loading diff…",
                move || {
                    fetch_revision_incremental_pair(
                        &gist_id,
                        &child_version,
                        parent_version.as_deref(),
                        &filename,
                        &owner_login,
                    )
                },
                move |result, state| {
                    screens::diff::on_revision_diff(state, entry, result, old_label, new_label)
                },
            );
        }
        KeyOutcome::RevisionDiff {
            entry,
            gist_id,
            filename,
            version,
            old_label,
            new_label,
            raw_url,
            owner_login,
        } => {
            jobs.spawn_action(
                state,
                "Loading diff…",
                move || {
                    let result = fetch_revision_pair(
                        &gist_id,
                        &version,
                        &filename,
                        raw_url.as_deref(),
                        &owner_login,
                        &old_label,
                        &new_label,
                    );
                    (result, old_label, new_label)
                },
                move |(result, old_label, new_label), state| {
                    screens::diff::on_revision_diff(state, entry, result, old_label, new_label)
                },
            );
        }
        KeyOutcome::RestoreRevisionPreview {
            entry,
            gist_id,
            filename,
            version,
            version_label,
            raw_url,
            owner_login,
        } => {
            jobs.spawn_action(
                state,
                "Loading revision…",
                move || {
                    let result = fetch_revision_pair_for_restore(
                        &gist_id,
                        &version,
                        &filename,
                        raw_url.as_deref(),
                        &owner_login,
                    );
                    (result, gist_id, filename, version)
                },
                move |(result, gist_id, filename, version), state| {
                    screens::confirm::on_restore_revision_ready(
                        state,
                        entry,
                        result,
                        gist_id,
                        filename,
                        version,
                        version_label,
                    )
                },
            );
        }
        KeyOutcome::ExecuteRestoreRevision => {
            let Some(PendingAction::RestoreRevision {
                gist_id,
                filename,
                content,
                ..
            }) = state.pending_action().cloned()
            else {
                return LoopFlow::Proceed;
            };
            // ScratchDir owns cleanup: `write_scratch_file` drops it on early failure; on
            // success ownership moves into the bg job that consumes the JSON payload
            // (issue #275).
            let body = crate::actions::restore_revision_json(&filename, &content);
            let Some((scratch, json_path)) = write_scratch_file(
                state,
                "restore",
                "restore.json",
                "restore payload",
                body.as_bytes(),
            ) else {
                return LoopFlow::Proceed;
            };
            let plan = crate::actions::restore_revision_command(&gist_id, &json_path);
            jobs.spawn_action(
                state,
                "Restoring revision…",
                move || {
                    let result = crate::actions::execute_command(&plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    drop(scratch);
                    result
                },
                move |result, state| {
                    screens::revisions::on_restore_revision_done(state, result, gist_id, filename)
                },
            );
        }
        KeyOutcome::ToggleGistStar { gist_id, starring } => {
            let plan = if starring {
                crate::actions::star_gist_command(&gist_id)
            } else {
                crate::actions::unstar_gist_command(&gist_id)
            };
            let msg = if starring {
                "Starring…"
            } else {
                "Unstarring…"
            };
            jobs.spawn_action(
                state,
                msg,
                move || {
                    crate::actions::execute_command(&plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                },
                move |result, state| {
                    gist_mutation::on_gist_star_toggle(state, result, gist_id, starring)
                },
            );
        }
        KeyOutcome::ForkGist { gist_id } => {
            if state.gist_is_owned(&gist_id) {
                state.set_status("already yours — no fork needed");
                return LoopFlow::Proceed;
            }
            let plan = crate::actions::fork_gist_command(&gist_id);
            jobs.spawn_action(
                state,
                "Forking…",
                move || {
                    crate::actions::execute_command(&plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                },
                move |result, state| gist_mutation::on_fork_gist(state, result, gist_id),
            );
        }
        KeyOutcome::None => {}
        // Handled by `dispatch_outcome`'s shell above, so unreachable here. Listed
        // rather than wildcarded to guard the forward direction: a *new* variant nobody
        // routes fails to compile. The assert guards the backward one — an arm dropped
        // from the shell would otherwise arrive via `terminal_free` and quietly do
        // nothing, which no compiler check and no test would catch.
        KeyOutcome::EditUpload
        | KeyOutcome::EditLocal { .. }
        | KeyOutcome::PersistSettings { .. } => {
            debug_assert!(
                false,
                "a terminal-bearing outcome reached route_outcome — dispatch_outcome must keep its arm"
            );
        }
    }
    LoopFlow::Proceed
}

/// What to do for each [`crate::domain::SyncStatus`] arm of a pinned mapping (issue #320):
/// push/pull the resolved side, or report why neither applies. Shared by
/// `SyncSelectedPair` and `SyncPinAuto`, which only differ in how they resolve `m`.
fn apply_sync_status(
    state: &mut AppState,
    jobs: &mut Jobs,
    m: &crate::domain::PinnedMapping,
    status: crate::domain::SyncStatus,
    entry: DeferredEntry,
) {
    match status {
        crate::domain::SyncStatus::Push => spawn_pin_push(state, jobs, m, entry),
        crate::domain::SyncStatus::Pull => spawn_pin_pull(state, jobs, m, entry),
        crate::domain::SyncStatus::InSync => state.set_status("already in sync"),
        crate::domain::SyncStatus::Missing => {
            state.set_status("local file is missing — use d to pull it back")
        }
        crate::domain::SyncStatus::Unknown => {
            state.set_status("can't tell which side is newer — use u to push or d to pull")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{GistFile, LocalCandidate, PinnedMapping};
    use crate::tui::test_support::idle_jobs;
    use std::path::PathBuf;

    /// A pinned `/cwd/a.txt` ↔ `g1:a.txt` pair. `local_mtime` and `remote_updated_at`
    /// are the only inputs `compute_pin_sync_status` reads for a hash-less mapping, so
    /// varying them alone walks `apply_sync_status` through its non-spawning arms.
    fn state_with_one_pin(
        cwd: PathBuf,
        local_mtime: Option<u64>,
        remote_updated_at: Option<&str>,
    ) -> AppState {
        let mut state = initial_state();
        state.cwd = cwd;
        state.pinned = vec![PinnedMapping {
            local_path: PathBuf::from("a.txt"),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
        }];
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

    fn route(state: &mut AppState, outcome: KeyOutcome) -> LoopFlow {
        route_outcome(outcome, state, &mut idle_jobs())
    }

    // ---- apply_sync_status's non-spawning arms ---------------------------

    #[test]
    fn sync_pin_auto_reports_in_sync_when_both_sides_share_an_mtime() {
        let updated_at = "2026-06-10T00:00:00Z";
        let ts = crate::domain::parse_rfc3339_to_unix(updated_at).unwrap();
        let mut state = state_with_one_pin(PathBuf::from("/cwd"), Some(ts), Some(updated_at));
        let entry = state.defer_entry();

        route(&mut state, KeyOutcome::SyncPinAuto { entry, index: 0 });

        assert_eq!(state.status.as_deref(), Some("already in sync"));
        assert!(state.bg_task_msg.is_none(), "InSync must not spawn");
    }

    #[test]
    fn sync_pin_auto_reports_a_missing_local_file() {
        // `pin_mtimes` falls back to stat-ing the path when `locals` has no match, so the
        // cwd must really be empty rather than merely improbable — hence a temp dir.
        let cwd = tempfile::tempdir().unwrap();
        let mut state =
            state_with_one_pin(cwd.path().to_path_buf(), None, Some("2026-06-10T00:00:00Z"));
        let entry = state.defer_entry();

        route(&mut state, KeyOutcome::SyncPinAuto { entry, index: 0 });

        assert_eq!(
            state.status.as_deref(),
            Some("local file is missing — use d to pull it back")
        );
        assert!(state.bg_task_msg.is_none(), "Missing must not spawn");
    }

    #[test]
    fn sync_pin_auto_reports_unknown_when_the_gist_side_is_absent() {
        let mut state = state_with_one_pin(PathBuf::from("/cwd"), Some(1_780_000_000), None);
        let entry = state.defer_entry();

        route(&mut state, KeyOutcome::SyncPinAuto { entry, index: 0 });

        assert_eq!(
            state.status.as_deref(),
            Some("can't tell which side is newer — use u to push or d to pull")
        );
        assert!(state.bg_task_msg.is_none(), "Unknown must not spawn");
    }

    // ---- early returns ----------------------------------------------------

    #[test]
    fn fork_gist_on_an_owned_gist_returns_before_spawning() {
        let mut state = test_support::state_with_gists();

        route(
            &mut state,
            KeyOutcome::ForkGist {
                gist_id: "g1".into(),
            },
        );

        assert_eq!(
            state.status.as_deref(),
            Some("already yours — no fork needed")
        );
        assert!(
            state.bg_task_msg.is_none(),
            "an owned gist must not be forked"
        );
    }

    /// The listed-not-wildcarded arm guards new variants; this guards the other
    /// direction, where an arm is dropped from `dispatch_outcome` and would otherwise
    /// become a silent no-op.
    #[test]
    #[should_panic(expected = "must keep its arm")]
    fn a_terminal_bearing_outcome_reaching_route_outcome_trips_the_assert() {
        let mut state = initial_state();
        route(&mut state, KeyOutcome::EditUpload);
    }

    #[test]
    fn execute_delete_without_a_pending_action_spawns_nothing() {
        let mut state = test_support::state_with_gists();

        route(&mut state, KeyOutcome::ExecuteDelete);

        assert!(state.pending_action().is_none());
        assert!(
            state.bg_task_msg.is_none(),
            "a delete with nothing pending must not reach gh"
        );
    }
}
