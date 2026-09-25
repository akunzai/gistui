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
/// Each spawn arm reifies its semantic kind, progress label, and non-content identity in an
/// `ActionJobSpec`; `Jobs` hands that spec and the opaque closure to its action-spawner adapter
/// (`src/tui/bg.rs`, issue #422). Tests use the recording adapter, so spawn arms are
/// observable here without executing their closures.
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
                ActionJobSpec::new(
                    ActionJobKind::FetchComments {
                        gist_id: fetch_id.clone(),
                        page: None,
                    },
                    "Loading comments…",
                ),
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
                ActionJobSpec::new(
                    ActionJobKind::FetchComments {
                        gist_id: fetch_id.clone(),
                        page: Some(page),
                    },
                    "Loading older comments…",
                ),
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
                ActionJobSpec::new(
                    ActionJobKind::AnalyzeCompact {
                        gist_id: gist_id.clone(),
                    },
                    "Checking revisions…",
                ),
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
            let local_label = format!("local: {}", crate::config::display_path(&local_path));
            match UploadDraft::read(
                gist_id,
                filename,
                local_path.clone(),
                String::new(),
                local_label,
                "(new file)".to_string(),
            ) {
                Ok(draft) => state.enter_upload_confirm(draft, None),
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
            let Some(draft) = state.upload_draft() else {
                return LoopFlow::Proceed;
            };
            let upload_content = draft.content(&state.settings);
            // The pin baseline is the local file on disk, not the bytes sent (#465).
            let local_content = draft.original_content.clone();
            let (gist_id, filename, local_path) = (
                draft.gist_id.clone(),
                draft.filename.clone(),
                draft.local_path.clone(),
            );

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

            // Confirm is gone once the job runs: the outcome gets the target and the local
            // file's bytes as read, never a re-read of the Upload draft on Confirm (#460).
            state.leave();
            let runner = jobs.command_runner();
            jobs.spawn_action(
                state,
                ActionJobSpec::new(ActionJobKind::Upload { file: file.clone() }, "Uploading…"),
                move || {
                    let result = crate::actions::run_command(runner.as_ref(), &plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    drop(scratch);
                    result
                },
                move |result, state| {
                    gist_mutation::on_upload_replace(
                        state,
                        result,
                        file,
                        &local_path,
                        &local_content,
                        &upload_content,
                    )
                },
            );
        }
        KeyOutcome::Create(public) => {
            let Some(PendingAction::Create { local_path }) = state.pending_action().cloned() else {
                return LoopFlow::Proceed;
            };
            let description = state.description_input.to_string();
            // Send the bytes the Sync policy dictates (#465). When that rewrites the file,
            // upload a same-named scratch copy; otherwise hand `gh` the file itself, so an
            // unreadable or non-text file still creates as before.
            let normalized = crate::domain::read_text_file_capped(&local_path)
                .ok()
                .and_then(|text| match state.settings.sync_policy().outbound(&text) {
                    std::borrow::Cow::Owned(normalized) => Some(normalized),
                    std::borrow::Cow::Borrowed(_) => None,
                });
            let mut scratch = None;
            let mut source = local_path.clone();
            if let Some(normalized) = normalized {
                let Some(filename) = local_path.file_name().and_then(|n| n.to_str()) else {
                    return LoopFlow::Proceed;
                };
                let Some((dir, path)) = write_scratch_file(
                    state,
                    "create",
                    filename,
                    "temp file",
                    normalized.as_bytes(),
                ) else {
                    return LoopFlow::Proceed;
                };
                scratch = Some(dir);
                source = path;
            }
            let plan = crate::actions::create_command(&source, public, &description);
            let runner = jobs.command_runner();

            jobs.spawn_action(
                state,
                ActionJobSpec::new(
                    ActionJobKind::Create {
                        local_path: local_path.clone(),
                        public,
                    },
                    "Creating gist…",
                ),
                move || {
                    let result = crate::actions::run_command(runner.as_ref(), &plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    drop(scratch);
                    result
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
                ActionJobSpec::new(
                    ActionJobKind::DeleteGist {
                        gist_id: gist_id.clone(),
                    },
                    "Deleting gist…",
                ),
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
                ActionJobSpec::new(
                    ActionJobKind::RemoveFile {
                        file: crate::domain::GistFileRef::id_name(
                            gist_id.clone(),
                            filename.clone(),
                        ),
                    },
                    "Removing file…",
                ),
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
                ActionJobSpec::new(
                    ActionJobKind::CompactGist {
                        gist_id: gist_id.clone(),
                    },
                    "Compacting revisions…",
                ),
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
                ActionJobSpec::new(
                    ActionJobKind::UpdateDescription {
                        gist_id: gist_id.clone(),
                    },
                    "Updating description…",
                ),
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
            let idx = crate::pins::find_by_resolved_path(
                &state.pinned,
                &state.cwd,
                crate::pins::PinKey::new(&local_abs, &gist_id, &filename),
            );
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
        KeyOutcome::Revision(request) => gist_revision::dispatch(jobs, state, request),
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
                ActionJobSpec::new(
                    ActionJobKind::ToggleGistStar {
                        gist_id: gist_id.clone(),
                        starring,
                    },
                    msg,
                ),
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
                ActionJobSpec::new(
                    ActionJobKind::ForkGist {
                        gist_id: gist_id.clone(),
                    },
                    "Forking…",
                ),
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
        // Both sides changed: show the diff and let the user pick d or u (issue #466).
        crate::domain::SyncStatus::Conflict => {
            spawn_pin_diff_then(state, jobs, m, entry, "both sides changed — press d or u")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{GistFile, LocalCandidate, PinnedMapping};
    use crate::tui::test_support::{idle_jobs, recording_jobs};
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
            remote_blob_sha: None,
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

    #[test]
    fn preview_content_routes_the_staged_gist_fetch_without_running_it() {
        let mut state = test_support::state_with_gists();
        let entry = state.defer_entry();
        let file = crate::domain::GistFileRef::new(
            "g1",
            "a.txt",
            Some("https://example.test/g1/a.txt".into()),
        );
        let (mut jobs, started) = recording_jobs();

        route_outcome(
            KeyOutcome::PreviewContent {
                entry,
                file: file.clone(),
            },
            &mut state,
            &mut jobs,
        );

        assert_eq!(
            started.take(),
            vec![ActionJobSpec::gist_fetch("Loading preview…", file)]
        );
    }

    #[test]
    fn fork_gist_routes_an_action_job_without_running_it() {
        let mut state = initial_state();
        let (mut jobs, started) = recording_jobs();

        route_outcome(
            KeyOutcome::ForkGist {
                gist_id: "not-owned".into(),
            },
            &mut state,
            &mut jobs,
        );

        assert_eq!(
            started.take(),
            vec![ActionJobSpec::new(
                ActionJobKind::ForkGist {
                    gist_id: "not-owned".into(),
                },
                "Forking…",
            )]
        );
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

    /// Succeeds every command and records, per call, the contents of each argument that
    /// names an existing file — the scratch copy a job hands `gh` is gone once it returns.
    #[derive(Default)]
    struct FileCapturingRunner {
        sent: std::sync::Mutex<Vec<(crate::actions::CommandPlan, Vec<String>)>>,
    }

    impl crate::actions::CommandRunner for FileCapturingRunner {
        fn run(
            &self,
            plan: &crate::actions::CommandPlan,
        ) -> anyhow::Result<crate::actions::CommandOutput> {
            let files = plan
                .args
                .iter()
                .filter_map(|arg| std::fs::read_to_string(arg).ok())
                .collect();
            self.sent.lock().unwrap().push((plan.clone(), files));
            Ok(crate::actions::CommandOutput::ok(""))
        }
    }

    /// Issue #460: a pin push confirmed from Pins → Confirm must record the pin sync and
    /// return to Pins, even after the upload arm has left Confirm and a setting changes while
    /// the job runs. Issue #465: it sends the normalized bytes, but the pin baseline is the
    /// local file as it sits on disk.
    #[test]
    fn upload_from_pin_push_records_pin_sync_and_returns_to_pins() {
        use crate::domain::SyncDirection;

        let _guard = crate::config::tests::ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", dir.path());

        let local_path = dir.path().join("a.txt");
        std::fs::write(&local_path, "a\r\nb\r\n").unwrap();
        let mapping = PinnedMapping {
            local_path: local_path.clone(),
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: None,
            remote_blob_sha: None,
        };
        let mut config = crate::config::AppConfig::default();
        config.pinned.push(mapping.clone());
        crate::config::save_config(&crate::config::config_path().unwrap(), &config).unwrap();

        let mut state = initial_state();
        state.cwd = dir.path().to_path_buf();
        state.pinned = vec![mapping];
        state.gist_catalog.owned = vec![GistFile::fixture("g1", "a.txt")];
        state.enter(Screen::Pins(Box::default()));
        state.enter_upload_confirm(
            UploadDraft {
                original_content: "a\r\nb\r\n".into(),
                ..UploadDraft::fixture("g1", "a.txt", &local_path)
            },
            None,
        );

        let runner = std::sync::Arc::new(FileCapturingRunner::default());
        let mut jobs = Jobs::inline(&state.gist_catalog.clone(), runner.clone());
        route_outcome(KeyOutcome::Upload, &mut state, &mut jobs);
        // A setting flipped mid-upload must not change what the pin records.
        state
            .settings
            .adjust(crate::tui::ConfigField::NormalizeLineEndings, true);
        // Drain only the action outcome: `absorb` would start a real gist-list refresh.
        jobs.on_action_outcome(&mut state);

        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let sent = runner.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, vec!["a\nb\n".to_string()]);
        assert_eq!(state.status.as_deref(), Some("Uploaded a.txt to gist g1"));
        assert_eq!(state.pinned[0].direction, Some(SyncDirection::Upload));
        assert_eq!(
            state.pinned[0].last_seen_hash.as_deref(),
            Some(crate::domain::sha256_hex(b"a\r\nb\r\n").as_str())
        );
        assert!(state.screen.is_pins(), "landed on {:?}", state.screen);
        assert_eq!(state.nav_stack.len(), 1);
    }

    /// Issue #465: create sends the bytes the Sync policy dictates — a normalized scratch copy
    /// under the same filename when normalization rewrites the file, the file itself otherwise.
    #[test]
    fn create_sends_normalized_bytes_only_when_normalization_rewrites_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let local_path = dir.path().join("a.txt");
        std::fs::write(&local_path, "a\r\nb\r\n").unwrap();

        for (normalize, expected) in [(true, "a\nb\n"), (false, "a\r\nb\r\n")] {
            let mut state = initial_state();
            if !normalize {
                state
                    .settings
                    .adjust(crate::tui::ConfigField::NormalizeLineEndings, true);
            }
            state.enter_confirm(
                PendingAction::Create {
                    local_path: local_path.clone(),
                },
                String::new(),
            );
            let runner = std::sync::Arc::new(FileCapturingRunner::default());
            let mut jobs = Jobs::inline(&state.gist_catalog.clone(), runner.clone());

            route_outcome(KeyOutcome::Create(false), &mut state, &mut jobs);

            let sent = runner.sent.lock().unwrap();
            assert_eq!(sent.len(), 1, "normalize={normalize}");
            let (plan, files) = &sent[0];
            assert_eq!(&plan.args[..2], ["gist", "create"]);
            assert!(plan.args[2].ends_with("a.txt"), "{:?}", plan.args);
            assert_eq!(
                plan.args[2] == local_path.display().to_string(),
                !normalize,
                "normalize={normalize}: {:?}",
                plan.args
            );
            assert_eq!(files, &vec![expected.to_string()], "normalize={normalize}");
        }
    }

    /// Issue #466: smart-sync on a Conflict opens the pin's diff instead of picking a side.
    #[test]
    fn sync_pin_auto_on_conflict_opens_the_pin_diff() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("a.txt");
        std::fs::write(&local, "edited").unwrap();
        let mut state = initial_state();
        state.pinned = vec![PinnedMapping {
            local_path: local,
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: Some(crate::domain::sha256_hex(b"synced")),
            remote_blob_sha: Some("1111111111111111111111111111111111111111".into()),
        }];
        state.gist_catalog.owned = vec![GistFile {
            raw_url: Some(
                "https://gist.githubusercontent.com/u/g1/raw/2222222222222222222222222222222222222222/a.txt"
                    .into(),
            ),
            ..GistFile::fixture("g1", "a.txt")
        }];
        assert_eq!(
            state.compute_pin_sync_status(0),
            crate::domain::SyncStatus::Conflict
        );
        let entry = state.defer_entry();
        let (mut jobs, started) = recording_jobs();

        route_outcome(
            KeyOutcome::SyncPinAuto { entry, index: 0 },
            &mut state,
            &mut jobs,
        );

        let started = started.take();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].progress, "Loading diff…");
    }
}
