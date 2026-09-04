//! `Screen::Confirm` — key handling, view-model, paint, and apply handlers colocated in
//! one file (issue #287, Phase 2; issue #383).

use crate::tui::bg::LoopFlow;
use crate::tui::gist_content::{ContentLookup, FetchPolicy};
use crate::tui::render::{gist_info_line, unix_now};
use crate::tui::screens::diff::DiffVm;
use crate::tui::{
    AppState, HelpTopic, HitTarget, KeyOutcome, MouseFrame, PendingAction, Screen, TextInput,
};
use crossterm::event::KeyCode;
use ratatui::style::Color;
use ratatui::Frame;
use std::path::PathBuf;

/// Compact-gist confirm background (info + file list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompactGistBgVm {
    pub block_title: String,
    pub info_line: String,
    pub files: Vec<String>,
    pub file_cursor: usize,
}

/// Confirm modal + which background to paint under it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConfirmVm {
    pub title: &'static str,
    pub border: Color,
    pub kind: ConfirmModalKind,
    pub background: ConfirmBackgroundVm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfirmBackgroundVm {
    /// Standard overwrite/upload/create backdrop: pre-built diff view model.
    Diff(DiffVm),
    /// Compaction confirm: gist info + file list.
    CompactGist(CompactGistBgVm),
    /// Missing group or nothing to show.
    Empty,
}

/// One key that resolves a confirm modal, plus the verb it performs. The verb is the same
/// word the footer hint and the resulting status use, so an action reads identically all the
/// way through (`docs/agents/design.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfirmKeyVm {
    pub key: &'static str,
    pub label: String,
}

impl ConfirmKeyVm {
    pub(crate) fn new(key: &'static str, label: impl Into<String>) -> Self {
        Self {
            key,
            label: label.into(),
        }
    }

    /// Cells the key column plus its label occupy, before any gutter.
    pub(crate) fn width(&self) -> usize {
        self.key.chars().count() + 2 + self.label.chars().count()
    }
}

/// The structured body of a confirm modal: the question, an optional second line of context
/// or consequence, the keys that resolve it, and any secondary toggles.
///
/// A destructive action puts `n cancel` first in `keys` and states its consequence in
/// `detail`, so the stakes survive without relying on the border colour alone.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ConfirmPromptVm {
    pub question: String,
    pub detail: Option<String>,
    pub keys: Vec<ConfirmKeyVm>,
    /// Toggles that change what the primary key would do, on their own row.
    pub options: Vec<ConfirmKeyVm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfirmModalKind {
    /// Static y/n (or multi-key) prompt body.
    Prompt(ConfirmPromptVm),
    /// Create-flow description editor.
    DescriptionInput {
        prefix: &'static str,
        input: TextInput,
        keys: Vec<ConfirmKeyVm>,
    },
}

pub(crate) fn build_compact_gist_bg_vm(state: &AppState, gist_id: &str) -> Option<CompactGistBgVm> {
    let group = state.group_by_id(gist_id)?;
    let block_title = if group.description.trim().is_empty() {
        format!("Gist {}", group.id)
    } else {
        format!("Gist: {}", group.description)
    };
    let files = state.gist_file_display_names(gist_id);
    let file_cursor = state
        .detail()
        .map(|d| d.file_cursor)
        .unwrap_or(0)
        .min(files.len().saturating_sub(1));
    Some(CompactGistBgVm {
        block_title,
        info_line: gist_info_line(
            &group,
            unix_now(),
            state.gist_catalog.user_login.as_deref(),
            state.gist_is_starred(gist_id),
            state.gist_counts(gist_id),
        ),
        files,
        file_cursor,
    })
}

/// The keys that resolve the create flow's description editor.
pub(crate) fn description_input_keys() -> Vec<ConfirmKeyVm> {
    vec![
        ConfirmKeyVm::new("Enter", "next"),
        ConfirmKeyVm::new("Esc", "cancel"),
    ]
}

pub(crate) const HELP_TOPIC: HelpTopic = HelpTopic::List;

pub(crate) fn help_topic() -> HelpTopic {
    HELP_TOPIC
}

pub(crate) fn wheel_step() -> usize {
    3
}

