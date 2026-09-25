//! The **Gist mutation** workflow: every change to a gist itself (upload, create, delete,
//! remove a file, compact, description, star, fork). Its async outcome belongs to no single
//! screen (issue #383) — List, Gists, GistDetail, Pins, and Confirm can all launch one.
//!
//! Callers hand [`dispatch`] one plain-data [`MutationRequest`], resolved when the user acted,
//! so nothing here reads Confirm afterwards (#460). This module stages scratch copies, builds
//! the `gh` plan, leaves the screen that launched it ([`leave_before_spawn`] — the one place
//! that policy lives), runs the command through the injected [`crate::actions::CommandRunner`]
//! seam, and hands the result to the `on_*` apply handler below. Restoring a Gist revision is
//! the Gist revision workflow's (`gist_revision`), not this module's. Eligibility guards
//! (ownership, what is selected) stay with the screens that build the request.

use super::bg::{
    record_pin_sync, write_scratch_file, ActionJobKind, ActionJobSpec, Jobs, LoopFlow,
};
use super::{AppState, UploadDraft};
use crate::domain::GistFileRef;
use std::path::PathBuf;

/// One gist mutation, as plain data captured when the user acted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MutationRequest {
    /// Upload the draft's content to its gist file (replacing it, or adding it when the
    /// catalog has no such file yet).
    Upload(Box<UploadDraft>),
    /// Create a gist from a local file.
    Create {
        local_path: PathBuf,
        public: bool,
        description: String,
    },
    /// Delete a whole gist.
    Delete { gist_id: String },
    /// Remove one file from a gist.
    RemoveFile { file: GistFileRef },
    /// Collapse a gist's history to one revision (force-push). `label` / `count` are the
    /// intent-time wording its status reports.
    Compact {
        gist_id: String,
        label: String,
        count: usize,
    },
    /// Replace a gist's description.
    Description {
        gist_id: String,
        description: String,
    },
    /// Star (`starring`) or unstar a gist.
    Star { gist_id: String, starring: bool },
    /// Fork someone else's gist into the account.
    Fork { gist_id: String },
}

/// Stage `request`, leave the screen that launched it, and spawn its `gh` job. A staging
/// failure (a scratch copy that can't be written) reports and stays put.
pub(super) fn dispatch(jobs: &mut Jobs, state: &mut AppState, request: MutationRequest) {
    let Some(staged) = stage(state, &request) else {
        return;
    };
    leave_before_spawn(state, &request);
    let runner = jobs.command_runner();
    let Staged {
        plan,
        scratch,
        spec,
        apply,
    } = staged;
    jobs.spawn_action(
        state,
        spec,
        move || {
            let result = match plan {
                Plan::Command(plan) => {
                    crate::actions::run_command(runner.as_ref(), &plan).map(|_| ())
                }
                Plan::Compact { gist_id } => {
                    crate::actions::execute_compact_gist(runner.as_ref(), &gist_id)
                }
            }
            .map_err(|e| e.to_string());
            // ScratchDir owns cleanup: it drops here, after the command (issue #275).
            drop(scratch);
            result
        },
        move |result, state| apply(state, result),
    );
}

enum Plan {
    Command(crate::actions::CommandPlan),
    /// Compaction is a clone → rewrite → force-push sequence, still behind the runner seam.
    Compact {
        gist_id: String,
    },
}

type Apply = Box<dyn FnOnce(&mut AppState, Result<(), String>) -> LoopFlow + Send>;

struct Staged {
    plan: Plan,
    scratch: Option<crate::temp_dir::ScratchDir>,
    spec: ActionJobSpec,
    apply: Apply,
}

