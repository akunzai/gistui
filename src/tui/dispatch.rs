//! `KeyOutcome` → IO side effects for the TUI event loop.
//! Extracted from `run_loop` (issue #225).

use super::bg::*;
use super::*;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

pub(super) fn dispatch_outcome(
    outcome: KeyOutcome,
    state: &mut AppState,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    channels: &mut BgChannels,
) -> Result<LoopFlow> {
    match outcome {
        KeyOutcome::Quit => return Ok(LoopFlow::Quit),
        KeyOutcome::PreviewDiff => {
            let Some(ranked) = state.selected_gist() else {
                return Ok(LoopFlow::Proceed);
            };
            // List-originated diff returns to the List on Esc (reset any
            // leftover Pins origin from an earlier pin diff).
            state.diff_return = Screen::List;
            let local_path = state.selected_local().map(|local| local.path.clone());
            let gist = ranked.file.clone();
            let gist_id = gist.gist_id.clone();
            let filename = gist.filename.clone();
            let raw_url = gist.raw_url.clone();
            let (local_label, gist_label) = diff_labels(local_path.as_deref(), &gist);
            let target = state.cwd.join(&filename);
            let upload_orientation = state.focus == FocusPane::Local;

            spawn_bg(state, &mut channels.bg, "Loading diff…", move || {
                let result = fetch_gist_content(&gist_id, &filename, raw_url.as_deref());
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
        KeyOutcome::Download => download(state),
        KeyOutcome::DownloadGist => {
            let Some(ranked) = state.selected_gist() else {
                return Ok(LoopFlow::Proceed);
            };
            let gist = ranked.file.clone();
            let gist_id = gist.gist_id.clone();
            let filename = gist.filename.clone();
            let raw_url = gist.raw_url.clone();
            let target = state.cwd.join(&filename);
            let (local_label, gist_label) = diff_labels(Some(&target), &gist);

            spawn_bg(state, &mut channels.bg, "Downloading…", move || {
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
        KeyOutcome::OpenGistDetail => {
            let Some(group) = state.selected_group() else {
                return Ok(LoopFlow::Proceed);
            };
            let gist_id = group.id.clone();
            state.screen = Screen::GistDetail;
            state.detail.gist_id = Some(gist_id);
            state.reset_comment_pagination();
            state.detail.scroll = 0;
            state.detail.focus = DetailFocus::Files;
            state.detail.file_cursor = 0;
        }
        KeyOutcome::FetchComments => {
            let Some(gist_id) = state.detail.gist_id.clone() else {
                return Ok(LoopFlow::Proceed);
            };
            if state.detail.comments.is_some() || state.detail.comments_loading {
                return Ok(LoopFlow::Proceed);
            }
            state.detail.comments_loading = true;
            let fetch_id = gist_id.clone();
            spawn_bg(state, &mut channels.bg, "Loading comments…", move || {
                let result = load_initial_comments(&fetch_id);
                BgTaskOutcome::CommentsInitialLoaded {
                    gist_id: fetch_id,
                    result,
                }
            });
        }
        KeyOutcome::LoadOlderComments => {
            let Some(gist_id) = state.detail.gist_id.clone() else {
                return Ok(LoopFlow::Proceed);
            };
            if !state.can_load_older_comments() {
                return Ok(LoopFlow::Proceed);
            }
            let page = state.detail.comments_loaded_oldest_page.saturating_sub(1);
            if page == 0 {
                return Ok(LoopFlow::Proceed);
            }
            state.detail.comments_loading_more = true;
            let fetch_id = gist_id.clone();
            spawn_bg(
                state,
                &mut channels.bg,
                "Loading older comments…",
                move || {
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
                },
            );
        }
        KeyOutcome::CompactGist => {
            let Some(gist_id) = state.context_gist_id() else {
                return Ok(LoopFlow::Proceed);
            };
            let Some(group) = state.group_by_id(&gist_id) else {
                return Ok(LoopFlow::Proceed);
            };
            let label = if group.description.trim().is_empty() {
                group.id.clone()
            } else {
                group.description.clone()
            };

            spawn_bg(
                state,
                &mut channels.bg,
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
                    BgTaskOutcome::CompactAnalyze {
                        result,
                        gist_id,
                        label,
                    }
                },
            );
        }
        KeyOutcome::Pin => pin_selected(state),
        KeyOutcome::Unpin => unpin_selected(state),
        KeyOutcome::UploadAdd => {
            let (local_path, gist_id) = if state.is_pin_diff_context() {
                let Some(gist_id) = state.download_gist_id.clone() else {
                    return Ok(LoopFlow::Proceed);
                };
                (state.preview_local.clone(), gist_id)
            } else {
                // List-originated upload: reset any leftover Pins origin from an earlier
                // pin push (mirrors KeyOutcome::PreviewDiff's own reset).
                state.diff_return = Screen::List;
                let (Some(local), Some(gist)) = (state.selected_local(), state.selected_gist())
                else {
                    return Ok(LoopFlow::Proceed);
                };
                (local.path.clone(), gist.file.gist_id.clone())
            };
            let Some(filename) = upload_local_filename(&local_path) else {
                state.set_status("local file has no name");
                return Ok(LoopFlow::Proceed);
            };

            state.pending_action = Some(PendingAction::Upload {
                gist_id,
                filename: filename.clone(),
                local_path: local_path.clone(),
            });

            let local_label = format!("local: {}", crate::config::display_path(&local_path));
            let gist_label = "(new file)".to_string();
            match state.init_upload_state(&local_path, Some(String::new()), local_label, gist_label)
            {
                Ok(()) => state.screen = Screen::Confirm,
                Err(error) => {
                    state.pending_action = None;
                    state.set_status(format!(
                        "cannot read {}: {error}",
                        crate::config::display_path(&local_path)
                    ));
                }
            }
        }
        KeyOutcome::UploadPreview => {
            let (local_path, gist_id, gist_file) = if state.is_pin_diff_context() {
                let Some(gist_id) = state.download_gist_id.clone() else {
                    return Ok(LoopFlow::Proceed);
                };
                let local_path = state.preview_local.clone();
                let filename = state.download_gist_filename.clone().unwrap_or_default();
                let gist_file = state
                    .gists
                    .iter()
                    .find(|g| g.gist_id == gist_id && g.filename == filename)
                    .cloned()
                    .unwrap_or_else(|| GistFile::for_sync(gist_id.clone(), filename.clone(), None));
                (local_path, gist_id, gist_file)
            } else {
                // List-originated upload: reset any leftover Pins origin from an earlier
                // pin push (mirrors KeyOutcome::PreviewDiff's own reset).
                state.diff_return = Screen::List;
                let (Some(local), Some(gist)) = (state.selected_local(), state.selected_gist())
                else {
                    return Ok(LoopFlow::Proceed);
                };
                (
                    local.path.clone(),
                    gist.file.gist_id.clone(),
                    gist.file.clone(),
                )
            };
            let Some(filename) = upload_local_filename(&local_path) else {
                state.set_status("local file has no name");
                return Ok(LoopFlow::Proceed);
            };
            let raw_url = gist_file.raw_url.clone();
            let (local_label, gist_label) = diff_labels(Some(&local_path), &gist_file);

            spawn_bg(state, &mut channels.bg, "Loading diff…", move || {
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
        KeyOutcome::Upload => {
            let Some(PendingAction::Upload {
                gist_id,
                filename,
                local_path: _,
            }) = state.pending_action.clone()
            else {
                return Ok(LoopFlow::Proceed);
            };

            let upload_content = state.content_to_upload();

            // Generate unique temp directory in workspace
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let temp_dir = std::env::temp_dir().join(format!(".gistui_upload_{timestamp}"));

            if let Err(e) = std::fs::create_dir_all(&temp_dir) {
                state.set_status(format!("failed to create temp dir: {e}"));
                return Ok(LoopFlow::Proceed);
            }

            let temp_file_path = temp_dir.join(&filename);
            if let Err(e) = std::fs::write(&temp_file_path, &upload_content) {
                state.set_status(format!("failed to write temp file: {e}"));
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Ok(LoopFlow::Proceed);
            }

            let has_same_name = state
                .gists
                .iter()
                .any(|g| g.gist_id == gist_id && g.filename == filename);

            let plan = if has_same_name {
                let target = GistFile::for_sync(gist_id.clone(), filename.clone(), None);
                crate::actions::upload_command(&temp_file_path, &target)
            } else {
                crate::actions::upload_add_command(&temp_file_path, &gist_id)
            };

            // Return to wherever this upload was initiated from (List, or Pins for a pin
            // push) instead of always snapping to List (mirrors download()).
            let return_screen = state.diff_return;
            state.back_to_list();
            state.screen = return_screen;
            spawn_bg(state, &mut channels.bg, "Uploading…", move || {
                let result = crate::actions::execute_command(&plan)
                    .map(|_| ())
                    .map_err(|e| e.to_string());

                let _ = std::fs::remove_dir_all(&temp_dir);

                BgTaskOutcome::UploadReplace {
                    result,
                    gist_id,
                    filename,
                }
            });
        }
        KeyOutcome::EditUpload => {
            edit_upload_buffer(terminal, state, channels)?;
        }
        KeyOutcome::Create(public) => {
            let Some(PendingAction::Create { local_path }) = state.pending_action.clone() else {
                return Ok(LoopFlow::Proceed);
            };
            let description = state.description_input.to_string();
            let plan = crate::actions::create_command(&local_path, public, &description);

            spawn_bg(state, &mut channels.bg, "Creating gist…", move || {
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
        KeyOutcome::PreviewContent => {
            // A detail-view number key records the exact file in `preview_request`;
            // otherwise fall back to the file selected on the list.
            let key = match state.preview_request.take() {
                Some(key) => key,
                None => match state.selected_gist() {
                    Some(gist) => (gist.file.gist_id.clone(), gist.file.filename.clone()),
                    None => return Ok(LoopFlow::Proceed),
                },
            };
            if let Some(cached) = state.gist_content_cache.get(&key) {
                state.preview_title = format!("Preview: {} / {}", key.0, key.1);
                state.preview_gist_key = Some(key);
                state.diff_text = cached.clone();
                state.diff_scroll = 0;
                state.diff_hscroll = 0;
                state.status = None;
                state.screen = Screen::Preview;
            } else {
                let gist_id = key.0.clone();
                let filename = key.1.clone();
                let raw_url = state.gist_file_raw_url(&gist_id, &filename);
                let preview_title = format!("Preview: {gist_id} / {filename}");
                spawn_bg(state, &mut channels.bg, "Loading preview…", move || {
                    let result = fetch_gist_content(&gist_id, &filename, raw_url.as_deref());
                    BgTaskOutcome::PreviewContent {
                        result,
                        key,
                        preview_title,
                    }
                });
            }
        }
        KeyOutcome::RefreshPreview => {
            if let Some(key) = state.preview_gist_key.clone() {
                state.gist_content_cache.remove(&key);
                let gist_id = key.0.clone();
                let filename = key.1.clone();
                let raw_url = state.gist_file_raw_url(&gist_id, &filename);
                let preview_title = format!("Preview: {gist_id} / {filename}");
                spawn_bg(state, &mut channels.bg, "Loading preview…", move || {
                    let result = fetch_gist_content(&gist_id, &filename, raw_url.as_deref());
                    BgTaskOutcome::PreviewContent {
                        result,
                        key,
                        preview_title,
                    }
                });
            }
        }
        KeyOutcome::OpenBrowser => open_browser(state),
        KeyOutcome::OpenRepoUrl => open_repo_url(state),
        KeyOutcome::CopyGistUrl => copy_gist_url(state),
        KeyOutcome::CopyPreviewContent => copy_preview_content(state),
        KeyOutcome::EditLocal => edit_local(terminal, state)?,
        KeyOutcome::ExecuteDelete => {
            let Some(PendingAction::Delete { gist_id, .. }) = state.pending_action.clone() else {
                return Ok(LoopFlow::Proceed);
            };
            let plan = crate::actions::delete_command(&gist_id);
            state.back_to_list();

            spawn_bg(state, &mut channels.bg, "Deleting gist…", move || {
                let result = crate::actions::execute_command(&plan)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                BgTaskOutcome::DeleteGist { result, gist_id }
            });
        }
        KeyOutcome::ExecuteRemoveFile => {
            let Some(PendingAction::RemoveFile {
                gist_id, filename, ..
            }) = state.pending_action.clone()
            else {
                return Ok(LoopFlow::Proceed);
            };
            let plan = crate::actions::remove_file_command(&gist_id, &filename);
            state.back_to_list();

            spawn_bg(state, &mut channels.bg, "Removing file…", move || {
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
            }) = state.pending_action.clone()
            else {
                return Ok(LoopFlow::Proceed);
            };
            state.pending_action = None;
            state.screen = state.detail.compact_return_screen;

            spawn_bg(
                state,
                &mut channels.bg,
                "Compacting revisions…",
                move || {
                    let result =
                        crate::actions::execute_compact_gist(&gist_id).map_err(|e| e.to_string());
                    BgTaskOutcome::CompactGist {
                        result,
                        label,
                        count,
                    }
                },
            );
        }
        KeyOutcome::ApplyDescription => {
            let gist_id = state
                .detail
                .gist_id
                .clone()
                .or_else(|| state.selected_group().map(|g| g.id.clone()));
            let Some(gist_id) = gist_id else {
                state.editing_description = false;
                return Ok(LoopFlow::Proceed);
            };
            let description = state.description_input.to_string();
            let plan = crate::actions::edit_description_command(&gist_id, &description);
            state.editing_description = false;
            state.description_input.clear();

            spawn_bg(
                state,
                &mut channels.bg,
                "Updating description…",
                move || {
                    let result = crate::actions::execute_command(&plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    BgTaskOutcome::ApplyDescription { result, gist_id }
                },
            );
        }
        KeyOutcome::RefreshLocals => {
            let generation = state.begin_local_scan();
            state.set_status("Scanning files…");
            state.local_scanning = true;
            channels.local = Some(spawn_local_scan(
                generation,
                state.cwd.clone(),
                state.pinned.clone(),
                state.local_recursive,
                state.skip_dirs.clone(),
                state.scan_depth,
            ));
        }
        KeyOutcome::UnpinAtPin => unpin_at_pin_index(state),
        KeyOutcome::SyncSelectedPair => {
            let (Some(local), Some(gist)) = (state.selected_local(), state.selected_gist()) else {
                return Ok(LoopFlow::Proceed);
            };
            let local_abs = state.cwd.join(&local.path);
            let gist_id = gist.file.gist_id.clone();
            let filename = gist.file.filename.clone();
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
            match state.pin_sync_status(idx) {
                crate::domain::SyncStatus::Push => spawn_pin_push(state, &mut channels.bg, &m),
                crate::domain::SyncStatus::Pull => spawn_pin_pull(state, &mut channels.bg, &m),
                crate::domain::SyncStatus::InSync => state.set_status("already in sync"),
                crate::domain::SyncStatus::Missing => {
                    state.set_status("local file is missing — use d to pull it back")
                }
                crate::domain::SyncStatus::Unknown => {
                    state.set_status("can't tell which side is newer — use u to push or d to pull")
                }
            }
        }
        KeyOutcome::SyncPinPush => {
            if let Some(m) = selected_pin(state) {
                spawn_pin_push(state, &mut channels.bg, &m);
            }
        }
        KeyOutcome::SyncPinPull => {
            if let Some(m) = selected_pin(state) {
                spawn_pin_pull(state, &mut channels.bg, &m);
            }
        }
        KeyOutcome::SyncPinAuto => {
            let Some(pin_idx) = state.selected_pin_index() else {
                return Ok(LoopFlow::Proceed);
            };
            let m = state.pinned[pin_idx].clone();
            match state.pin_sync_status(pin_idx) {
                crate::domain::SyncStatus::InSync => state.set_status("already in sync"),
                crate::domain::SyncStatus::Pull => spawn_pin_pull(state, &mut channels.bg, &m),
                // The content-hash no-op check now happens upstream in
                // AppState::pin_sync_status (a matching hash is already reclassified to
                // InSync above), so a genuine Push here always means a real change.
                crate::domain::SyncStatus::Push => spawn_pin_push(state, &mut channels.bg, &m),
                crate::domain::SyncStatus::Missing => {
                    state.set_status("local file is missing — use d to pull it back")
                }
                crate::domain::SyncStatus::Unknown => {
                    state.set_status("can't tell which side is newer — use u to push or d to pull")
                }
            }
        }
        KeyOutcome::PreviewPinDiff => {
            if let Some(m) = selected_pin(state) {
                state.diff_return = Screen::Pins;
                spawn_pin_diff(state, &mut channels.bg, &m);
            }
        }
        KeyOutcome::PersistDiffContext => persist_diff_context(state),
        KeyOutcome::ThemeToggle => persist_theme(state),
        KeyOutcome::FetchRevisions => {
            let Some(gist_id) = state.revision.gist_id.clone() else {
                return Ok(LoopFlow::Proceed);
            };
            spawn_bg(state, &mut channels.bg, "Loading revisions…", move || {
                let result = crate::gh::fetch_gist_commits_json(&gist_id)
                    .map_err(|e| e.to_string())
                    .and_then(|raw| {
                        crate::gh::parse_gist_commits_json(&raw).map_err(|e| e.to_string())
                    });
                BgTaskOutcome::RevisionsFetched { gist_id, result }
            });
        }
        KeyOutcome::RevisionDiffIncremental => {
            let Some(gist_id) = state.revision.gist_id.clone() else {
                return Ok(LoopFlow::Proceed);
            };
            let Some(child) = state.selected_revision().cloned() else {
                return Ok(LoopFlow::Proceed);
            };
            let filename = state.revision.target_file.clone();
            let child_version = child.version.clone();
            let child_label = revision_version_label(&child);
            let parent = state
                .revision
                .entries
                .as_ref()
                .and_then(|entries| entries.get(state.revision.index + 1).cloned());
            let (parent_version, old_label) = match parent {
                Some(parent) => {
                    let label = revision_version_label(&parent);
                    (Some(parent.version), format!("revision {label}"))
                }
                None => (None, "(initial)".into()),
            };
            let new_label = format!("revision {child_label}");
            let owner_login = state.gist_owner_login(&gist_id);
            spawn_bg(state, &mut channels.bg, "Loading diff…", move || {
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
        KeyOutcome::RevisionDiff => {
            let Some(gist_id) = state.revision.gist_id.clone() else {
                return Ok(LoopFlow::Proceed);
            };
            let Some(revision) = state.selected_revision().cloned() else {
                return Ok(LoopFlow::Proceed);
            };
            let filename = state.revision.target_file.clone();
            let version = revision.version.clone();
            let version_label = revision_version_label(&revision);
            let old_label = format!("revision {version_label}");
            let new_label = format!("current {filename}");
            let raw_url = state.gist_file_raw_url(&gist_id, &filename);
            let owner_login = state.gist_owner_login(&gist_id);
            spawn_bg(state, &mut channels.bg, "Loading diff…", move || {
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
        KeyOutcome::RestoreRevisionPreview => {
            let Some(gist_id) = state.revision.gist_id.clone() else {
                return Ok(LoopFlow::Proceed);
            };
            let Some(revision) = state.selected_revision().cloned() else {
                return Ok(LoopFlow::Proceed);
            };
            let filename = state.revision.target_file.clone();
            let version = revision.version.clone();
            let version_label = revision_version_label(&revision);
            let raw_url = state.gist_file_raw_url(&gist_id, &filename);
            let owner_login = state.gist_owner_login(&gist_id);
            spawn_bg(state, &mut channels.bg, "Loading revision…", move || {
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
            }) = state.pending_action.clone()
            else {
                return Ok(LoopFlow::Proceed);
            };
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let temp_dir = std::env::temp_dir().join(format!(".gistui_restore_{timestamp}"));
            if let Err(e) = std::fs::create_dir_all(&temp_dir) {
                state.set_status(format!("failed to create temp dir: {e}"));
                return Ok(LoopFlow::Proceed);
            }
            let json_path = temp_dir.join("restore.json");
            let body = crate::actions::restore_revision_json(&filename, &content);
            if let Err(e) = std::fs::write(&json_path, &body) {
                state.set_status(format!("failed to write restore payload: {e}"));
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Ok(LoopFlow::Proceed);
            }
            let plan = crate::actions::restore_revision_command(&gist_id, &json_path);
            spawn_bg(
                state,
                &mut channels.bg,
                "Restoring revision…",
                move || {
                    let result = crate::actions::execute_command(&plan)
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    let _ = std::fs::remove_dir_all(&temp_dir);
                    BgTaskOutcome::RestoreRevisionDone {
                        result,
                        gist_id,
                        filename,
                    }
                },
            );
        }
        KeyOutcome::ToggleGistStar => {
            let Some(gist_id) = state.context_gist_id() else {
                state.set_status("select a gist first");
                return Ok(LoopFlow::Proceed);
            };
            let starring = !state.gist_is_starred(&gist_id);
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
            spawn_bg(state, &mut channels.bg, msg, move || {
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
        KeyOutcome::ForkGist => {
            let Some(gist_id) = state.context_gist_id() else {
                state.set_status("select a gist to fork");
                return Ok(LoopFlow::Proceed);
            };
            if state.gist_is_owned(&gist_id) {
                state.set_status("already yours — no fork needed");
                return Ok(LoopFlow::Proceed);
            }
            let plan = crate::actions::fork_gist_command(&gist_id);
            spawn_bg(state, &mut channels.bg, "Forking…", move || {
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