/// Stage an upload preview before dispatch fetches the current gist content.
pub(crate) fn stage_upload_preview(
    state: &mut AppState,
    local_path: PathBuf,
    file: crate::domain::GistFileRef,
) -> (crate::domain::GistFileRef, String, String) {
    let gist_file = state.gist_file_for_diff(&file);
    let (local_label, gist_label) = crate::tui::render::diff_labels(Some(&local_path), &gist_file);
    let ContentLookup::Miss(file) =
        state
            .gist_content_store
            .lookup(&state.gist_catalog, file, FetchPolicy::Refresh)
    else {
        unreachable!("fresh fetch always bypasses cached content")
    };
    (file, local_label, gist_label)
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
                KeyCode::Char('p') if AppState::is_json_file(local_path) => {
                    self.upload.json_pretty = !self.upload.json_pretty;
                    self.update_upload_diff();
                }
                KeyCode::Char('s') if AppState::is_json_file(local_path) => {
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
            Some(PendingAction::RestoreRevision {
                gist_id,
                filename,
                content,
                ..
            }) => match code {
                KeyCode::Char('y') => {
                    let owner_login = self.gist_owner_login(&gist_id);
                    return KeyOutcome::Revision(
                        crate::tui::gist_revision::RevisionRequest::ExecuteRestore {
                            target: crate::tui::gist_revision::RevisionTarget::new(
                                crate::domain::GistFileRef::id_name(gist_id, filename),
                                owner_login,
                            ),
                            content,
                        },
                    );
                }
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    self.cancel_confirm();
                    if !self.screen.is_revisions() {
                        self.screen = Screen::Revisions(Box::default());
                    }
                }
                _ => {}
            },
            // Exhaustive over `PendingAction` (issue #417): a new variant that forgets its
            // arm fails to compile rather than silently landing here with no `y` binding.
            // `None` means "not on the Confirm screen", which `screens::lookup` never routes
            // here — handled defensively, not as an eighth kind of confirmation.
            None => {
                if matches!(code, KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q')) {
                    self.cancel_confirm();
                }
            }
        }
        KeyOutcome::None
    }
}