fn stage(state: &mut AppState, request: &MutationRequest) -> Option<Staged> {
    use crate::actions::*;
    let staged = match request.clone() {
        MutationRequest::Upload(draft) => {
            let sent = draft.content(&state.settings);
            // The pin baseline is the local file on disk, not the bytes sent (#465).
            let local_content = draft.original_content.clone();
            let (scratch, path) = write_scratch_file(
                state,
                "upload",
                &draft.filename,
                "temp file",
                sent.as_bytes(),
            )?;
            let has_same_name = state
                .gist_catalog
                .owned
                .iter()
                .any(|g| g.gist_id == draft.gist_id && g.filename == draft.filename);
            let file = GistFileRef::id_name(draft.gist_id.clone(), draft.filename.clone());
            let plan = if has_same_name {
                upload_command(&path, &file.to_gist_file())
            } else {
                upload_add_command(&path, &file.gist_id)
            };
            let local_path = draft.local_path.clone();
            Staged {
                plan: Plan::Command(plan),
                scratch: Some(scratch),
                spec: ActionJobSpec::new(
                    ActionJobKind::Upload { file: file.clone() },
                    "Uploading…",
                ),
                apply: Box::new(move |state, result| {
                    on_upload_replace(state, result, file, &local_path, &local_content, &sent)
                }),
            }
        }
        MutationRequest::Create {
            local_path,
            public,
            description,
        } => {
            // Send the bytes the Sync policy dictates (#465). When that rewrites the file,
            // upload a same-named scratch copy; otherwise hand `gh` the file itself, so an
            // unreadable or non-text file still creates as before.
            let normalized = crate::domain::read_text_file_capped(&local_path)
                .ok()
                .and_then(|text| match state.settings.sync_policy().outbound(&text) {
                    std::borrow::Cow::Owned(normalized) => Some(normalized),
                    std::borrow::Cow::Borrowed(_) => None,
                });
            let (scratch, source) = match normalized {
                Some(normalized) => {
                    let filename = local_path.file_name().and_then(|n| n.to_str())?;
                    let (dir, path) = write_scratch_file(
                        state,
                        "create",
                        filename,
                        "temp file",
                        normalized.as_bytes(),
                    )?;
                    (Some(dir), path)
                }
                None => (None, local_path.clone()),
            };
            Staged {
                plan: Plan::Command(create_command(&source, public, &description)),
                scratch,
                spec: ActionJobSpec::new(
                    ActionJobKind::Create {
                        local_path: local_path.clone(),
                        public,
                    },
                    "Creating gist…",
                ),
                apply: Box::new(move |state, result| {
                    on_create_gist(state, result, local_path, public)
                }),
            }
        }
        MutationRequest::Delete { gist_id } => Staged {
            plan: Plan::Command(delete_command(&gist_id)),
            scratch: None,
            spec: ActionJobSpec::new(
                ActionJobKind::DeleteGist {
                    gist_id: gist_id.clone(),
                },
                "Deleting gist…",
            ),
            apply: Box::new(move |state, result| on_delete_gist(state, result, gist_id)),
        },
        MutationRequest::RemoveFile { file } => Staged {
            plan: Plan::Command(remove_file_command(&file.gist_id, &file.filename)),
            scratch: None,
            spec: ActionJobSpec::new(
                ActionJobKind::RemoveFile { file: file.clone() },
                "Removing file…",
            ),
            apply: Box::new(move |state, result| {
                on_remove_file(state, result, file.gist_id, file.filename)
            }),
        },
        MutationRequest::Compact {
            gist_id,
            label,
            count,
        } => Staged {
            plan: Plan::Compact {
                gist_id: gist_id.clone(),
            },
            scratch: None,
            spec: ActionJobSpec::new(
                ActionJobKind::CompactGist { gist_id },
                "Compacting revisions…",
            ),
            apply: Box::new(move |state, result| on_compact_gist(state, result, label, count)),
        },
        MutationRequest::Description {
            gist_id,
            description,
        } => Staged {
            plan: Plan::Command(edit_description_command(&gist_id, &description)),
            scratch: None,
            spec: ActionJobSpec::new(
                ActionJobKind::UpdateDescription {
                    gist_id: gist_id.clone(),
                },
                "Updating description…",
            ),
            apply: Box::new(move |state, result| on_apply_description(state, result, gist_id)),
        },
        MutationRequest::Star { gist_id, starring } => Staged {
            plan: Plan::Command(if starring {
                star_gist_command(&gist_id)
            } else {
                unstar_gist_command(&gist_id)
            }),
            scratch: None,
            spec: ActionJobSpec::new(
                ActionJobKind::ToggleGistStar {
                    gist_id: gist_id.clone(),
                    starring,
                },
                if starring {
                    "Starring…"
                } else {
                    "Unstarring…"
                },
            ),
            apply: Box::new(move |state, result| {
                on_gist_star_toggle(state, result, gist_id, starring)
            }),
        },
        MutationRequest::Fork { gist_id } => Staged {
            plan: Plan::Command(fork_gist_command(&gist_id)),
            scratch: None,
            spec: ActionJobSpec::new(
                ActionJobKind::ForkGist {
                    gist_id: gist_id.clone(),
                },
                "Forking…",
            ),
            apply: Box::new(move |state, result| on_fork_gist(state, result, gist_id)),
        },
    };
    Some(staged)
}

