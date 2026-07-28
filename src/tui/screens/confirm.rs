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