/// Everything the confirm modal shows, as one exhaustive match on the pending action:
/// each row carries that action's title, border colour, body, and background together
/// (issue #417).
///
/// There is deliberately no wildcard arm. A new [`PendingAction`] that forgets its row
/// fails to compile, so it can never silently inherit another action's prompt — the
/// failure this replaced, where an unhandled action rendered the overwrite gate's
/// "Overwrite …? The local file is replaced" in destructive red.
///
/// Destructive actions (delete, remove, compact, overwrite) wear `del_color`, lead their
/// keys with `n cancel`, and spell out what cannot be undone; the rest lead with their own
/// verb on the neutral `notice_color`.
pub(crate) fn build_confirm_vm(state: &AppState) -> ConfirmVm {
    let theme = state.settings.theme();
    let cancel = || ConfirmKeyVm::new("n", "cancel");
    let Some(action) = state.pending_action() else {
        // Unreachable through `screens::lookup`, which routes here only for
        // `Screen::Confirm`, and `ConfirmState::action` is not optional. Paint an empty
        // modal rather than panic in the render path.
        return ConfirmVm {
            title: "Confirm",
            border: theme.notice_color,
            kind: ConfirmModalKind::Prompt(ConfirmPromptVm::default()),
            background: ConfirmBackgroundVm::Empty,
        };
    };
    match action {
        PendingAction::Download => ConfirmVm {
            title: "Overwrite",
            border: theme.del_color,
            kind: ConfirmModalKind::Prompt(ConfirmPromptVm {
                question: format!(
                    "Overwrite {}?",
                    crate::config::display_path(&state.download_target())
                ),
                detail: Some("The local file is replaced with the gist's content.".to_string()),
                keys: vec![cancel(), ConfirmKeyVm::new("y", "overwrite")],
                options: Vec::new(),
            }),
            background: diff_background(state),
        },
        PendingAction::Upload {
            gist_id,
            filename,
            local_path,
        } => ConfirmVm {
            title: "Upload",
            border: theme.notice_color,
            kind: ConfirmModalKind::Prompt(upload_prompt(state, gist_id, filename, local_path)),
            background: diff_background(state),
        },
        // Two steps, one row: the description editor first, then the visibility choice.
        PendingAction::Create { .. } if state.editing_description => ConfirmVm {
            title: "Description",
            border: theme.accent,
            kind: ConfirmModalKind::DescriptionInput {
                prefix: crate::tui::render::CREATE_DESC_PREFIX,
                input: state.description_input.clone(),
                keys: description_input_keys(),
            },
            background: diff_background(state),
        },
        PendingAction::Create { local_path } => ConfirmVm {
            title: "Create gist",
            border: theme.notice_color,
            kind: ConfirmModalKind::Prompt(ConfirmPromptVm {
                question: format!(
                    "Create a gist from {}?",
                    crate::config::display_path(local_path)
                ),
                detail: Some(if state.description_input.is_empty() {
                    "No description.".to_string()
                } else {
                    format!("Description: {}", state.description_input)
                }),
                keys: vec![
                    ConfirmKeyVm::new("s", "secret"),
                    ConfirmKeyVm::new("p", "public"),
                    ConfirmKeyVm::new("Esc", "cancel"),
                ],
                options: Vec::new(),
            }),
            background: diff_background(state),
        },
        PendingAction::Delete { gist_id, label } => ConfirmVm {
            title: "Delete",
            border: theme.del_color,
            kind: ConfirmModalKind::Prompt(ConfirmPromptVm {
                question: format!("Permanently delete \"{label}\"?"),
                detail: Some(format!("gist {gist_id} — every file in it goes with it.")),
                keys: vec![cancel(), ConfirmKeyVm::new("y", "delete")],
                options: Vec::new(),
            }),
            background: diff_background(state),
        },
        PendingAction::RemoveFile {
            gist_id, filename, ..
        } => ConfirmVm {
            title: "Remove file",
            border: theme.del_color,
            kind: ConfirmModalKind::Prompt(ConfirmPromptVm {
                question: format!("Remove {filename} from this gist?"),
                detail: Some(format!("gist {gist_id} — the file is deleted remotely.")),
                keys: vec![cancel(), ConfirmKeyVm::new("y", "remove")],
                options: Vec::new(),
            }),
            background: diff_background(state),
        },
        PendingAction::CompactGist {
            gist_id,
            label,
            count,
        } => ConfirmVm {
            title: "Compact revisions",
            border: theme.del_color,
            kind: ConfirmModalKind::Prompt(ConfirmPromptVm {
                question: format!("Compact {count} revisions of \"{label}\" into one?"),
                detail: Some("This force-pushes and cannot be undone.".to_string()),
                keys: vec![cancel(), ConfirmKeyVm::new("y", "compact")],
                options: Vec::new(),
            }),
            // The only action with its own backdrop: the revisions about to collapse.
            background: match build_compact_gist_bg_vm(state, gist_id) {
                Some(bg) => ConfirmBackgroundVm::CompactGist(bg),
                None => ConfirmBackgroundVm::Empty,
            },
        },
        PendingAction::RestoreRevision {
            filename,
            version_label,
            ..
        } => ConfirmVm {
            title: "Restore revision",
            border: theme.notice_color,
            kind: ConfirmModalKind::Prompt(ConfirmPromptVm {
                question: format!("Restore {filename} to revision {version_label}?"),
                detail: Some("The old content is uploaded as a new revision.".to_string()),
                keys: vec![ConfirmKeyVm::new("y", "restore"), cancel()],
                options: Vec::new(),
            }),
            background: diff_background(state),
        },
    }
}

/// The upload question: a spinner while the external editor is still open, otherwise the
/// upload prompt plus the JSON toggles the keys actually honour.
fn upload_prompt(
    state: &AppState,
    gist_id: &str,
    filename: &str,
    local_path: &std::path::Path,
) -> ConfirmPromptVm {
    if state.upload.watching {
        return ConfirmPromptVm {
            question: format!(
                "{} Waiting for the editor to close…",
                crate::tui::render::spinner_glyph(state.spinner_frame)
            ),
            detail: Some(format!("Editing {filename} before upload.")),
            keys: vec![ConfirmKeyVm::new("n", "cancel")],
            options: Vec::new(),
        };
    }
    let edit_label = if state.upload.edited_content.is_some() {
        "edit first [edited]"
    } else {
        "edit first"
    };
    // Offered only for JSON, because `p` / `s` are guarded on the same predicate.
    let options = if AppState::is_json_file(local_path) {
        vec![
            ConfirmKeyVm::new("p", format!("pretty {}", on_off(state.upload.json_pretty))),
            ConfirmKeyVm::new("s", format!("sort {}", on_off(state.upload.json_sort))),
        ]
    } else {
        Vec::new()
    };
    ConfirmPromptVm {
        question: format!("Upload {filename} to gist {gist_id}?"),
        detail: None,
        keys: vec![
            ConfirmKeyVm::new("y", "upload"),
            ConfirmKeyVm::new("n", "cancel"),
            ConfirmKeyVm::new("e", edit_label),
        ],
        options,
    }
}