/// Where each mutation leaves the UI once it is staged, before its job runs — the one place
/// this policy lives. Where it lands after the job is each `on_*` handler's business.
fn leave_before_spawn(state: &mut AppState, request: &MutationRequest) {
    match request {
        // Back to wherever the upload was opened from (List, or Pins for a pin push).
        MutationRequest::Upload(_) => state.leave(),
        // Confirm stays up until the result arrives; `on_create_gist` navigates.
        MutationRequest::Create { .. } => {}
        // Also pops the just-deleted gist's own GistDetail.
        MutationRequest::Delete { .. } => state.cancel_confirm_after_delete(),
        MutationRequest::RemoveFile { .. } => state.back_to_list(),
        MutationRequest::Compact { .. } => state.cancel_confirm(),
        MutationRequest::Description { .. } => {
            state.editing_description = false;
            state.description_input.clear();
        }
        MutationRequest::Star { .. } | MutationRequest::Fork { .. } => {}
    }
}

fn apply(
    state: &mut AppState,
    result: Result<(), String>,
    verb: &str,
    on_ok: impl FnOnce(&mut AppState) -> String,
) -> LoopFlow {
    match result {
        Ok(()) => {
            let msg = on_ok(state);
            state.set_status(msg);
            state.gist_list_stale = true;
        }
        Err(error) => state.set_status(format!("{verb} failed: {error}")),
    }
    LoopFlow::Proceed
}

/// `UploadReplace` outcome: commit the pin-sync record for the local file's bytes on disk
/// (the pin baseline, #465 — not the possibly redacted / transformed bytes sent), then
/// re-fetch the gist list. Navigation already happened when the upload started — the
/// Upload arm left Confirm for wherever it was opened from (List, or Pins for a pin push).
pub(crate) fn on_upload_replace(
    state: &mut AppState,
    result: Result<(), String>,
    file: crate::domain::GistFileRef,
    local_path: &std::path::Path,
    local_content: &str,
    sent_content: &str,
) -> LoopFlow {
    apply(state, result, "upload", |state| {
        state.gist_content_store.invalidate_file(&file);
        // The gist file's blob sha is now that of the bytes sent. Patch it into the in-memory
        // catalog so the pin reads as in sync before the refresh this upload triggers lands
        // (issue #466); the refresh then publishes the same sha.
        let sha = crate::domain::git_blob_sha1(sent_content.as_bytes());
        for g in state.gist_catalog.owned.iter_mut() {
            if g.gist_id == file.gist_id && g.filename == file.filename {
                if let Some(url) = g
                    .raw_url
                    .as_deref()
                    .and_then(|u| crate::domain::raw_url_with_blob_sha(u, &sha))
                {
                    g.raw_url = Some(url);
                }
            }
        }
        record_pin_sync(
            state,
            local_path,
            &file.gist_id,
            &file.filename,
            local_content,
            sent_content,
            Some(crate::domain::SyncDirection::Upload),
        );
        format!("Uploaded {} to gist {}", file.filename, file.gist_id)
    })
}

