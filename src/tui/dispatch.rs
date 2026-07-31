//! `KeyOutcome` → IO side effects for the TUI event loop.
//! Extracted from `run_loop` (issue #225). Outcomes carry payloads (issue #244) so this
//! layer does not re-resolve list/detail selection.

use super::bg::*;
use super::*;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

pub(super) fn dispatch_outcome(
    outcome: KeyOutcome,
    state: &mut AppState,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    jobs: &mut Jobs,
) -> Result<LoopFlow> {
    match outcome {
        KeyOutcome::Quit => return Ok(LoopFlow::Quit),
        KeyOutcome::PreviewDiff {
            local_path,
            file,
            target,
            upload_orientation,
        } => {
            // List-originated diff returns to List on Esc.
            state.pending_return = Some(Screen::List);
            let gist = file.to_gist_file();
            let (local_label, gist_label) = diff_labels(local_path.as_deref(), &gist);

            jobs.spawn_gist_fetch_action(state, "Loading diff…", file, move |result, _file| {
                BgTaskOutcome::PreviewDiff {
                    result,
                    local_path,
                    local_label,
                    gist_label,
                    target,
                    upload_orientation,
                }
            });
        }
        KeyOutcome::Download { mode } => download(state, mode),
        KeyOutcome::DownloadRequested { target } => {
            if target.exists() {
                state.enter_confirm_from_diff(PendingAction::Download);
            } else {
                download(state, crate::actions::DownloadMode::CreateNew);
            }
        }
        KeyOutcome::DownloadGist { file, target } => {
            let gist = file.to_gist_file();
            let (local_label, gist_label) = diff_labels(Some(&target), &gist);

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
            if state
                .detail()
                .is_some_and(|d| d.comments.is_some() || d.comments_loading)
            {
                return Ok(LoopFlow::Proceed);
            }
            if let Some(d) = state.detail_mut() {
                d.comments_loading = true;
            }
            let fetch_id = gist_id.clone();
            jobs.spawn_action(state, "Loading comments…", move || {
                let result = load_initial_comments(&fetch_id);
                BgTaskOutcome::CommentsInitialLoaded {
                    gist_id: fetch_id,
                    result,
                }
            });
        }
        KeyOutcome::LoadOlderComments { gist_id, page } => {
            if page == 0 || !state.can_load_older_comments() {
                return Ok(LoopFlow::Proceed);
            }
            if let Some(d) = state.detail_mut() {
                d.comments_loading_more = true;
            }
            let fetch_id = gist_id.clone();
            jobs.spawn_action(state, "Loading older comments…", move || {
                let result = crate::gh::fetch_gist_comments_page(
                    &fetch_id,
                    page,
                    crate::gh::COMMENTS_PAGE_SIZE,
                )
                .map_err(|e| e.to_string())
                .and_then(|raw| {
                    crate::gh::parse_gist_comments_json(&raw).map_err(|e| e.to_string())
                });
                BgTaskOutcome::CommentsOlderLoaded {
                    gist_id: fetch_id,
                    result,
                }
            });
        }
        KeyOutcome::CompactGist { gist_id, label } => {
            jobs.spawn_action(state, "Checking revisions…", move || {
                let result = crate::actions::execute_command(
                    &crate::actions::gist_revision_count_command(&gist_id),
                )
                .map_err(|e| e.to_string())
                .and_then(|out| {
                    crate::actions::parse_revision_count(&out)
                        .ok_or_else(|| "could not parse revision count".to_string())
                });
                BgTaskOutcome::CompactAnalyze {
                    result,
                    gist_id,
                    label,
                }
            });
        }
        KeyOutcome::Pin {
            local_path,
            gist_id,
            filename,
        } => pin_paths(state, &local_path, &gist_id, &filename),
        KeyOutcome::Unpin {
            local_path,
            gist_id: _,
            filename: _,
        } => unpin_path(state, &local_path),
        KeyOutcome::UploadAdd {
            local_path,
            gist_id,
            filename,
        } => {
            if !state.is_pin_diff_context() {
                state.pending_return = Some(Screen::List);
            }
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
            local_path,
            file,
            from_pin_diff,
        } => {
            if !from_pin_diff {
                state.pending_return = Some(Screen::List);
            }
            let gist_file = state
                .gists
                .iter()
                .find(|g| g.gist_id == file.gist_id && g.filename == file.filename)
                .cloned()
                .unwrap_or_else(|| file.to_gist_file());
            let (local_label, gist_label) = diff_labels(Some(&local_path), &gist_file);
            // Prefer list-row raw_url when present (may be richer than the outcome's ref).
            let mut file = file;
            if file.raw_url.is_none() {
                file.raw_url = gist_file.raw_url.clone();
            }

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
        KeyOutcome::Upload => {
            let Some(PendingAction::Upload {
                gist_id,
                filename,
                local_path: _,
            }) = state.pending_action().cloned()
            else {
                return Ok(LoopFlow::Proceed);
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
                return Ok(LoopFlow::Proceed);
            };

            let has_same_name = state
                .gists
                .iter()
                .any(|g| g.gist_id == gist_id && g.filename == filename);

            let file = crate::domain::GistFileRef::id_name(gist_id, filename);
            let plan = if has_same_name {
                crate::actions::upload_command(&temp_file_path, &file.to_gist_file())
            } else {
                crate::actions::upload_add_command(&temp_file_path, &file.gist_id)
            };

            state.staged_diff_gist = None;
            state.leave();
            jobs.spawn_action(state, "Uploading…", move || {
                let result = crate::actions::execute_command(&plan)
                    .map(|_| ())
                    .map_err(|e| e.to_string());

                drop(scratch);

                BgTaskOutcome::UploadReplace { result, file }
            });
        }
        KeyOutcome::EditUpload => {
            edit_upload_buffer(terminal, state, jobs)?;
        }
        KeyOutcome::Create(public) => {
            let Some(PendingAction::Create { local_path }) = state.pending_action().cloned() else {
                return Ok(LoopFlow::Proceed);
            };
            let description = state.description_input.to_string();
            let plan = crate::actions::create_command(&local_path, public, &description);

            jobs.spawn_action(state, "Creating gist…", move || {
                let result = crate::actions::execute_command(&plan)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                BgTaskOutcome::CreateGist {
                    result,
                    local_path,
                    public,
                }
            });
        }
        KeyOutcome::PreviewContent { mut file } => {
            let key = file.cache_key();
            if let Some(content) = state.gist_content_cache.get(&key).cloned() {
                state.enter_preview(
                    format!("Preview: {} / {}", file.gist_id, file.filename),
                    content,
                    Some(key),
                );
            } else {
                if file.raw_url.is_none() {
                    file.raw_url = state.gist_file_raw_url(&file.gist_id, &file.filename);
                }
                let preview_title = format!("Preview: {} / {}", file.gist_id, file.filename);
                jobs.spawn_gist_fetch_action(
                    state,
                    "Loading preview…",
                    file,
                    move |result, file| BgTaskOutcome::PreviewContent {
                        result,
                        file,
                        preview_title,
                    },
                );
            }
        }
        KeyOutcome::RefreshPreview { mut file } => {
            // Keep the current return path when reloading.
            if state.screen.is_preview() {
                state.pending_return = state.nav_stack.last().cloned();
            }
            state.gist_content_cache.remove(&file.cache_key());
            if file.raw_url.is_none() {
                file.raw_url = state.gist_file_raw_url(&file.gist_id, &file.filename);
            }
            let preview_title = format!("Preview: {} / {}", file.gist_id, file.filename);
            jobs.spawn_gist_fetch_action(state, "Loading preview…", file, move |result, file| {
                BgTaskOutcome::PreviewContent {
                    result,
                    file,
                    preview_title,
                }
            });
        }
        KeyOutcome::OpenBrowser { gist_id } => open_browser_gist(state, &gist_id),
        KeyOutcome::OpenRepoUrl { url } => {
            open_url(state, &url, "Opening GitHub repository in the browser…")
        }
        KeyOutcome::CopyGistUrl { gist_id } => copy_gist_url_id(state, &gist_id),
        KeyOutcome::CopyPreviewContent => copy_preview_content(state),
        KeyOutcome::EditLocal { path } => edit_local_path(terminal, state, &path)?,
        KeyOutcome::ExecuteDelete => {
            let Some(PendingAction::Delete { gist_id, .. }) = state.pending_action().cloned()
            else {
                return Ok(LoopFlow::Proceed);
            };
            let plan = crate::actions::delete_command(&gist_id);
            state.back_to_list();

            jobs.spawn_action(state, "Deleting gist…", move || {
                let result = crate::actions::execute_command(&plan)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                BgTaskOutcome::DeleteGist { result, gist_id }
            });
        }
        KeyOutcome::ExecuteRemoveFile => {
            let Some(PendingAction::RemoveFile {
                gist_id, filename, ..
            }) = state.pending_action().cloned()
            else {
                return Ok(LoopFlow::Proceed);
            };
            let plan = crate::actions::remove_file_command(&gist_id, &filename);
            state.back_to_list();

            jobs.spawn_action(state, "Removing file…", move || {
                let result = crate::actions::execute_command(&plan)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                BgTaskOutcome::RemoveFile {
                    result,
                    gist_id,
                    filename,
                }
            });
        }
        KeyOutcome::ExecuteCompactGist => {
            let Some(PendingAction::CompactGist {
                gist_id,
                label,
                count,
            }) = state.pending_action().cloned()
            else {
                return Ok(LoopFlow::Proceed);
            };
            state.cancel_confirm();

            jobs.spawn_action(state, "Compacting revisions…", move || {
                let result =
                    crate::actions::execute_compact_gist(&gist_id).map_err(|e| e.to_string());
                BgTaskOutcome::CompactGist {
                    result,
                    label,
                    count,
                }
            });
        }
        KeyOutcome::ApplyDescription {
            gist_id,
            description,
        } => {
            let plan = crate::actions::edit_description_command(&gist_id, &description);
            state.editing_description = false;
            state.description_input.clear();

            jobs.spawn_action(state, "Updating description…", move || {
                let result = crate::actions::execute_command(&plan)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                BgTaskOutcome::ApplyDescription { result, gist_id }
            });
        }
        KeyOutcome::RefreshLocals => {
            jobs.request_local_scan(state);
        }
        KeyOutcome::UnpinAtPin { index } => unpin_at_pin_index(state, index),
        KeyOutcome::SyncSelectedPair {
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
                return Ok(LoopFlow::Proceed);
            };
            let m = state.pinned[idx].clone();
            let status = state.compute_pin_sync_status(idx);
            apply_sync_status(state, jobs, &m, status);
        }
        KeyOutcome::SyncPinPush { index } => {
            if let Some(m) = state.pinned.get(index).cloned() {
                spawn_pin_push(state, jobs, &m);
            }
        }
        KeyOutcome::SyncPinPull { index } => {
            if let Some(m) = state.pinned.get(index).cloned() {
                spawn_pin_pull(state, jobs, &m);
            }
        }
        KeyOutcome::SyncPinAuto { index } => {
            let Some(m) = state.pinned.get(index).cloned() else {
                return Ok(LoopFlow::Proceed);
            };
            let status = state.compute_pin_sync_status(index);
            apply_sync_status(state, jobs, &m, status);
        }
        KeyOutcome::PreviewPinDiff { index } => {
            if let Some(m) = state.pinned.get(index).cloned() {
                park_pins_on_diff_return(state);
                spawn_pin_diff(state, jobs, &m);
            }
        }
        KeyOutcome::PersistDiffContext => persist_diff_context(state),
        KeyOutcome::PersistSettings => {
            persist_settings(state);
            sync_mouse_capture(terminal, state.mouse_enabled)?;
        }
        KeyOutcome::ThemeToggle => persist_theme(state),
        KeyOutcome::FetchRevisions { gist_id } => {
            jobs.spawn_action(state, "Loading revisions…", move || {
                let result = crate::gh::fetch_gist_commits_json(&gist_id)
                    .map_err(|e| e.to_string())
                    .and_then(|raw| {
                        crate::gh::parse_gist_commits_json(&raw).map_err(|e| e.to_string())
                    });
                BgTaskOutcome::RevisionsFetched { gist_id, result }
            });
        }
        KeyOutcome::RevisionDiffIncremental {
            gist_id,
            filename,
            child_version,
            parent_version,
            old_label,
            new_label,
            owner_login,
        } => {
            jobs.spawn_action(state, "Loading diff…", move || {
                let result = fetch_revision_incremental_pair(
                    &gist_id,
                    &child_version,
                    parent_version.as_deref(),
                    &filename,
                    &owner_login,
                );
                BgTaskOutcome::RevisionDiff {
                    result,
                    old_label,
                    new_label,
                }
            });
        }
        KeyOutcome::RevisionDiff {
            gist_id,
            filename,
            version,
            old_label,
            new_label,
            raw_url,
            owner_login,
        } => {
            jobs.spawn_action(state, "Loading diff…", move || {
                let result = fetch_revision_pair(
                    &gist_id,
                    &version,
                    &filename,
                    raw_url.as_deref(),
                    &owner_login,
                    &old_label,
                    &new_label,
                );
                BgTaskOutcome::RevisionDiff {
                    result,
                    old_label,
                    new_label,
                }
            });
        }
        KeyOutcome::RestoreRevisionPreview {
            gist_id,
            filename,
            version,
            version_label,
            raw_url,
            owner_login,
        } => {
            jobs.spawn_action(state, "Loading revision…", move || {
                let result = fetch_revision_pair_for_restore(
                    &gist_id,
                    &version,
                    &filename,
                    raw_url.as_deref(),
                    &owner_login,
                );
                BgTaskOutcome::RestoreRevisionReady {
                    result,
                    gist_id,
                    filename,
                    version,
                    version_label,
                }
            });
        }
        KeyOutcome::ExecuteRestoreRevision => {
            let Some(PendingAction::RestoreRevision {
                gist_id,
                filename,
                content,
                ..
            }) = state.pending_action().cloned()
            else {
                return Ok(LoopFlow::Proceed);
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
                return Ok(LoopFlow::Proceed);
            };
            let plan = crate::actions::restore_revision_command(&gist_id, &json_path);
            jobs.spawn_action(state, "Restoring revision…", move || {
                let result = crate::actions::execute_command(&plan)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                drop(scratch);
                BgTaskOutcome::RestoreRevisionDone {
                    result,
                    gist_id,
                    filename,
                }
            });
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
            jobs.spawn_action(state, msg, move || {
                let result = crate::actions::execute_command(&plan)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                BgTaskOutcome::GistStarToggle {
                    result,
                    gist_id,
                    starred: starring,
                }
            });
        }
        KeyOutcome::ForkGist { gist_id } => {
            if state.gist_is_owned(&gist_id) {
                state.set_status("already yours — no fork needed");
                return Ok(LoopFlow::Proceed);
            }
            let plan = crate::actions::fork_gist_command(&gist_id);
            jobs.spawn_action(state, "Forking…", move || {
                let result = crate::actions::execute_command(&plan)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                BgTaskOutcome::ForkGist { result, gist_id }
            });
        }
        KeyOutcome::None => {}
    }
    Ok(LoopFlow::Proceed)
}

/// What to do for each [`crate::domain::SyncStatus`] arm of a pinned mapping (issue #320):
/// push/pull the resolved side, or report why neither applies. Shared by
/// `SyncSelectedPair` and `SyncPinAuto`, which only differ in how they resolve `m`.
fn apply_sync_status(
    state: &mut AppState,
    jobs: &mut Jobs,
    m: &crate::domain::PinnedMapping,
    status: crate::domain::SyncStatus,
) {
    match status {
        crate::domain::SyncStatus::Push => spawn_pin_push(state, jobs, m),
        crate::domain::SyncStatus::Pull => spawn_pin_pull(state, jobs, m),
        crate::domain::SyncStatus::InSync => state.set_status("already in sync"),
        crate::domain::SyncStatus::Missing => {
            state.set_status("local file is missing — use d to pull it back")
        }
        crate::domain::SyncStatus::Unknown => {
            state.set_status("can't tell which side is newer — use u to push or d to pull")
        }
    }
}