fn diff_background(state: &AppState) -> ConfirmBackgroundVm {
    ConfirmBackgroundVm::Diff(crate::tui::screens::diff::build_diff_vm(state))
}

fn on_off(flag: bool) -> &'static str {
    if flag {
        "[on]"
    } else {
        "[off]"
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
    layout: &mut MouseFrame,
) {
    match &confirm.background {
        ConfirmBackgroundVm::CompactGist(bg) => {
            crate::tui::render::render_compact_gist_bg_vm(
                frame,
                frame.area(),
                bg,
                &state.settings.theme(),
            );
        }
        ConfirmBackgroundVm::Diff(diff) => {
            crate::tui::render::render_diff_pane_vm(
                frame,
                frame.area(),
                diff,
                &state.settings.theme(),
            );
        }
        ConfirmBackgroundVm::Empty => {}
    }
    let modal = match &confirm.kind {
        ConfirmModalKind::DescriptionInput {
            prefix,
            input,
            keys,
        } => crate::tui::render::render_confirm_input_modal(
            frame,
            confirm.title,
            prefix,
            input,
            keys,
            confirm.border,
            &state.settings.theme(),
        ),
        ConfirmModalKind::Prompt(prompt) => crate::tui::render::render_confirm_modal(
            frame,
            confirm.title,
            prompt,
            confirm.border,
            &state.settings.theme(),
        ),
    };
    if chrome.mouse_enabled {
        // Put the close button on the modal box itself, not the full-screen corner.
        let close = crate::tui::render::render_close_button(frame, modal, &state.settings.theme());
        layout.register(HitTarget::Close, close);
    }
}

