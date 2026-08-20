//! `Screen::Confirm` — key handling, view-model, paint, and apply handlers colocated in
//! one file (issue #287, Phase 2; issue #383).

use crate::tui::bg::LoopFlow;
use crate::tui::view_model::{ConfirmBackgroundVm, ConfirmModalKind, ConfirmVm};
use crate::tui::{AppState, HelpTopic, KeyOutcome, MouseLayout, PendingAction, Screen};
use crossterm::event::KeyCode;
use ratatui::{style::Color, Frame};
use std::path::PathBuf;

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::List;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    3
}

impl AppState {
    pub(crate) fn handle_key_confirm(&mut self, code: KeyCode) -> KeyOutcome {
        // While typing the create flow's description, arrows drive the text cursor (handled
        // below), not the background diff scroll.
        match self.pending_action().cloned() {
            Some(PendingAction::Download) => match code {
                KeyCode::Char('y') => {
                    return KeyOutcome::Download {
                        mode: crate::actions::DownloadMode::overwrite_after_user_confirm(),
                    };
                }
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.cancel_confirm_to_diff();
                }
                _ => {}
            },
            Some(PendingAction::Upload { ref local_path, .. }) => match code {
                KeyCode::Char('y') if self.upload.watching => {
                    self.set_status("editor still open — finish editing first");
                }
                KeyCode::Char('y') => return KeyOutcome::Upload,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    // Return to wherever the upload was initiated from (List, or Pins for
                    // a pin push) instead of always snapping back to List.
                    self.cancel_confirm();
                    // The background watch thread (if any) is not force-killed — it cleans
                    // itself up once the editor closes. Reset the flag now so a stale
                    // late-arriving event (see AppState::apply_upload_edit_event) doesn't
                    // matter, and so a future upload-edit session isn't blocked by it.
                    self.upload.watching = false;
                }
                KeyCode::Char('e') if self.upload.watching => {
                    self.set_status("editor already open");
                }
                KeyCode::Char('e') => return KeyOutcome::EditUpload,
                KeyCode::Char('p') if crate::tui::render::is_json_file(local_path) => {
                    self.upload.json_pretty = !self.upload.json_pretty;
                    self.update_upload_diff();
                }
                KeyCode::Char('s') if crate::tui::render::is_json_file(local_path) => {
                    self.upload.json_sort = !self.upload.json_sort;
                    self.update_upload_diff();
                }
                _ => {}
            },
            Some(PendingAction::Create { .. }) if self.editing_description => match code {
                // Step 1: type the optional description. Enter advances to the
                // visibility choice; Esc cancels the whole create.
                KeyCode::Enter => self.editing_description = false,
                KeyCode::Esc => {
                    self.editing_description = false;
                    self.description_input.clear();
                    self.back_to_list();
                }
                _ => {
                    self.description_input.apply_edit(code);
                }
            },
            Some(PendingAction::Create { .. }) => match code {
                // Step 2: choose visibility (the description is kept in description_input).
                KeyCode::Char('s') => return KeyOutcome::Create(false),
                KeyCode::Char('p') => return KeyOutcome::Create(true),
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.description_input.clear();
                    self.back_to_list();
                }
                _ => {}
            },
            Some(PendingAction::Delete { .. }) => match code {
                KeyCode::Char('y') => return KeyOutcome::ExecuteDelete,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.cancel_confirm();
                }
                _ => {}
            },
            Some(PendingAction::RemoveFile { .. }) => match code {
                KeyCode::Char('y') => return KeyOutcome::ExecuteRemoveFile,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.back_to_list();
                }
                _ => {}
            },
            Some(PendingAction::CompactGist { .. }) => match code {
                KeyCode::Char('y') => return KeyOutcome::ExecuteCompactGist,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    // Return to whichever screen launched the compaction (Gists or GistDetail).
                    self.cancel_confirm();
                }
                _ => {}
            },
            Some(PendingAction::RestoreRevision { .. }) => match code {
                KeyCode::Char('y') => return KeyOutcome::ExecuteRestoreRevision,
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.cancel_confirm();
                    if !self.screen.is_revisions() {
                        self.screen = Screen::Revisions(Box::default());
                    }
                }
                _ => {}
            },
            _ => {
                if matches!(code, KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q')) {
                    self.cancel_confirm();
                }
            }
        }
        KeyOutcome::None
    }
}

