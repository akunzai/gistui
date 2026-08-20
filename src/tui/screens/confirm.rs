//! `Screen::Confirm` — key handling, view-model, and paint colocated in one file (issue #287, Phase 2).

use crate::tui::view_model::{ConfirmBackgroundVm, ConfirmModalKind, ConfirmVm};
use crate::tui::{AppState, HelpTopic, KeyOutcome, MouseLayout, PendingAction, Screen};
use crossterm::event::KeyCode;
use ratatui::{style::Color, Frame};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::*;

    use crate::tui::tests::{set_pending, state_with_gists};

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
        assert!(state.diff_body_text().contains("new"));
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
}