/// `UploadPreview` outcome: stage the pending Upload action and open Confirm with the
/// local-vs-gist diff.
pub(crate) fn on_upload_preview(
    state: &mut AppState,
    entry: crate::tui::DeferredEntry,
    result: std::result::Result<String, String>,
    file: crate::domain::GistFileRef,
    local_path: PathBuf,
    local_label: String,
    gist_label: String,
) -> LoopFlow {
    match result {
        Ok(remote) => {
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
                    state.open_deferred(
                        entry,
                        Screen::Confirm(Box::new(crate::tui::ConfirmState {
                            action,
                            body: crate::tui::ScrollBody::default(),
                        })),
                    );
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
    entry: crate::tui::DeferredEntry,
    result: std::result::Result<usize, String>,
    gist_id: String,
    label: String,
) -> LoopFlow {
    match result {
        Ok(count) if count <= 1 => state.set_status(format!(
            "\"{label}\" already has a single revision — nothing to compact"
        )),
        Ok(count) => {
            state.open_deferred(
                entry,
                Screen::Confirm(Box::new(crate::tui::ConfirmState {
                    action: PendingAction::CompactGist {
                    gist_id: gist_id.clone(),
                    label: label.clone(),
                    count,
                    },
                    body: crate::tui::ScrollBody {
                        text: format!(
                            "Compact gist {gist_id} (\"{label}\").\n\nIt has {count} revisions. Compacting clones it to a temp dir, squashes the history to a single commit, and force-pushes — the {} older revisions are gone for good.",
                            count - 1
                        ),
                        ..crate::tui::ScrollBody::default()
                    },
                })),
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
    entry: crate::tui::DeferredEntry,
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
                state.settings.ignore_trailing_newline(),
            );
            state.open_deferred(
                entry,
                Screen::Confirm(Box::new(crate::tui::ConfirmState {
                    action: PendingAction::RestoreRevision {
                        gist_id,
                        filename,
                        version,
                        version_label,
                        content: revision_content,
                    },
                    body: crate::tui::ScrollBody {
                        text: diff,
                        ..crate::tui::ScrollBody::default()
                    },
                })),
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
    use crate::tui::*;
    use crossterm::event::KeyCode;
    use ratatui::style::Color;
    use std::path::PathBuf;

    // Every confirm assertion below enters through `build_confirm_vm`, the screen's
    // view-model column in `screens::lookup` (issue #417). The title, border, body and
    // background it returns are the modal's whole presentation, so nothing needs to reach
    // past it into how that presentation is assembled.
    fn confirm_vm(state: &AppState) -> ConfirmVm {
        build_confirm_vm(state)
    }

    fn style(state: &AppState) -> (&'static str, Color) {
        let vm = confirm_vm(state);
        (vm.title, vm.border)
    }

    fn prompt_vm(state: &AppState) -> ConfirmPromptVm {
        match confirm_vm(state).kind {
            ConfirmModalKind::Prompt(prompt) => prompt,
            other => panic!("expected a prompt body, got {other:?}"),
        }
    }

    #[test]
    fn confirm_vm_covers_each_pending_action() {
        let mut state = initial_state();
        let keys = |state: &AppState| {
            prompt_vm(state)
                .keys
                .iter()
                .map(|k| format!("{} {}", k.key, k.label))
                .collect::<Vec<_>>()
        };

        state.enter_diff(
            String::new(),
            String::new(),
            PathBuf::new(),
            PathBuf::from("notes.txt"),
        );
        state.enter_confirm_from_diff(PendingAction::Download);
        let prompt = prompt_vm(&state);
        assert_eq!(prompt.question, "Overwrite notes.txt?");
        // Destructive: cancel leads, and the consequence is stated rather than left to the
        // border colour (see `docs/agents/design.md`).
        assert_eq!(keys(&state), ["n cancel", "y overwrite"]);
        assert!(prompt.detail.is_some());
        assert_eq!(style(&state), ("Overwrite", Color::Red));

        set_pending(
            &mut state,
            PendingAction::Delete {
                gist_id: "abc".into(),
                label: "my config".into(),
            },
        );
        let prompt = prompt_vm(&state);
        assert_eq!(prompt.question, "Permanently delete \"my config\"?");
        assert!(prompt.detail.unwrap().contains("abc"));
        assert_eq!(keys(&state), ["n cancel", "y delete"]);
        assert_eq!(style(&state), ("Delete", Color::Red));

        set_pending(
            &mut state,
            PendingAction::RemoveFile {
                gist_id: "abc".into(),
                filename: "main.rs".into(),
                label: "my config".into(),
            },
        );
        assert_eq!(keys(&state), ["n cancel", "y remove"]);

        set_pending(
            &mut state,
            PendingAction::Upload {
                gist_id: "g1".into(),
                filename: "main.rs".into(),
                local_path: PathBuf::from("main.rs"),
            },
        );
        let prompt = prompt_vm(&state);
        assert_eq!(prompt.question, "Upload main.rs to gist g1?");
        // Not destructive: the action's own verb leads, and a non-JSON file offers no toggles.
        assert_eq!(keys(&state), ["y upload", "n cancel", "e edit first"]);
        assert!(prompt.options.is_empty());
        assert_eq!(style(&state), ("Upload", Color::Yellow));

        set_pending(
            &mut state,
            PendingAction::CompactGist {
                gist_id: "abc".into(),
                label: "my config".into(),
                count: 4,
            },
        );
        let prompt = prompt_vm(&state);
        assert_eq!(
            prompt.question,
            "Compact 4 revisions of \"my config\" into one?"
        );
        assert_eq!(
            prompt.detail.as_deref(),
            Some("This force-pushes and cannot be undone.")
        );
        assert_eq!(keys(&state), ["n cancel", "y compact"]);
        assert_eq!(style(&state), ("Compact revisions", Color::Red));
    }

    /// The exhaustive match in `build_confirm_vm` guarantees every pending action has a
    /// row; it cannot guarantee the row was filled in, and an empty question compiles fine.
    /// This walks all of them, including the create flow's description step (issue #417).
    #[test]
    fn every_pending_action_has_a_question_and_a_resolving_key() {
        let mut state = initial_state();
        state.screen = Screen::Confirm(Box::default());
        let actions = [
            ("Download", PendingAction::Download),
            (
                "Upload",
                PendingAction::Upload {
                    gist_id: "g1".into(),
                    filename: "a.txt".into(),
                    local_path: PathBuf::from("a.txt"),
                },
            ),
            (
                "Create",
                PendingAction::Create {
                    local_path: PathBuf::from("a.txt"),
                },
            ),
            (
                "Delete",
                PendingAction::Delete {
                    gist_id: "g1".into(),
                    label: "my config".into(),
                },
            ),
            (
                "RemoveFile",
                PendingAction::RemoveFile {
                    gist_id: "g1".into(),
                    filename: "a.txt".into(),
                    label: "my config".into(),
                },
            ),
            (
                "CompactGist",
                PendingAction::CompactGist {
                    gist_id: "g1".into(),
                    label: "my config".into(),
                    count: 4,
                },
            ),
            (
                "RestoreRevision",
                PendingAction::RestoreRevision {
                    gist_id: "g1".into(),
                    filename: "a.txt".into(),
                    version: "oldsha".into(),
                    version_label: "oldsha (3d ago)".into(),
                    content: "old\n".into(),
                },
            ),
        ];

        for editing_description in [false, true] {
            state.editing_description = editing_description;
            for (name, action) in actions.clone() {
                set_pending(&mut state, action);
                let vm = confirm_vm(&state);
                assert!(
                    !vm.title.is_empty(),
                    "{name} (editing={editing_description}): no title"
                );
                let keys = match &vm.kind {
                    ConfirmModalKind::Prompt(prompt) => {
                        assert!(
                            !prompt.question.trim().is_empty(),
                            "{name} (editing={editing_description}): empty question"
                        );
                        &prompt.keys
                    }
                    ConfirmModalKind::DescriptionInput { keys, .. } => keys,
                };
                assert!(
                    !keys.is_empty(),
                    "{name} (editing={editing_description}): no key resolves the modal"
                );
            }
        }
    }

    #[test]
    fn upload_confirm_offers_json_toggles_only_for_json() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Upload {
                gist_id: "g1".into(),
                filename: "settings.json".into(),
                local_path: PathBuf::from("settings.json"),
            },
        );
        state.upload.json_pretty = true;
        let options: Vec<String> = prompt_vm(&state)
            .options
            .iter()
            .map(|k| format!("{} {}", k.key, k.label))
            .collect();
        assert_eq!(options, ["p pretty [on]", "s sort [off]"]);
    }

    #[test]
    fn confirm_vm_shows_description_editor_for_create() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Create {
                local_path: PathBuf::from("notes.txt"),
            },
        );
        state.editing_description = true;
        state.description_input = "hello".into();
        // The editor itself is a `DescriptionInput` body, so what matters here is the
        // title/border and the keys offered beneath it.
        assert_eq!(style(&state), ("Description", Color::Cyan));
        let keys: Vec<String> = description_input_keys()
            .iter()
            .map(|k| format!("{} {}", k.key, k.label))
            .collect();
        assert_eq!(keys, ["Enter next", "Esc cancel"]);
    }

    #[test]
    fn confirm_vm_shows_watching_indicator_for_upload() {
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Upload {
                gist_id: "a".into(),
                filename: "notes.txt".into(),
                local_path: PathBuf::from("/tmp/notes.txt"),
            },
        );
        state.upload.watching = true;

        let prompt = prompt_vm(&state);
        assert!(prompt.question.contains("Waiting for the editor"));
        assert_eq!(
            prompt.keys.iter().map(|k| k.key).collect::<Vec<_>>(),
            ["n"],
            "upload and edit are unavailable while the editor is open"
        );
    }

    #[test]
    fn create_confirm_prompt_shortens_home_path() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/u"));
        let mut state = initial_state();
        set_pending(
            &mut state,
            PendingAction::Create {
                local_path: home.join("notes.txt"),
            },
        );
        let prompt = prompt_vm(&state);
        assert_eq!(prompt.question, "Create a gist from ~/notes.txt?");
        assert_eq!(
            prompt.keys.iter().map(|k| k.key).collect::<Vec<_>>(),
            ["s", "p", "Esc"]
        );
    }

    #[test]
    fn stage_upload_preview_falls_back_to_list_raw_url() {
        let mut state = state_with_gists();
        state.gist_catalog.owned[0].raw_url = Some("https://example.test/a.txt".into());
        let file = gist_file_ref("g1", "a.txt");

        let (file, local_label, gist_label) =
            stage_upload_preview(&mut state, PathBuf::from("/tmp/a.txt"), file);

        assert_eq!(file.raw_url.as_deref(), Some("https://example.test/a.txt"));
        assert!(local_label.starts_with("local: a.txt"));
        assert!(gist_label.starts_with("gist g1 / a.txt"));
    }

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
        assert_eq!(style(&state), ("Restore revision", Color::Yellow));
        assert_eq!(
            prompt_vm(&state).question,
            "Restore a.txt to revision oldsha (3d ago)?"
        );
        assert_eq!(
            state.handle_key(KeyCode::Char('y')),
            KeyOutcome::Revision(crate::tui::gist_revision::RevisionRequest::ExecuteRestore {
                target: crate::tui::gist_revision::RevisionTarget::new(
                    crate::domain::GistFileRef::id_name("g1", "a.txt"),
                    String::new(),
                ),
                content: "old\n".into(),
            }),
            "the confirmed content is what gets restored, snapshotted at intent time"
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
            initial_state().defer_entry(),
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
            initial_state().defer_entry(),
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

        let entry = state.defer_entry();
        on_compact_analyze(&mut state, entry, Ok(1), "g1".into(), "demo".into());

        assert_eq!(
            state.status.as_deref(),
            Some("\"demo\" already has a single revision — nothing to compact")
        );
    }

    #[test]
    fn on_compact_analyze_multi_revision_enters_confirm() {
        let mut state = initial_state();

        let entry = state.defer_entry();
        on_compact_analyze(&mut state, entry, Ok(4), "g1".into(), "demo".into());

        assert!(matches!(
            state.pending_action(),
            Some(PendingAction::CompactGist { gist_id, count, .. })
                if gist_id == "g1" && *count == 4
        ));
    }

    #[test]
    fn on_compact_analyze_err_sets_status() {
        let mut state = initial_state();

        let entry = state.defer_entry();
        on_compact_analyze(
            &mut state,
            entry,
            Err("boom".into()),
            "g1".into(),
            "demo".into(),
        );

        assert_eq!(state.status.as_deref(), Some("revision check failed: boom"));
    }

    #[test]
    fn on_restore_revision_ready_returns_skip_iteration_when_identical() {
        let mut state = initial_state();

        let flow = on_restore_revision_ready(
            &mut state,
            initial_state().defer_entry(),
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
            initial_state().defer_entry(),
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
            initial_state().defer_entry(),
            Err("boom".into()),
            "g1".into(),
            "a.txt".into(),
            "abc123".into(),
            "abc1234".into(),
        );

        assert_eq!(state.status.as_deref(), Some("boom"));
    }

    #[test]
    fn confirm_vm_prompt_identity() {
        let mut state = initial_state();
        state.enter_confirm(
            PendingAction::Upload {
                gist_id: "g1".into(),
                filename: "notes.txt".into(),
                local_path: PathBuf::from("notes.txt"),
            },
            String::new(),
        );
        let c = build_confirm_vm(&state);
        assert_eq!(c.title, "Upload");
        match c.kind {
            ConfirmModalKind::Prompt(prompt) => {
                assert_eq!(prompt.question, "Upload notes.txt to gist g1?");
            }
            other => panic!("expected Prompt, got {other:?}"),
        }
    }

    #[test]
    fn confirm_vm_overwrite_download() {
        let mut state = initial_state();
        state.enter_diff(
            String::new(),
            String::new(),
            PathBuf::new(),
            PathBuf::from("notes.txt"),
        );
        state.enter_confirm_from_diff(PendingAction::Download);
        let c = build_confirm_vm(&state);
        assert_eq!(c.title, "Overwrite");
        match c.kind {
            ConfirmModalKind::Prompt(prompt) => {
                assert_eq!(prompt.question, "Overwrite notes.txt?");
            }
            other => panic!("expected Prompt, got {other:?}"),
        }
    }

    #[test]
    fn confirm_vm_compact_background() {
        use crate::domain::GistFile;

        let mut state = initial_state();
        state.gist_catalog.owned = vec![GistFile {
            description: "pack".into(),
            ..GistFile::fixture("g1", "a.txt")
        }];
        state.enter_confirm(
            PendingAction::CompactGist {
                gist_id: "g1".into(),
                label: "pack".into(),
                count: 3,
            },
            String::new(),
        );
        let c = build_confirm_vm(&state);
        match c.background {
            ConfirmBackgroundVm::CompactGist(bg) => {
                assert!(bg.block_title.contains("pack") || bg.block_title.contains("g1"));
                assert!(!bg.files.is_empty());
            }
            other => panic!("expected CompactGist bg, got {other:?}"),
        }
    }
}