pub(crate) fn build_confirm_vm(state: &AppState) -> ConfirmVm {
    let (title, border) = confirm_modal_style(state);
    let kind = if matches!(state.pending_action(), Some(PendingAction::Create { .. }))
        && state.editing_description
    {
        ConfirmModalKind::DescriptionInput {
            prefix: crate::tui::render::CREATE_DESC_PREFIX,
            input: state.description_input.clone(),
            suffix: crate::tui::render::CREATE_DESC_SUFFIX,
        }
    } else {
        ConfirmModalKind::Prompt {
            text: crate::tui::view_model::confirm_prompt(state),
        }
    };
    let background = match state.pending_action() {
        Some(PendingAction::CompactGist { gist_id, .. }) => {
            match crate::tui::view_model::build_compact_gist_bg_vm(state, gist_id) {
                Some(bg) => ConfirmBackgroundVm::CompactGist(bg),
                None => ConfirmBackgroundVm::Empty,
            }
        }
        _ => ConfirmBackgroundVm::Diff(crate::tui::screens::diff::build_diff_vm(state)),
    };
    ConfirmVm {
        title,
        border,
        kind,
        background,
    }
}

/// Title and border colour for the confirm modal. Destructive actions are tinted with the
/// theme's `del_color` so the stakes read at a glance; non-destructive writes use the neutral
/// `notice_color` prompt.
pub(crate) fn confirm_modal_style(state: &AppState) -> (&'static str, Color) {
    let theme = &state.theme;
    match state.pending_action() {
        Some(PendingAction::Create { .. }) if state.editing_description => {
            ("Description", theme.accent)
        }
        Some(PendingAction::Create { .. }) => ("Create gist", theme.notice_color),
        Some(PendingAction::Upload { .. }) => ("Upload", theme.notice_color),
        Some(PendingAction::Delete { .. }) => ("Delete", theme.del_color),
        Some(PendingAction::RemoveFile { .. }) => ("Remove file", theme.del_color),
        Some(PendingAction::CompactGist { .. }) => ("Compact revisions", theme.del_color),
        Some(PendingAction::RestoreRevision { .. }) => ("Restore revision", theme.notice_color),
        _ => ("Overwrite", theme.del_color),
    }
}

/// `Screen::Confirm`: the diff fills the screen as context behind a centered prompt modal,
/// keeping the overwrite gate's diff visible while the question is asked front-and-centre.
/// #72 audit: this modal intentionally does not surface `state.status`. It is a transient y/n
/// gate — confirming executes the action and transitions to `List`/`Gists`, where the result
/// status is shown; cancelling returns to the launching screen without setting a status here.
pub(crate) fn render_confirm_vm(
    frame: &mut Frame,
    state: &AppState,
    confirm: &ConfirmVm,
    chrome: &crate::tui::view_model::ChromeVm,
    layout: &mut MouseLayout,
) {
    match &confirm.background {
        ConfirmBackgroundVm::CompactGist(bg) => {
            crate::tui::render::render_compact_gist_bg_vm(frame, frame.area(), bg, &state.theme);
        }
        ConfirmBackgroundVm::Diff(diff) => {
            crate::tui::render::render_diff_pane_vm(frame, frame.area(), diff, &state.theme);
        }
        ConfirmBackgroundVm::Empty => {}
    }
    let modal = match &confirm.kind {
        ConfirmModalKind::DescriptionInput {
            prefix,
            input,
            suffix,
        } => crate::tui::render::render_centered_modal_input(
            frame,
            confirm.title,
            prefix,
            input,
            suffix,
            confirm.border,
            &state.theme,
        ),
        ConfirmModalKind::Prompt { text } => crate::tui::render::render_centered_modal(
            frame,
            confirm.title,
            text,
            confirm.border,
            &state.theme,
        ),
    };
    if chrome.mouse_enabled {
        // Put the close button on the modal box itself, not the full-screen corner.
        layout.close_button = Some(crate::tui::render::render_close_button(
            frame,
            modal,
            &state.theme,
        ));
    }
}