/// `CreateGist` outcome. Bespoke: the Err arm resets the screen, unlike [`apply`].
pub(crate) fn on_create_gist(
    state: &mut AppState,
    result: Result<(), String>,
    local_path: PathBuf,
    public: bool,
) -> LoopFlow {
    match result {
        Ok(()) => {
            let visibility = if public { "public" } else { "secret" };
            state.set_status(format!(
                "Created {} gist from {}",
                visibility,
                crate::config::display_path(&local_path)
            ));
            state.description_input.clear();
            state.back_to_list();
            state.gist_list_stale = true;
        }
        Err(error) => {
            // `back_to_list`, not a bare `screen = List`: the create Confirm was entered from
            // the list, and a plain assignment would leave that entry behind it (#475).
            state.set_status(format!("create failed: {error}"));
            state.back_to_list();
            state.description_input.clear();
        }
    }
    LoopFlow::Proceed
}

/// `DeleteGist` outcome.
pub(crate) fn on_delete_gist(
    state: &mut AppState,
    result: Result<(), String>,
    gist_id: String,
) -> LoopFlow {
    apply(state, result, "delete", |state| {
        state.gist_content_store.invalidate_gist(&gist_id);
        format!("Deleted gist {gist_id}")
    })
}

/// `RemoveFile` outcome.
pub(crate) fn on_remove_file(
    state: &mut AppState,
    result: Result<(), String>,
    gist_id: String,
    filename: String,
) -> LoopFlow {
    apply(state, result, "remove", |state| {
        state
            .gist_content_store
            .invalidate_file(&crate::domain::GistFileRef::id_name(
                gist_id.clone(),
                filename.clone(),
            ));
        format!("Removed {filename} from gist {gist_id}")
    })
}

/// `ApplyDescription` outcome.
pub(crate) fn on_apply_description(
    state: &mut AppState,
    result: Result<(), String>,
    gist_id: String,
) -> LoopFlow {
    apply(state, result, "description update", |_| {
        format!("Updated description for gist {gist_id}")
    })
}

/// `CompactGist` outcome.
pub(crate) fn on_compact_gist(
    state: &mut AppState,
    result: Result<(), String>,
    label: String,
    count: usize,
) -> LoopFlow {
    apply(state, result, "compact", |_| {
        format!("Compacted \"{label}\" ({count} → 1 revision)")
    })
}

/// `GistStarToggle` outcome.
pub(crate) fn on_gist_star_toggle(
    state: &mut AppState,
    result: Result<(), String>,
    gist_id: String,
    starred: bool,
) -> LoopFlow {
    apply(state, result, "star toggle", |state| {
        if starred {
            state.gist_catalog.starred_ids.insert(gist_id.clone());
            format!("starred {gist_id}")
        } else {
            state.gist_catalog.starred_ids.remove(&gist_id);
            format!("unstarred {gist_id}")
        }
    })
}

