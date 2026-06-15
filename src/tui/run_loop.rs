use super::*;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

pub(super) fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut state = load_startup_state()?;
    let mut gist_rx = Some(spawn_gist_fetch());
    let mut local_rx: Option<std::sync::mpsc::Receiver<Vec<LocalCandidate>>> = None;
    let mut bg_rx: Option<std::sync::mpsc::Receiver<BgTaskOutcome>> = None;

    loop {
        terminal.draw(|frame| render(frame, &state))?;
        // Advance the spinner once per iteration; the poll below caps the loop at ~150ms, so
        // in-progress states (scanning/loading/working) animate even with no input.
        state.spinner_frame = state.spinner_frame.wrapping_add(1);

        // Absorb the background gist list once it arrives.
        if state.loading {
            if let Some(ref rx) = gist_rx {
                if let Ok((gists, comment_counts)) = rx.try_recv() {
                    cache_gists(&gists);
                    state.gists = gists;
                    state.gist_comment_counts = comment_counts;
                    state.loading = false;
                    if state.gist_index >= state.ranked_gists().len() {
                        state.gist_index = 0;
                    }
                    let count = state.visible_gist_groups().len();
                    if count > 0 && state.gists_index >= count {
                        state.gists_index = count - 1;
                    }
                    gist_rx = None;
                }
            }
        }

        // Absorb a completed background local scan.
        if state.local_scanning {
            if let Some(ref rx) = local_rx {
                if let Ok(locals) = rx.try_recv() {
                    let selected = state.selected_local().map(|c| c.path.clone());
                    state.locals = locals;
                    state.local_index = selected
                        .and_then(|path| state.locals.iter().position(|c| c.path == path))
                        .unwrap_or(0)
                        .min(state.locals.len().saturating_sub(1));
                    if state.gist_index >= state.ranked_gists().len() {
                        state.gist_index = 0;
                    }
                    state.local_scanning = false;
                    state.status = None;
                    local_rx = None;
                }
            }
        }

        // Absorb a completed background per-action task.
        if let Some(ref rx) = bg_rx {
            if let Ok(outcome) = rx.try_recv() {
                state.bg_task_msg = None;
                bg_rx = None;
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
                            match local_path.as_ref().map(std::fs::read_to_string).transpose() {
                                Ok(local) => {
                                    let local_content = local.unwrap_or_default();
                                    let diff = preview_diff_text(
                                        upload_orientation,
                                        &local_label,
                                        &local_content,
                                        &gist_label,
                                        &remote,
                                    );
                                    let identical = local_content == remote;
                                    state.enter_diff(
                                        diff,
                                        remote,
                                        local_path.unwrap_or_default(),
                                        target,
                                    );
                                    state.diff_identical = identical;
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
                                match std::fs::read_to_string(&target) {
                                    Ok(local_content) => {
                                        let diff = crate::diff::unified_diff(
                                            &local_label,
                                            &local_content,
                                            &gist_label,
                                            &remote,
                                        );
                                        let identical = local_content == remote;
                                        state.download_gist_id = Some(gist_id);
                                        state.download_gist_filename = Some(filename);
                                        state.enter_diff(diff, remote, target.clone(), target);
                                        state.diff_identical = identical;
                                    }
                                    Err(error) => state.set_status(format!("read failed: {error}")),
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
                                            &mut state,
                                            &target,
                                            &gist_id,
                                            &filename,
                                            &remote,
                                            crate::domain::SyncDirection::Download,
                                        );
                                        refresh_locals(&mut state);
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
                            state.init_upload_state(
                                &local_path,
                                Some(remote),
                                local_label,
                                gist_label,
                            );
                            state.diff_scroll = 0;
                            state.diff_hscroll = 0;
                            state.status = None;
                            state.screen = Screen::Confirm;
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
                                    &mut state,
                                    &local_path,
                                    &gist_id,
                                    &filename,
                                    &content,
                                    crate::domain::SyncDirection::Upload,
                                );
                            }
                            state.back_to_list();
                            state.loading = true;
                            gist_rx = Some(spawn_gist_fetch());
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
                            gist_rx = Some(spawn_gist_fetch());
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
                            gist_rx = Some(spawn_gist_fetch());
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
                            gist_rx = Some(spawn_gist_fetch());
                        }
                        Err(error) => state.set_status(format!("remove failed: {error}")),
                    },
                    BgTaskOutcome::ApplyDescription { result, gist_id } => match result {
                        Ok(_) => {
                            state.set_status(format!("Updated description for gist {gist_id}"));
                            state.loading = true;
                            gist_rx = Some(spawn_gist_fetch());
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
                            gist_rx = Some(spawn_gist_fetch());
                        }
                        Err(error) => state.set_status(format!("compact failed: {error}")),
                    },
                    BgTaskOutcome::CommentsFetched { gist_id, result } => {
                        state.apply_fetched_comments(&gist_id, result);
                    }
                }
            }
        }

        // Poll so the loop also wakes to check the background fetches, not only on input.
        if !event::poll(std::time::Duration::from_millis(150))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            // Windows reports both Press and Release (and Repeat) for each
            // keystroke, while Unix terminals report only Press. Without this
            // filter every key fires twice on Windows — Tab toggles focus back
            // to where it started and Up/Down jump two rows. See ratatui#347.
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if state.bg_task_msg.is_some() {
                if key.code == KeyCode::Esc {
                    state.bg_task_msg = None;
                    bg_rx = None;
                    state.set_status("Cancelled");
                }
                continue;
            }

            match state.handle_key(key.code) {
                KeyOutcome::Quit => break,
                KeyOutcome::PreviewDiff => {
                    let Some(ranked) = state.selected_gist() else {
                        continue;
                    };
                    // List-originated diff returns to the List on Esc (reset any
                    // leftover Pins origin from an earlier pin diff).
                    state.diff_return = Screen::List;
                    let local_path = state.selected_local().map(|local| local.path.clone());
                    let gist = ranked.file.clone();
                    let gist_id = gist.gist_id.clone();
                    let filename = gist.filename.clone();
                    let (local_label, gist_label) = diff_labels(local_path.as_deref(), &gist);
                    let target = state.cwd.join(&filename);
                    let upload_orientation = state.focus == FocusPane::Local;

                    spawn_bg(&mut state, &mut bg_rx, "Loading diff…", move || {
                        let result = crate::gh::fetch_gist_file_content(&gist_id, &filename)
                            .map_err(|e| e.to_string());
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
                KeyOutcome::Download => download(&mut state),
                KeyOutcome::DownloadGist => {
                    let Some(ranked) = state.selected_gist() else {
                        continue;
                    };
                    let gist = ranked.file.clone();
                    let gist_id = gist.gist_id.clone();
                    let filename = gist.filename.clone();
                    let target = state.cwd.join(&filename);
                    let (local_label, gist_label) = diff_labels(Some(&target), &gist);

                    spawn_bg(&mut state, &mut bg_rx, "Downloading…", move || {
                        let result = crate::gh::fetch_gist_file_content(&gist_id, &filename)
                            .map_err(|e| e.to_string());
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
                        continue;
                    };
                    let gist_id = group.id.clone();
                    state.screen = Screen::GistDetail;
                    state.detail_gist_id = Some(gist_id.clone());
                    state.detail_comments = None;
                    state.detail_comments_error = None;
                    state.detail_scroll = 0;
                    state.detail_focus = DetailFocus::Files;
                    state.detail_file_cursor = 0;

                    let fetch_id = gist_id.clone();
                    spawn_bg(&mut state, &mut bg_rx, "Loading comments…", move || {
                        let result = crate::gh::fetch_gist_comments_json(&fetch_id)
                            .map_err(|e| e.to_string())
                            .and_then(|raw| {
                                crate::gh::parse_gist_comments_json(&raw).map_err(|e| e.to_string())
                            });
                        BgTaskOutcome::CommentsFetched {
                            gist_id: fetch_id,
                            result,
                        }
                    });
                }
                KeyOutcome::CompactGist => {
                    let Some(gist_id) = state.context_gist_id() else {
                        continue;
                    };
                    let Some(group) = state.group_by_id(&gist_id) else {
                        continue;
                    };
                    let label = if group.description.trim().is_empty() {
                        group.id.clone()
                    } else {
                        group.description.clone()
                    };

                    spawn_bg(&mut state, &mut bg_rx, "Checking revisions…", move || {
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
                KeyOutcome::Pin => pin_selected(&mut state),
                KeyOutcome::Unpin => unpin_selected(&mut state),
                KeyOutcome::UploadAdd => {
                    let (Some(local), Some(gist)) = (state.selected_local(), state.selected_gist())
                    else {
                        continue;
                    };
                    let local_path = local.path.clone();
                    let gist_id = gist.file.gist_id.clone();
                    let Some(filename) = upload_local_filename(&local_path) else {
                        state.set_status("local file has no name");
                        continue;
                    };

                    state.pending_action = Some(PendingAction::Upload {
                        gist_id,
                        filename: filename.clone(),
                        local_path: local_path.clone(),
                    });

                    let local_label =
                        format!("local: {}", crate::config::display_path(&local_path));
                    let gist_label = "(new file)".to_string();
                    state.init_upload_state(
                        &local_path,
                        Some(String::new()),
                        local_label,
                        gist_label,
                    );
                    state.screen = Screen::Confirm;
                }
                KeyOutcome::UploadPreview => {
                    let (Some(local), Some(gist)) = (state.selected_local(), state.selected_gist())
                    else {
                        continue;
                    };
                    let Some(filename) = upload_local_filename(&local.path) else {
                        state.set_status("local file has no name");
                        continue;
                    };
                    let gist_id = gist.file.gist_id.clone();
                    let gist_file = gist.file.clone();
                    let local_path = local.path.clone();
                    let (local_label, gist_label) = diff_labels(Some(&local_path), &gist_file);

                    spawn_bg(&mut state, &mut bg_rx, "Loading diff…", move || {
                        let result = crate::gh::fetch_gist_file_content(&gist_id, &filename)
                            .map_err(|e| e.to_string());
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
                        continue;
                    };

                    let upload_content = state.content_to_upload();

                    // Generate unique temp directory in workspace
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    let temp_dir = state.cwd.join(format!(".gistui_upload_{timestamp}"));

                    if let Err(e) = std::fs::create_dir_all(&temp_dir) {
                        state.set_status(format!("failed to create temp dir: {e}"));
                        continue;
                    }

                    let temp_file_path = temp_dir.join(&filename);
                    if let Err(e) = std::fs::write(&temp_file_path, &upload_content) {
                        state.set_status(format!("failed to write temp file: {e}"));
                        let _ = std::fs::remove_dir_all(&temp_dir);
                        continue;
                    }

                    let has_same_name = state
                        .gists
                        .iter()
                        .any(|g| g.gist_id == gist_id && g.filename == filename);

                    let plan = if has_same_name {
                        let target = GistFile {
                            gist_id: gist_id.clone(),
                            description: String::new(),
                            filename: filename.clone(),
                            public: false,
                            updated_at: String::new(),
                            created_at: String::new(),
                        };
                        crate::actions::upload_command(&temp_file_path, &target)
                    } else {
                        crate::actions::upload_add_command(&temp_file_path, &gist_id)
                    };

                    state.back_to_list();
                    spawn_bg(&mut state, &mut bg_rx, "Uploading…", move || {
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
                    edit_upload_buffer(terminal, &mut state)?;
                }
                KeyOutcome::Create(public) => {
                    let Some(PendingAction::Create { local_path }) = state.pending_action.clone()
                    else {
                        continue;
                    };
                    let description = state.description_input.clone();
                    let plan = crate::actions::create_command(&local_path, public, &description);

                    spawn_bg(&mut state, &mut bg_rx, "Creating gist…", move || {
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
                            None => continue,
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
                        let preview_title = format!("Preview: {gist_id} / {filename}");
                        spawn_bg(&mut state, &mut bg_rx, "Loading preview…", move || {
                            let result = crate::gh::fetch_gist_file_content(&gist_id, &filename)
                                .map_err(|e| e.to_string());
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
                        let preview_title = format!("Preview: {gist_id} / {filename}");
                        spawn_bg(&mut state, &mut bg_rx, "Loading preview…", move || {
                            let result = crate::gh::fetch_gist_file_content(&gist_id, &filename)
                                .map_err(|e| e.to_string());
                            BgTaskOutcome::PreviewContent {
                                result,
                                key,
                                preview_title,
                            }
                        });
                    }
                }
                KeyOutcome::OpenBrowser => open_browser(&mut state),
                KeyOutcome::CopyGistUrl => copy_gist_url(&mut state),
                KeyOutcome::CopyPreviewContent => copy_preview_content(&mut state),
                KeyOutcome::EditLocal => edit_local(terminal, &mut state)?,
                KeyOutcome::ExecuteDelete => {
                    let Some(PendingAction::Delete { gist_id, .. }) = state.pending_action.clone()
                    else {
                        continue;
                    };
                    let plan = crate::actions::delete_command(&gist_id);
                    state.back_to_list();

                    spawn_bg(&mut state, &mut bg_rx, "Deleting gist…", move || {
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
                        continue;
                    };
                    let plan = crate::actions::remove_file_command(&gist_id, &filename);
                    state.back_to_list();

                    spawn_bg(&mut state, &mut bg_rx, "Removing file…", move || {
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
                        continue;
                    };
                    state.pending_action = None;
                    state.screen = state.compact_return_screen;

                    spawn_bg(
                        &mut state,
                        &mut bg_rx,
                        "Compacting revisions…",
                        move || {
                            let result = crate::actions::execute_compact_gist(&gist_id)
                                .map_err(|e| e.to_string());
                            BgTaskOutcome::CompactGist {
                                result,
                                label,
                                count,
                            }
                        },
                    );
                }
                KeyOutcome::ApplyDescription => {
                    let Some(group) = state.selected_group() else {
                        state.editing_description = false;
                        continue;
                    };
                    let gist_id = group.id.clone();
                    let description = state.description_input.clone();
                    let plan = crate::actions::edit_description_command(&gist_id, &description);
                    state.editing_description = false;
                    state.description_input.clear();

                    spawn_bg(
                        &mut state,
                        &mut bg_rx,
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
                    state.set_status("Scanning files…");
                    state.local_scanning = true;
                    local_rx = Some(spawn_local_scan(
                        state.cwd.clone(),
                        state.pinned.clone(),
                        state.local_recursive,
                        state.skip_dirs.clone(),
                        state.scan_depth,
                    ));
                }
                KeyOutcome::UnpinAtPin => unpin_at_pin_index(&mut state),
                KeyOutcome::SyncSelectedPair => {
                    let (Some(local), Some(gist)) = (state.selected_local(), state.selected_gist())
                    else {
                        continue;
                    };
                    let local_abs = state.cwd.join(&local.path);
                    let gist_id = gist.file.gist_id.clone();
                    let filename = gist.file.filename.clone();
                    let idx = state.pinned.iter().position(|m| {
                        pin_local_abs(&state, m) == local_abs
                            && m.gist_id == gist_id
                            && m.gist_filename == filename
                    });
                    let Some(idx) = idx else {
                        state.set_status("pair is not pinned — press p to pin first");
                        continue;
                    };
                    let m = state.pinned[idx].clone();
                    match state.pin_sync_status(idx) {
                        crate::domain::SyncStatus::Push => {
                            spawn_pin_push(&mut state, &mut bg_rx, &m)
                        }
                        crate::domain::SyncStatus::Pull => {
                            spawn_pin_pull(&mut state, &mut bg_rx, &m)
                        }
                        crate::domain::SyncStatus::InSync => state.set_status("already in sync"),
                        crate::domain::SyncStatus::Unknown => state.set_status(
                            "can't tell which side is newer — use u to push or d to pull",
                        ),
                    }
                }
                KeyOutcome::SyncPinPush => {
                    if let Some(m) = selected_pin(&state) {
                        spawn_pin_push(&mut state, &mut bg_rx, &m);
                    }
                }
                KeyOutcome::SyncPinPull => {
                    if let Some(m) = selected_pin(&state) {
                        spawn_pin_pull(&mut state, &mut bg_rx, &m);
                    }
                }
                KeyOutcome::SyncPinAuto => {
                    let Some(pin_idx) = state.selected_pin_index() else {
                        continue;
                    };
                    let m = state.pinned[pin_idx].clone();
                    match state.pin_sync_status(pin_idx) {
                        crate::domain::SyncStatus::InSync => state.set_status("already in sync"),
                        crate::domain::SyncStatus::Pull => {
                            spawn_pin_pull(&mut state, &mut bg_rx, &m)
                        }
                        crate::domain::SyncStatus::Push => {
                            // Cheap, network-free no-op check: if the local file still
                            // hashes to last_seen_hash, the newer mtime is a touch with
                            // no content change. Note: only fires for plain pushes — a
                            // push whose baseline was a JSON-transformed/redacted upload
                            // won't match the raw file, so it harmlessly falls through to
                            // a full push.
                            let local_abs = pin_local_abs(&state, &m);
                            let unchanged = m.last_seen_hash.as_deref().is_some_and(|baseline| {
                                std::fs::read(&local_abs)
                                    .map(|b| crate::domain::sha256_hex(&b) == baseline)
                                    .unwrap_or(false)
                            });
                            if unchanged {
                                state.set_status("already in sync");
                            } else {
                                spawn_pin_push(&mut state, &mut bg_rx, &m);
                            }
                        }
                        crate::domain::SyncStatus::Unknown => state.set_status(
                            "can't tell which side is newer — use u to push or d to pull",
                        ),
                    }
                }
                KeyOutcome::PreviewPinDiff => {
                    if let Some(m) = selected_pin(&state) {
                        state.diff_return = Screen::Pins;
                        spawn_pin_diff(&mut state, &mut bg_rx, &m);
                    }
                }
                KeyOutcome::PersistDiffContext => persist_diff_context(&mut state),
                KeyOutcome::None => {}
            }
        }
    }

    Ok(())
}

enum BgTaskOutcome {
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
    CommentsFetched {
        gist_id: String,
        result: Result<Vec<GistComment>, String>,
    },
}

fn cache_gists(gists: &[GistFile]) {
    if let Ok(path) = crate::cache::cache_path() {
        crate::cache::save_cached_gists(&path, gists);
    }
}

/// Fetches the gist list on a background thread so startup does not block on `gh`.
/// Mirrors the previous graceful degradation: an empty list on any error.
fn spawn_gist_fetch(
) -> std::sync::mpsc::Receiver<(Vec<GistFile>, std::collections::HashMap<String, u32>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = if crate::gh::check_gh_ready().is_ok() {
            crate::gh::fetch_gist_list_json()
                .map(|raw| {
                    let files = crate::gh::parse_gist_list_json(&raw).unwrap_or_default();
                    let comment_counts =
                        crate::gh::parse_gist_comment_counts(&raw).unwrap_or_default();
                    (files, comment_counts)
                })
                .unwrap_or_default()
        } else {
            Default::default()
        };
        let _ = tx.send(result);
    });
    rx
}

fn spawn_local_scan(
    cwd: std::path::PathBuf,
    pinned: Vec<crate::domain::PinnedMapping>,
    recursive: bool,
    skip_dirs: Vec<String>,
    max_depth: u32,
) -> std::sync::mpsc::Receiver<Vec<LocalCandidate>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let candidates = crate::local::discover_local_candidates(
            &cwd, &pinned, recursive, &skip_dirs, max_depth,
        )
        .unwrap_or_default();
        let _ = tx.send(candidates);
    });
    rx
}

type BgRx = Option<std::sync::mpsc::Receiver<BgTaskOutcome>>;

/// Run `work` on a background thread, wiring its result channel into `bg_rx` and setting
/// the in-progress `bg_task_msg` the main loop renders. The worker's returned
/// [`BgTaskOutcome`] is sent back for the loop to drain. Encapsulates the channel +
/// thread + status boilerplate every async action otherwise repeats by hand.
fn spawn_bg<F>(state: &mut AppState, bg_rx: &mut BgRx, msg: impl Into<String>, work: F)
where
    F: FnOnce() -> BgTaskOutcome + Send + 'static,
{
    state.bg_task_msg = Some(msg.into());
    let (tx, rx) = std::sync::mpsc::channel();
    *bg_rx = Some(rx);
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
}

/// The pin currently selected in the Pins screen, if any.
fn selected_pin(state: &AppState) -> Option<crate::domain::PinnedMapping> {
    state
        .selected_pin_index()
        .and_then(|i| state.pinned.get(i).cloned())
}

/// Resolve a pin's absolute local path against cwd.
fn pin_local_abs(state: &AppState, m: &crate::domain::PinnedMapping) -> PathBuf {
    if m.local_path.is_absolute() {
        m.local_path.clone()
    } else {
        state.cwd.join(&m.local_path)
    }
}

/// Spawn the push (upload local → gist) flow for a pin: lands in the existing
/// upload `Screen::Confirm` diff.
fn spawn_pin_push(state: &mut AppState, bg_rx: &mut BgRx, m: &crate::domain::PinnedMapping) {
    let local_path = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    state.pending_action = Some(PendingAction::Upload {
        gist_id: gist_id.clone(),
        filename: filename.clone(),
        local_path: local_path.clone(),
    });
    let gist_file = GistFile {
        gist_id: gist_id.clone(),
        description: String::new(),
        filename: filename.clone(),
        public: false,
        updated_at: String::new(),
        created_at: String::new(),
    };
    let (local_label, gist_label) = diff_labels(Some(&local_path), &gist_file);
    spawn_bg(state, bg_rx, "Loading diff…", move || {
        let result =
            crate::gh::fetch_gist_file_content(&gist_id, &filename).map_err(|e| e.to_string());
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
fn spawn_pin_pull(state: &mut AppState, bg_rx: &mut BgRx, m: &crate::domain::PinnedMapping) {
    let target = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    let gist_file = GistFile {
        gist_id: gist_id.clone(),
        description: String::new(),
        filename: filename.clone(),
        public: false,
        updated_at: String::new(),
        created_at: String::new(),
    };
    let (local_label, gist_label) = diff_labels(Some(&target), &gist_file);
    spawn_bg(state, bg_rx, "Downloading…", move || {
        let result =
            crate::gh::fetch_gist_file_content(&gist_id, &filename).map_err(|e| e.to_string());
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
fn spawn_pin_diff(state: &mut AppState, bg_rx: &mut BgRx, m: &crate::domain::PinnedMapping) {
    let local_abs = pin_local_abs(state, m);
    let gist_id = m.gist_id.clone();
    let filename = m.gist_filename.clone();
    let gist_file = GistFile {
        gist_id: gist_id.clone(),
        description: String::new(),
        filename: filename.clone(),
        public: false,
        updated_at: String::new(),
        created_at: String::new(),
    };
    let (local_label, gist_label) = diff_labels(Some(&local_abs), &gist_file);
    let target = local_abs.clone();
    spawn_bg(state, bg_rx, "Loading diff…", move || {
        let result =
            crate::gh::fetch_gist_file_content(&gist_id, &filename).map_err(|e| e.to_string());
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
fn record_pin_sync(
    state: &mut AppState,
    local_abs: &std::path::Path,
    gist_id: &str,
    filename: &str,
    content: &str,
    direction: crate::domain::SyncDirection,
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
fn open_browser(state: &mut AppState) {
    let gist_id = state.context_gist_id();
    let Some(gist_id) = gist_id else {
        return;
    };
    let plan = crate::actions::open_browser_command(&gist_id);
    match crate::actions::execute_command(&plan) {
        Ok(_) => state.set_status(format!("Opened gist {gist_id} in the browser")),
        Err(error) => state.set_status(format!("open failed: {error}")),
    }
}

/// Copies the context gist's web URL to the system clipboard. On the Preview screen the
/// URL comes from the previewed file's gist; elsewhere from the current selection.
fn copy_gist_url(state: &mut AppState) {
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
fn copy_preview_content(state: &mut AppState) {
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

/// Opens the selected local file in `$VISUAL`/`$EDITOR` (default `vi`). A terminal editor
/// needs the full terminal, so the TUI leaves raw mode / the alternate screen for the
/// duration and restores afterwards. `$EDITOR` may include flags (e.g. `code --wait`).
fn edit_local(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
) -> Result<()> {
    let Some(local) = state.selected_local() else {
        return Ok(());
    };
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let Some(program) = parts.next() else {
        state.set_status("no editor configured (set $EDITOR)");
        return Ok(());
    };
    let args: Vec<&str> = parts.collect();

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    let result = std::process::Command::new(program)
        .args(&args)
        .arg(&local.path)
        .status();
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
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

fn edit_upload_buffer(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
) -> Result<()> {
    let Some(local_path) = state.upload_local_path() else {
        return Ok(());
    };
    let Some(filename) = local_path.file_name().and_then(|n| n.to_str()) else {
        return Ok(());
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_file_path =
        std::env::temp_dir().join(format!(".gistui_redact_{timestamp}_{filename}"));

    let current_content = state.content_to_upload();
    if let Err(e) = std::fs::write(&temp_file_path, &current_content) {
        state.set_status(format!("failed to write temp file: {e}"));
        return Ok(());
    }

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let Some(program) = parts.next() else {
        state.set_status("no editor configured (set $EDITOR)");
        let _ = std::fs::remove_file(&temp_file_path);
        return Ok(());
    };
    let args: Vec<&str> = parts.collect();

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    let result = std::process::Command::new(program)
        .args(&args)
        .arg(&temp_file_path)
        .status();
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.clear()?;

    match result {
        Ok(_) => match std::fs::read_to_string(&temp_file_path) {
            Ok(edited_content) => {
                state.upload_edited_content = Some(edited_content);
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

fn download(state: &mut AppState) {
    let target = state.download_target.clone();
    let content = state.preview_remote.clone();
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
                    crate::domain::SyncDirection::Download,
                );
            }
            state.back_to_list();
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
fn refresh_locals(state: &mut AppState) {
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
fn persist_diff_context(state: &mut AppState) {
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

fn pin_selected(state: &mut AppState) {
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

fn unpin_selected(state: &mut AppState) {
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

fn unpin_at_pin_index(state: &mut AppState) {
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
            state.pins_index = state
                .pins_index
                .min(state.visible_pin_indices().len().saturating_sub(1));
            refresh_locals(state);
            state.set_status(format!("Unpinned {label}"));
        }
        Err(error) => state.set_status(format!("unpin failed: {error}")),
    }
}

fn upload_local_filename(local: &std::path::Path) -> Option<String> {
    local.file_name().and_then(|n| n.to_str()).map(String::from)
}