/// `UploadPreview` outcome: stage the pending Upload action and open Confirm with the
/// local-vs-gist diff.
pub(crate) fn on_upload_preview(
    state: &mut AppState,
    result: std::result::Result<String, String>,
    file: crate::domain::GistFileRef,
    local_path: PathBuf,
    local_label: String,
    gist_label: String,
) -> LoopFlow {
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

    LoopFlow::Proceed
}

/// `CompactAnalyze` outcome: a single-revision gist has nothing to compact; otherwise open
/// the Confirm warning before compacting.
pub(crate) fn on_compact_analyze(
    state: &mut AppState,
    result: std::result::Result<usize, String>,
    gist_id: String,
    label: String,
) -> LoopFlow {
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

    LoopFlow::Proceed
}

/// `RestoreRevisionReady` outcome. Returns [`LoopFlow::SkipIteration`] when the revision
/// content matches current (nothing to restore).
pub(crate) fn on_restore_revision_ready(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{
        detail_mut, gist_file_ref, gists_mut, pins_mut, set_diff_body, set_pending,
        state_ready_to_create, state_with_gists,
    };
    use crate::tui::view_model::confirm_prompt;
    use crate::tui::*;
    use crossterm::event::KeyCode;
    use std::path::PathBuf;

    fn upload_pending(gist_id: &str, filename: &str) -> PendingAction {
        PendingAction::Upload {
            gist_id: gist_id.into(),
            filename: filename.into(),
            local_path: PathBuf::from(format!("/tmp/{filename}")),
        }
    }

    #[test]
    fn apply_upload_edit_event_content_changed_updates_diff_live() {
        let mut state = initial_state();
        state.screen = Screen::Confirm(Box::default());
        set_pending(&mut state, upload_pending("a", "notes.txt"));
        state.upload.watching = true;
        state.upload.remote_content = Some("old\n".into());
        state.upload.local_label = Some("local".into());
        state.upload.gist_label = Some("gist".into());

        state.apply_upload_edit_event(crate::tui::bg::UploadEditWatchEvent::ContentChanged {
            gist_id: "a".into(),
            filename: "notes.txt".into(),
            content: "new\n".into(),
        });

        assert_eq!(state.upload.edited_content.as_deref(), Some("new\n"));
        assert!(
            state.upload.watching,
            "still watching — editor hasn't closed yet"
        );
        assert!(state
            .scroll_body()
            .expect("Confirm ScrollBody")
            .text
            .contains("new"));
    }

    #[test]
    fn apply_upload_edit_event_editor_closed_stops_watching() {
        let mut state = initial_state();
        state.screen = Screen::Confirm(Box::default());
        set_pending(&mut state, upload_pending("a", "notes.txt"));
        state.upload.watching = true;

        state.apply_upload_edit_event(crate::tui::bg::UploadEditWatchEvent::EditorClosed {
            gist_id: "a".into(),
            filename: "notes.txt".into(),
            content: "final\n".into(),
        });

        assert_eq!(state.upload.edited_content.as_deref(), Some("final\n"));
        assert!(!state.upload.watching);
    }

    #[test]
    fn apply_upload_edit_event_read_error_stops_watching_and_sets_status() {
        let mut state = initial_state();
        state.screen = Screen::Confirm(Box::default());
        set_pending(&mut state, upload_pending("a", "notes.txt"));
        state.upload.watching = true;

        state.apply_upload_edit_event(crate::tui::bg::UploadEditWatchEvent::ReadError {
            gist_id: "a".into(),
            filename: "notes.txt".into(),
            message: "permission denied".into(),
        });

        assert!(!state.upload.watching);
        assert_eq!(
            state.status.as_deref(),
            Some("failed to read edited file: permission denied")
        );
    }

    #[test]
    fn apply_upload_edit_event_discards_when_a_different_upload_is_now_pending() {
        let mut state = initial_state();
        // A new upload edit session started before the OLD one's final event arrived.
        state.screen = Screen::Confirm(Box::default());
        set_pending(&mut state, upload_pending("a", "other.txt"));
        state.upload.watching = true;
        state.upload.edited_content = Some("current session content".into());

        state.apply_upload_edit_event(crate::tui::bg::UploadEditWatchEvent::EditorClosed {
            gist_id: "a".into(),
            filename: "notes.txt".into(), // stale session's filename, not "other.txt"
            content: "stale content".into(),
        });

        assert_eq!(
            state.upload.edited_content.as_deref(),
            Some("current session content")
        );
        assert!(
            state.upload.watching,
            "the current session's watch must not be cancelled"
        );
    }

    #[test]
    fn apply_upload_edit_event_discards_stale_event_after_cancel_reentry_same_identity() {
        let mut state = initial_state();
        // Simulates: user cancelled a GUI-editor watch session (n resets watching to false but
        // does NOT kill the background thread), then re-entered upload for the SAME gist/file
        // without pressing `e` again. An event from the abandoned first session's thread must
        // not silently overwrite this new, non-watching session's content.
        state.screen = Screen::Confirm(Box::default());
        set_pending(&mut state, upload_pending("a", "notes.txt"));
        state.upload.watching = false; // never re-entered edit mode this session
        state.upload.edited_content = None;

        state.apply_upload_edit_event(crate::tui::bg::UploadEditWatchEvent::ContentChanged {
            gist_id: "a".into(),
            filename: "notes.txt".into(),
            content: "leaked from abandoned session".into(),
        });

        assert_eq!(
            state.upload.edited_content, None,
            "an event from an abandoned (cancelled, still-running) watch session must not \
             leak into a new, non-watching session with the same gist/file identity"
        );
    }

    #[test]
    fn restore_revision_confirm_prompt_and_y_intent() {
        let mut state = state_with_gists();
        state.screen = Screen::Confirm(Box::default());
        set_pending(
            &mut state,
            PendingAction::RestoreRevision {
                gist_id: "g1".into(),
                filename: "a.txt".into(),
                version: "oldsha".into(),
                version_label: "oldsha (3d ago)".into(),
                content: "old\n".into(),
            },
        );
        assert_eq!(
            confirm_modal_style(&state),
            ("Restore revision", Color::Yellow)
        );
        assert!(confirm_prompt(&state).contains("Restore a.txt to revision oldsha (3d ago)"));
        assert_eq!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::ExecuteRestoreRevision
        );
    }

    #[test]
    fn palette_blocked_during_confirm() {
        let mut state = crate::tui::initial_state();
        state.screen = Screen::Confirm(Box::default());
        state.handle_key(KeyCode::Char(';'));
        assert!(state.screen.is_confirm());
    }

    #[test]
    fn confirm_screen_scrolls_diff() {
        let mut state = initial_state();
        set_pending(&mut state, PendingAction::Download);
        set_diff_body(&mut state, "l1\nl2\nl3");
        assert_eq!(state.handle_key(KeyCode::Down), KeyOutcome::None);
        assert_eq!(state.scroll_body().expect("Confirm ScrollBody").scroll, 1);
        state.handle_key(KeyCode::Up);
        assert_eq!(state.scroll_body().expect("Confirm ScrollBody").scroll, 0);
    }

    #[test]
    fn confirm_y_returns_download() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        set_pending(&mut state, PendingAction::Download);
        assert!(matches!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::Download {
                mode: crate::actions::DownloadMode::Overwrite(_)
            }
        ));
    }

    #[test]
    fn confirm_n_returns_to_diff() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        set_pending(&mut state, PendingAction::Download);
        assert_eq!(state.handle_key(KeyCode::Char('n')), KeyOutcome::None);
        assert!(state.screen.is_diff());
    }

    #[test]
    fn confirm_esc_returns_to_diff() {
        let mut state = initial_state();
        state.enter_diff(
            "d".into(),
            "r".into(),
            PathBuf::from("/tmp/x"),
            PathBuf::from("/tmp/x"),
        );
        set_pending(&mut state, PendingAction::Download);
        assert_eq!(state.handle_key(KeyCode::Esc), KeyOutcome::None);
        assert!(state.screen.is_diff());
    }

    #[test]
    fn confirm_upload_y_returns_upload() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Upload {
                gist_id: "a".into(),
                filename: "settings.json".into(),
                local_path: PathBuf::from("/tmp/settings.json"),
            },
        );
        assert_eq!(state.handle_key(KeyCode::Char('y')), KeyOutcome::Upload);
    }

    #[test]
    fn confirm_upload_e_returns_edit_upload() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Upload {
                gist_id: "a".into(),
                filename: "settings.json".into(),
                local_path: PathBuf::from("/tmp/settings.json"),
            },
        );
        assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::EditUpload);
    }

    #[test]
    fn confirm_upload_y_is_blocked_while_watching() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Upload {
                gist_id: "a".into(),
                filename: "settings.json".into(),
                local_path: PathBuf::from("/tmp/settings.json"),
            },
        );
        state.upload.watching = true;

        assert_eq!(state.handle_key(KeyCode::Char('y')), KeyOutcome::None);
        assert_eq!(
            state.status.as_deref(),
            Some("editor still open — finish editing first")
        );
    }

    #[test]
    fn confirm_upload_e_is_blocked_while_watching() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Upload {
                gist_id: "a".into(),
                filename: "settings.json".into(),
                local_path: PathBuf::from("/tmp/settings.json"),
            },
        );
        state.upload.watching = true;

        assert_eq!(state.handle_key(KeyCode::Char('e')), KeyOutcome::None);
        assert_eq!(state.status.as_deref(), Some("editor already open"));
    }

    #[test]
    fn confirm_upload_json_toggles() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Upload {
                gist_id: "a".into(),
                filename: "settings.json".into(),
                local_path: PathBuf::from("/tmp/settings.json"),
            },
        );
        assert!(!state.upload.json_pretty);
        assert!(!state.upload.json_sort);

        // Toggle pretty
        assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::None);
        assert!(state.upload.json_pretty);

        // Toggle sort
        assert_eq!(state.handle_key(KeyCode::Char('s')), KeyOutcome::None);
        assert!(state.upload.json_sort);

        // Toggle pretty off
        assert_eq!(state.handle_key(KeyCode::Char('p')), KeyOutcome::None);
        assert!(!state.upload.json_pretty);
    }

    #[test]
    fn remove_file_confirm_y_returns_execute_remove_file() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::RemoveFile {
                gist_id: "abc123".into(),
                filename: "a.md".into(),
                label: "my notes".into(),
            },
        );
        assert_eq!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::ExecuteRemoveFile
        );
    }

    #[test]
    fn delete_confirm_y_returns_execute_delete() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Delete {
                gist_id: "abc123".into(),
                label: "my notes".into(),
            },
        );
        assert_eq!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::ExecuteDelete
        );
    }

    #[test]
    fn delete_from_list_returns_to_list() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Delete {
                gist_id: "abc123".into(),
                label: "my notes".into(),
            },
        );
        assert_eq!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::ExecuteDelete
        );
        state.cancel_confirm_after_delete();
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn delete_from_gist_detail_opened_via_gists_returns_to_gists() {
        let mut state = initial_state();
        gists_mut(&mut state);
        state.enter(Screen::GistDetail(Box::default()));
        detail_mut(&mut state).gist_id = Some("abc123".into());
        set_pending(
            &mut state,
            PendingAction::Delete {
                gist_id: "abc123".into(),
                label: "my notes".into(),
            },
        );
        assert_eq!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::ExecuteDelete
        );
        state.cancel_confirm_after_delete();
        assert!(state.screen.is_gists());
    }

    #[test]
    fn delete_from_gist_detail_opened_via_pins_returns_to_pins() {
        let mut state = initial_state();
        pins_mut(&mut state);
        state.enter(Screen::GistDetail(Box::default()));
        detail_mut(&mut state).gist_id = Some("abc123".into());
        set_pending(
            &mut state,
            PendingAction::Delete {
                gist_id: "abc123".into(),
                label: "my notes".into(),
            },
        );
        assert_eq!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::ExecuteDelete
        );
        state.cancel_confirm_after_delete();
        assert!(state.screen.is_pins());
    }

    #[test]
    fn create_description_edits_mid_string_with_cursor_keys() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Create {
                local_path: PathBuf::from("notes.txt"),
            },
        );
        state.editing_description = true;
        for c in "helo".chars() {
            state.handle_key(KeyCode::Char(c));
        }
        // Fix the typo: go back one char and insert the missing 'l'.
        state.handle_key(KeyCode::Left);
        state.handle_key(KeyCode::Char('l'));
        assert_eq!(state.description_input, "hello");
        // Enter advances to the visibility step without losing the text.
        state.handle_key(KeyCode::Enter);
        assert!(!state.editing_description);
        assert_eq!(state.description_input, "hello");
    }

    #[test]
    fn create_confirm_s_and_p_choose_visibility() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Create {
                local_path: PathBuf::from("/tmp/config.toml"),
            },
        );
        assert_eq!(
            state.handle_key(KeyCode::Char('s')),
            KeyOutcome::Create(false)
        );

        set_pending(
            &mut state,
            PendingAction::Create {
                local_path: PathBuf::from("/tmp/config.toml"),
            },
        );
        assert_eq!(
            state.handle_key(KeyCode::Char('p')),
            KeyOutcome::Create(true)
        );
    }

    #[test]
    fn n_starts_create_in_the_description_editor() {
        let mut state = state_ready_to_create();
        state.handle_key(KeyCode::Char('n'));
        assert!(state.screen.is_confirm());
        assert!(state.editing_description);
        // While editing, letters (incl. s/p) are typed into the description, not
        // interpreted as the visibility choice.
        for c in "notes".chars() {
            assert_eq!(state.handle_key(KeyCode::Char(c)), KeyOutcome::None);
        }
        assert_eq!(state.description_input, "notes");
    }

    #[test]
    fn create_enter_advances_to_visibility_then_s_creates() {
        let mut state = state_ready_to_create();
        state.handle_key(KeyCode::Char('n'));
        state.handle_key(KeyCode::Char('h'));
        state.handle_key(KeyCode::Char('i'));
        // Enter ends the description step (does not create yet).
        assert_eq!(state.handle_key(KeyCode::Enter), KeyOutcome::None);
        assert!(!state.editing_description);
        assert_eq!(state.description_input, "hi");
        // Now s/p choose visibility and trigger the create.
        assert_eq!(
            state.handle_key(KeyCode::Char('s')),
            KeyOutcome::Create(false)
        );
    }

    #[test]
    fn on_upload_preview_err_sets_status() {
        let mut state = initial_state();

        on_upload_preview(
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

        on_upload_preview(
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

    #[test]
    fn on_compact_analyze_single_revision_sets_status() {
        let mut state = initial_state();

        on_compact_analyze(&mut state, Ok(1), "g1".into(), "demo".into());

        assert_eq!(
            state.status.as_deref(),
            Some("\"demo\" already has a single revision — nothing to compact")
        );
    }

    #[test]
    fn on_compact_analyze_multi_revision_enters_confirm() {
        let mut state = initial_state();

        on_compact_analyze(&mut state, Ok(4), "g1".into(), "demo".into());

        assert!(matches!(
            state.pending_action(),
            Some(PendingAction::CompactGist { gist_id, count, .. })
                if gist_id == "g1" && *count == 4
        ));
    }

    #[test]
    fn on_compact_analyze_err_sets_status() {
        let mut state = initial_state();

        on_compact_analyze(&mut state, Err("boom".into()), "g1".into(), "demo".into());

        assert_eq!(state.status.as_deref(), Some("revision check failed: boom"));
    }

    #[test]
    fn on_restore_revision_ready_returns_skip_iteration_when_identical() {
        let mut state = initial_state();

        let flow = on_restore_revision_ready(
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

        let flow = on_restore_revision_ready(
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

        on_restore_revision_ready(
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