/// `ForkGist` outcome.
pub(crate) fn on_fork_gist(
    state: &mut AppState,
    result: Result<(), String>,
    gist_id: String,
) -> LoopFlow {
    apply(state, result, "fork", |_| {
        format!("forked {gist_id} into your account")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{PinnedMapping, SyncDirection};
    use crate::tui::test_support::gist_file_ref;
    use crate::tui::*;
    use std::sync::Arc;

    use crate::actions::test_support::SeqRunner;
    use crate::actions::CommandOutput;

    /// Drive one request through the workflow: stage, leave, run the worker inline against
    /// `runner`, then apply through the generation guard. Only the action outcome is drained —
    /// `Jobs::absorb` would start a real gist-list refresh once an apply marks it stale.
    fn run(state: &mut AppState, runner: &Arc<SeqRunner>, request: MutationRequest) {
        let mut jobs = Jobs::inline(&state.gist_catalog.clone(), runner.clone());
        dispatch(&mut jobs, state, request);
        jobs.on_action_outcome(state);
    }

    fn ok_runner(n: usize) -> Arc<SeqRunner> {
        Arc::new(SeqRunner::new(vec![CommandOutput::ok(""); n]))
    }

    /// Every single-command mutation runs exactly its planned `gh` command through the
    /// injected runner, and reports success.
    #[test]
    fn each_mutation_runs_its_gh_command_through_the_runner() {
        use crate::actions::*;
        let file = GistFileRef::id_name("g1", "a.txt");
        let cases: Vec<(MutationRequest, CommandPlan, &str)> = vec![
            (
                MutationRequest::Delete {
                    gist_id: "g1".into(),
                },
                delete_command("g1"),
                "deleted",
            ),
            (
                MutationRequest::RemoveFile { file: file.clone() },
                remove_file_command("g1", "a.txt"),
                "a.txt",
            ),
            (
                MutationRequest::Description {
                    gist_id: "g1".into(),
                    description: "new words".into(),
                },
                edit_description_command("g1", "new words"),
                "description",
            ),
            (
                MutationRequest::Star {
                    gist_id: "g1".into(),
                    starring: true,
                },
                star_gist_command("g1"),
                "starred",
            ),
            (
                MutationRequest::Star {
                    gist_id: "g1".into(),
                    starring: false,
                },
                unstar_gist_command("g1"),
                "unstarred",
            ),
            (
                MutationRequest::Fork {
                    gist_id: "g1".into(),
                },
                fork_gist_command("g1"),
                "forked",
            ),
        ];
        for (request, expected, status_word) in cases {
            let mut state = initial_state();
            let runner = ok_runner(1);
            let label = format!("{request:?}");

            run(&mut state, &runner, request);

            assert_eq!(runner.calls(), vec![expected], "{label}");
            let status = state.status.clone().unwrap_or_default();
            assert!(
                status.to_lowercase().contains(status_word) && !status.contains("failed"),
                "{label}: {status}"
            );
        }
    }

    #[test]
    fn compact_runs_its_clone_rewrite_force_push_through_the_runner() {
        let mut state = initial_state();
        let runner = Arc::new(SeqRunner::new(vec![
            CommandOutput::ok(""),
            CommandOutput::ok("main\n"),
            CommandOutput::ok(""),
            CommandOutput::ok(""),
            CommandOutput::ok(""),
            CommandOutput::ok(""),
            CommandOutput::ok(""),
        ]));

        run(
            &mut state,
            &runner,
            MutationRequest::Compact {
                gist_id: "g1".into(),
                label: "my gist".into(),
                count: 3,
            },
        );

        let calls = runner.calls();
        assert_eq!(calls.len(), 7);
        assert_eq!(calls[0].program, "git");
        assert_eq!(
            &calls[0].args[..2],
            ["clone", "https://gist.github.com/g1.git"]
        );
        assert!(calls[6].args.ends_with(&[
            "push".to_string(),
            "--force".to_string(),
            "origin".to_string(),
            "main".to_string()
        ]));
        assert!(!state.status.unwrap_or_default().contains("failed"));
    }

    /// With no such file in the catalog yet, the upload adds it to the gist.
    #[test]
    fn upload_of_a_file_new_to_the_gist_adds_it() {
        let mut state = initial_state();
        state.enter_upload_confirm(
            UploadDraft {
                original_content: "hello\n".into(),
                ..UploadDraft::fixture("g1", "new.txt", "/tmp/new.txt")
            },
            None,
        );
        let draft = state.upload_draft().cloned().unwrap();
        let runner = ok_runner(1);

        run(
            &mut state,
            &runner,
            MutationRequest::Upload(Box::new(draft)),
        );

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(&calls[0].args[..3], ["gist", "edit", "g1"]);
        assert!(
            calls[0].args.contains(&"--add".to_string()),
            "{:?}",
            calls[0].args
        );
        assert!(calls[0].args.last().unwrap().ends_with("new.txt"));
    }

    /// The one place pre-spawn navigation lives, pinned per request (behaviour unchanged by
    /// the workflow refactor; #476 tracks changing it for failures).
    #[test]
    fn leave_before_spawn_per_request() {
        let confirm = |state: &mut AppState, action| state.enter_confirm(action, String::new());

        // Delete from GistDetail: leaves Confirm and the deleted gist's detail.
        let mut state = initial_state();
        state.enter(Screen::Gists(Box::default()));
        state.enter(Screen::GistDetail(Box::default()));
        confirm(
            &mut state,
            PendingAction::Delete {
                gist_id: "g1".into(),
                label: "x".into(),
            },
        );
        leave_before_spawn(
            &mut state,
            &MutationRequest::Delete {
                gist_id: "g1".into(),
            },
        );
        assert!(state.screen.is_gists(), "{:?}", state.screen);

        // RemoveFile: a hard reset to the list.
        let mut state = initial_state();
        state.enter(Screen::Gists(Box::default()));
        leave_before_spawn(
            &mut state,
            &MutationRequest::RemoveFile {
                file: GistFileRef::id_name("g1", "a.txt"),
            },
        );
        assert_eq!(state.screen, Screen::List);
        assert!(state.nav_stack.is_empty());

        // Compact: back to where Confirm was opened.
        let mut state = initial_state();
        state.enter(Screen::Gists(Box::default()));
        confirm(
            &mut state,
            PendingAction::CompactGist {
                gist_id: "g1".into(),
                label: "x".into(),
                count: 2,
            },
        );
        leave_before_spawn(
            &mut state,
            &MutationRequest::Compact {
                gist_id: "g1".into(),
                label: "x".into(),
                count: 2,
            },
        );
        assert!(state.screen.is_gists());

        // Create: Confirm stays up until the result arrives.
        let mut state = initial_state();
        confirm(
            &mut state,
            PendingAction::Create {
                local_path: "/tmp/a.txt".into(),
            },
        );
        leave_before_spawn(
            &mut state,
            &MutationRequest::Create {
                local_path: "/tmp/a.txt".into(),
                public: false,
                description: String::new(),
            },
        );
        assert!(state.screen.is_confirm());

        // Description: editing ends and the input clears.
        let mut state = initial_state();
        state.editing_description = true;
        state.description_input = TextInput::from("typed");
        leave_before_spawn(
            &mut state,
            &MutationRequest::Description {
                gist_id: "g1".into(),
                description: "typed".into(),
            },
        );
        assert!(!state.editing_description);
        assert!(state.description_input.to_string().is_empty());
    }

    #[test]
    fn on_upload_replace_err_sets_status() {
        let mut state = initial_state();

        on_upload_replace(
            &mut state,
            Err("boom".into()),
            gist_file_ref("g1", "a.txt"),
            std::path::Path::new("/tmp/a.txt"),
            "hello",
            "hello",
        );

        assert_eq!(state.status.as_deref(), Some("upload failed: boom"));
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_upload_replace_ok_records_pin_and_marks_list_stale() {
        let _guard = crate::config::tests::ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", dir.path());

        let local_path = dir.path().join("a.txt");
        std::fs::write(&local_path, "hello").unwrap();
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
        let path = crate::config::config_path().unwrap();
        crate::config::save_config(&path, &config).unwrap();

        let mut state = initial_state();
        state.cwd = dir.path().to_path_buf();
        state.pinned = vec![mapping];
        let file = crate::domain::GistFileRef::id_name("g1", "a.txt");
        state.gist_content_store.insert(&file, "stale".into());
        state.gist_catalog.owned = vec![crate::domain::GistFile {
            raw_url: Some(
                "https://gist.githubusercontent.com/u/g1/raw/1111111111111111111111111111111111111111/a.txt"
                    .into(),
            ),
            ..crate::domain::GistFile::fixture("g1", "a.txt")
        }];
        // The local file is CRLF on disk; the upload sent LF.
        on_upload_replace(
            &mut state,
            Ok(()),
            gist_file_ref("g1", "a.txt"),
            &local_path,
            "hello",
            "hello\n",
        );

        assert!(state.gist_list_stale);
        assert_eq!(state.status.as_deref(), Some("Uploaded a.txt to gist g1"));
        assert!(matches!(
            state.gist_content_store.lookup(
                &state.gist_catalog,
                file,
                crate::tui::gist_content::FetchPolicy::PreferCache
            ),
            crate::tui::gist_content::ContentLookup::Miss(_)
        ));
        assert_eq!(state.pinned[0].direction, Some(SyncDirection::Upload));
        assert_eq!(
            state.pinned[0].last_seen_hash.as_deref(),
            Some(crate::domain::sha256_hex(b"hello").as_str())
        );
        // Issue #466: the remote baseline is the blob sha of the bytes sent, and the catalog
        // already shows it, so the pin reads as in sync before the refresh lands.
        let sent_sha = crate::domain::git_blob_sha1(b"hello\n");
        assert_eq!(
            state.pinned[0].remote_blob_sha.as_deref(),
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

        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn on_create_gist_err_resets_screen() {
        let mut state = initial_state();
        state.description_input.set("desc");
        state.enter_confirm(
            PendingAction::Create {
                local_path: PathBuf::from("a.txt"),
            },
            String::new(),
        );

        on_create_gist(&mut state, Err("boom".into()), PathBuf::from("a.txt"), true);

        assert_eq!(state.status.as_deref(), Some("create failed: boom"));
        assert!(matches!(state.screen, Screen::List));
        assert!(
            state.nav_stack.is_empty(),
            "nothing stale left behind the list (#475): {:?}",
            state.nav_stack
        );
        assert!(state.description_input.is_empty());
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_create_gist_ok_returns_to_list_and_marks_stale() {
        let mut state = initial_state();
        state.description_input.set("desc");
        state.screen = Screen::Confirm(Box::default());

        on_create_gist(&mut state, Ok(()), PathBuf::from("a.txt"), true);

        assert!(state.gist_list_stale);
        assert_eq!(
            state.status.as_deref(),
            Some("Created public gist from a.txt")
        );
        assert!(matches!(state.screen, Screen::List));
        assert!(state.description_input.is_empty());
    }

    #[test]
    fn on_delete_gist_err_sets_status() {
        let mut state = initial_state();
        let file = crate::domain::GistFileRef::id_name("g1", "a.txt");
        state.gist_content_store.insert(&file, "body".into());

        on_delete_gist(&mut state, Err("boom".into()), "g1".into());

        assert_eq!(state.status.as_deref(), Some("delete failed: boom"));
        assert!(!state.gist_list_stale);
        assert!(matches!(
            state.gist_content_store.lookup(
                &state.gist_catalog,
                file,
                crate::tui::gist_content::FetchPolicy::PreferCache
            ),
            crate::tui::gist_content::ContentLookup::Hit(_)
        ));
    }

    #[test]
    fn on_delete_gist_ok_marks_list_stale() {
        let mut state = initial_state();
        let deleted = crate::domain::GistFileRef::id_name("g1", "a.txt");
        let retained = crate::domain::GistFileRef::id_name("g2", "b.txt");
        state.gist_content_store.insert(&deleted, "deleted".into());
        state
            .gist_content_store
            .insert(&retained, "retained".into());

        on_delete_gist(&mut state, Ok(()), "g1".into());

        assert!(state.gist_list_stale);
        assert_eq!(state.status.as_deref(), Some("Deleted gist g1"));
        assert!(matches!(
            state.gist_content_store.lookup(
                &state.gist_catalog,
                deleted,
                crate::tui::gist_content::FetchPolicy::PreferCache
            ),
            crate::tui::gist_content::ContentLookup::Miss(_)
        ));
        assert!(matches!(
            state.gist_content_store.lookup(
                &state.gist_catalog,
                retained,
                crate::tui::gist_content::FetchPolicy::PreferCache
            ),
            crate::tui::gist_content::ContentLookup::Hit(_)
        ));
    }

    #[test]
    fn on_remove_file_err_sets_status() {
        let mut state = initial_state();

        on_remove_file(&mut state, Err("boom".into()), "g1".into(), "a.txt".into());

        assert_eq!(state.status.as_deref(), Some("remove failed: boom"));
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_remove_file_ok_drops_cache_and_marks_list_stale() {
        let mut state = initial_state();
        let file = crate::domain::GistFileRef::id_name("g1", "a.txt");
        state.gist_content_store.insert(&file, "body".into());

        on_remove_file(&mut state, Ok(()), "g1".into(), "a.txt".into());

        assert!(state.gist_list_stale);
        assert_eq!(state.status.as_deref(), Some("Removed a.txt from gist g1"));
        assert!(matches!(
            state.gist_content_store.lookup(
                &state.gist_catalog,
                file,
                crate::tui::gist_content::FetchPolicy::PreferCache
            ),
            crate::tui::gist_content::ContentLookup::Miss(_)
        ));
    }

    #[test]
    fn on_apply_description_err_sets_status() {
        let mut state = initial_state();

        on_apply_description(&mut state, Err("boom".into()), "g1".into());

        assert_eq!(
            state.status.as_deref(),
            Some("description update failed: boom")
        );
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_apply_description_ok_marks_list_stale() {
        let mut state = initial_state();

        on_apply_description(&mut state, Ok(()), "g1".into());

        assert!(state.gist_list_stale);
        assert_eq!(
            state.status.as_deref(),
            Some("Updated description for gist g1")
        );
    }

    #[test]
    fn on_compact_gist_err_sets_status() {
        let mut state = initial_state();

        on_compact_gist(&mut state, Err("boom".into()), "demo".into(), 3);

        assert_eq!(state.status.as_deref(), Some("compact failed: boom"));
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_compact_gist_ok_marks_list_stale() {
        let mut state = initial_state();

        on_compact_gist(&mut state, Ok(()), "demo".into(), 3);

        assert!(state.gist_list_stale);
        assert_eq!(
            state.status.as_deref(),
            Some("Compacted \"demo\" (3 → 1 revision)")
        );
    }

    #[test]
    fn on_gist_star_toggle_err_sets_status() {
        let mut state = initial_state();

        on_gist_star_toggle(&mut state, Err("boom".into()), "g1".into(), true);

        assert_eq!(state.status.as_deref(), Some("star toggle failed: boom"));
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_gist_star_toggle_ok_stars_and_marks_list_stale() {
        let mut state = initial_state();

        on_gist_star_toggle(&mut state, Ok(()), "g1".into(), true);

        assert!(state.gist_list_stale);
        assert!(state.gist_catalog.starred_ids.contains("g1"));
        assert_eq!(state.status.as_deref(), Some("starred g1"));
    }

    #[test]
    fn on_fork_gist_err_sets_status() {
        let mut state = initial_state();

        on_fork_gist(&mut state, Err("boom".into()), "g1".into());

        assert_eq!(state.status.as_deref(), Some("fork failed: boom"));
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn on_fork_gist_ok_marks_list_stale() {
        let mut state = initial_state();

        on_fork_gist(&mut state, Ok(()), "g1".into());

        assert!(state.gist_list_stale);
        assert_eq!(state.status.as_deref(), Some("forked g1 into your account"));
    }
}
