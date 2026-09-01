//! The **Gist revision workflow** (issue #430): one interface for every history, diff,
//! preview, and restore request the TUI can make against a Gist's revisions.
//!
//! Callers hand [`dispatch`] one plain-data [`RevisionRequest`]. This module stages the
//! action job, runs the remote work through the injected [`crate::actions::CommandRunner`]
//! seam, and hands a request-specific typed result to the screen-owned apply handler that
//! the job carries. It owns fallback ordering, absent-file semantics, the per-buffer text
//! size gate, restore JSON construction and scratch lifetime, and restore follow-up policy.
//!
//! It deliberately does **not** own screen knowledge. Eligibility guards, selection,
//! ownership and previewability checks, return-entry capture, and intent-time labels stay
//! in `screens::revisions` / `screens::confirm`; navigation, status, and every other piece
//! of observable state application stays in those screens' `on_*` handlers.

use super::bg::{write_scratch_file, ActionJobKind, ActionJobSpec, Jobs};
use super::{screens, AppState, DeferredEntry};
use crate::actions::CommandRunner;
use crate::domain::{GistFileRef, GistRevision};

/// The Gist file a revision request acts on, plus the owner identity the canonical
/// revision raw URL needs. Shared by every request kind; versions, labels, the return
/// entry, and restore content stay request-specific.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionTarget {
    pub file: GistFileRef,
    pub owner_login: String,
}

impl RevisionTarget {
    pub fn new(file: GistFileRef, owner_login: String) -> Self {
        Self { file, owner_login }
    }

    fn gist_id(&self) -> &str {
        &self.file.gist_id
    }

    fn filename(&self) -> &str {
        &self.file.filename
    }

    /// Non-content identity for the job kind (no raw URL, matching every other
    /// `ActionJobKind` payload).
    fn identity(&self) -> GistFileRef {
        GistFileRef::id_name(self.file.gist_id.clone(), self.file.filename.clone())
    }
}

/// Every Gist revision request, as plain comparable data (ADR-0002): no closures, no
/// runners, no job handles.
#[derive(Debug, PartialEq, Eq)]
pub enum RevisionRequest {
    /// Load (or reload) the revision history of one Gist.
    FetchHistory { gist_id: String },
    /// Compare the selected revision with the one before it. A missing parent version is
    /// the initial revision, and compares against empty content.
    DiffAdjacent {
        entry: DeferredEntry,
        target: RevisionTarget,
        child_version: String,
        parent_version: Option<String>,
        old_label: String,
        new_label: String,
    },
    /// Compare the selected revision with the file's current content.
    DiffAgainstCurrent {
        entry: DeferredEntry,
        target: RevisionTarget,
        version: String,
        old_label: String,
        new_label: String,
    },
    /// Fetch both sides of a restore so the user can confirm it.
    PreviewRestore {
        entry: DeferredEntry,
        target: RevisionTarget,
        version: String,
        version_label: String,
    },
    /// Write the confirmed historical content back as a **new** Gist revision.
    ExecuteRestore {
        target: RevisionTarget,
        content: String,
    },
}

/// Semantic identity of a staged revision job, grouped under one workflow-owned kind so
/// the recording action-spawner adapter can still tell revision jobs apart by kind,
/// progress, and non-content payload without invoking `gh` (issue #422).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionJobKind {
    FetchHistory {
        gist_id: String,
    },
    DiffAdjacent {
        file: GistFileRef,
        child_version: String,
        parent_version: Option<String>,
    },
    DiffAgainstCurrent {
        file: GistFileRef,
        version: String,
    },
    PreviewRestore {
        file: GistFileRef,
        version: String,
    },
    ExecuteRestore {
        file: GistFileRef,
    },
}

/// What a successful restore changed, and what must be refreshed because of it. The
/// workflow decides this policy; `screens::revisions` only applies what it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreApplied {
    /// The Gist file whose cached content is now stale.
    pub file: GistFileRef,
    /// What the new Gist revision made stale, in the order `Jobs::absorb` consumes it.
    pub refresh: Vec<RestoreRefresh>,
}

/// One follow-up a completed restore requires. The screen marks it; only `Jobs::absorb`
/// starts the work (issue #383).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreRefresh {
    /// The Gist catalog: a restore is a new Gist revision, so metadata moved.
    GistCatalog,
    /// The revision history of the Gist the Revisions screen is showing.
    RevisionHistory,
}

/// Stage one revision request: build its job spec, run its remote work through the
/// registry's command runner, and let `Jobs` absorb the typed result into the screen
/// handler that owns the state it touches.
pub(super) fn dispatch(jobs: &mut Jobs, state: &mut AppState, request: RevisionRequest) {
    match request {
        RevisionRequest::FetchHistory { gist_id } => {
            let runner = jobs.command_runner();
            let spec = job_spec(
                RevisionJobKind::FetchHistory {
                    gist_id: gist_id.clone(),
                },
                "Loading revisions…",
            );
            jobs.spawn_action(
                state,
                spec,
                move || {
                    let result = fetch_history(runner.as_ref(), &gist_id);
                    (result, gist_id)
                },
                move |(result, gist_id), state| {
                    screens::revisions::on_revisions_fetched(state, gist_id, result)
                },
            );
        }
        RevisionRequest::DiffAdjacent {
            entry,
            target,
            child_version,
            parent_version,
            old_label,
            new_label,
        } => {
            let runner = jobs.command_runner();
            let spec = job_spec(
                RevisionJobKind::DiffAdjacent {
                    file: target.identity(),
                    child_version: child_version.clone(),
                    parent_version: parent_version.clone(),
                },
                "Loading diff…",
            );
            jobs.spawn_action(
                state,
                spec,
                move || {
                    adjacent_pair(
                        runner.as_ref(),
                        &target,
                        &child_version,
                        parent_version.as_deref(),
                    )
                },
                move |result, state| {
                    screens::diff::on_revision_diff(state, entry, result, old_label, new_label)
                },
            );
        }
        RevisionRequest::DiffAgainstCurrent {
            entry,
            target,
            version,
            old_label,
            new_label,
        } => {
            let runner = jobs.command_runner();
            let spec = job_spec(
                RevisionJobKind::DiffAgainstCurrent {
                    file: target.identity(),
                    version: version.clone(),
                },
                "Loading diff…",
            );
            jobs.spawn_action(
                state,
                spec,
                move || selected_and_current(runner.as_ref(), &target, &version),
                move |result, state| {
                    screens::diff::on_revision_diff(state, entry, result, old_label, new_label)
                },
            );
        }
        RevisionRequest::PreviewRestore {
            entry,
            target,
            version,
            version_label,
        } => {
            let runner = jobs.command_runner();
            let spec = job_spec(
                RevisionJobKind::PreviewRestore {
                    file: target.identity(),
                    version: version.clone(),
                },
                "Loading revision…",
            );
            let gist_id = target.gist_id().to_string();
            let filename = target.filename().to_string();
            jobs.spawn_action(
                state,
                spec,
                {
                    let version = version.clone();
                    move || selected_and_current(runner.as_ref(), &target, &version)
                },
                move |result, state| {
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
        RevisionRequest::ExecuteRestore { target, content } => {
            // Scratch preparation is synchronous and happens *before* the spawn, so a
            // failure keeps its own status wording and neither starts nor supersedes a
            // background job. On success the `ScratchDir` moves into the worker, whose
            // RAII drop cleans up after success, failure, or an ignored completion
            // (issue #275).
            let body = crate::actions::restore_revision_json(target.filename(), &content);
            let Some((scratch, json_path)) = write_scratch_file(
                state,
                "restore",
                "restore.json",
                "restore payload",
                body.as_bytes(),
            ) else {
                return;
            };
            let plan = crate::actions::restore_revision_command(target.gist_id(), &json_path);
            let runner = jobs.command_runner();
            let file = target.identity();
            let spec = job_spec(
                RevisionJobKind::ExecuteRestore { file: file.clone() },
                "Restoring revision…",
            );
            jobs.spawn_action(
                state,
                spec,
                move || {
                    let result = crate::actions::run_command(runner.as_ref(), &plan)
                        .map(|_| RestoreApplied {
                            file,
                            refresh: vec![
                                RestoreRefresh::GistCatalog,
                                RestoreRefresh::RevisionHistory,
                            ],
                        })
                        .map_err(|e| e.to_string());
                    drop(scratch);
                    result
                },
                move |result, state| screens::revisions::on_restore_revision_done(state, result),
            );
        }
    }
}

fn job_spec(kind: RevisionJobKind, progress: &str) -> ActionJobSpec {
    ActionJobSpec::new(ActionJobKind::Revision(kind), progress)
}

// ---- remote work ---------------------------------------------------------------

fn fetch_history(
    runner: &dyn CommandRunner,
    gist_id: &str,
) -> std::result::Result<Vec<GistRevision>, String> {
    crate::gh::fetch_gist_commits_json(runner, gist_id)
        .map_err(|e| e.to_string())
        .and_then(|raw| crate::gh::parse_gist_commits_json(&raw).map_err(|e| e.to_string()))
}

/// Selected revision vs. its parent. The child is fetched first, then the parent; a
/// missing parent version *is* the initial revision and compares against empty content.
/// Both sides keep the optional-absence rule, so a file added or removed at that point in
/// history still reads as a whole-file addition or deletion.
fn adjacent_pair(
    runner: &dyn CommandRunner,
    target: &RevisionTarget,
    child_version: &str,
    parent_version: Option<&str>,
) -> std::result::Result<(String, String), String> {
    let new_content = historical_text_optional(runner, target, child_version)?;
    let old_content = match parent_version {
        Some(parent) => historical_text_optional(runner, target, parent)?,
        None => String::new(),
    };
    Ok((old_content, new_content))
}

/// Selected revision vs. current content. The historical side is fetched first and is
/// **required** — its absence stays an error rather than empty content — then the current
/// side, which keeps the gist-view-to-raw-URL fallback.
fn selected_and_current(
    runner: &dyn CommandRunner,
    target: &RevisionTarget,
    version: &str,
) -> std::result::Result<(String, String), String> {
    let old_content = historical_text(runner, target, version)?;
    let new_content = current_text(runner, target)?;
    Ok((old_content, new_content))
}

fn historical_text(
    runner: &dyn CommandRunner,
    target: &RevisionTarget,
    version: &str,
) -> std::result::Result<String, String> {
    ensure_within_size_limit(
        crate::gh::fetch_revision_file_text(
            runner,
            target.gist_id(),
            version,
            target.filename(),
            &target.owner_login,
        )
        .map_err(|e| e.to_string())?,
    )
}

fn historical_text_optional(
    runner: &dyn CommandRunner,
    target: &RevisionTarget,
    version: &str,
) -> std::result::Result<String, String> {
    ensure_within_size_limit(
        crate::gh::fetch_revision_file_text_optional(
            runner,
            target.gist_id(),
            version,
            target.filename(),
            &target.owner_login,
        )
        .map_err(|e| e.to_string())?,
    )
}

fn current_text(
    runner: &dyn CommandRunner,
    target: &RevisionTarget,
) -> std::result::Result<String, String> {
    ensure_within_size_limit(
        crate::gh::fetch_gist_file_content(
            runner,
            target.gist_id(),
            target.filename(),
            target.file.raw_url.as_deref(),
        )
        .map_err(|e| e.to_string())?,
    )
}

/// Cap every fetched buffer the same way live gist content is capped (issue #222). Applied
/// independently to each side of a two-content workflow.
fn ensure_within_size_limit(content: String) -> std::result::Result<String, String> {
    crate::domain::ensure_text_size(content.len() as u64)?;
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_support::SeqRunner;
    use crate::actions::CommandOutput;
    use crate::domain::{GistCatalog, GistFile};
    use crate::tui::bg::LoopFlow;
    use crate::tui::gist_content::{ContentLookup, FetchPolicy};
    use crate::tui::test_support::state_with_gists;
    use crate::tui::{initial_state, ListCursor, PendingAction, RevisionState, Screen};
    use std::sync::Arc;

    /// Drive one request all the way through: stage it, execute the real worker inline,
    /// and let `Jobs::absorb` apply its result through the generation guard. This is the
    /// workflow's test seam — no step of it is bypassed.
    fn run(state: &mut AppState, runner: &Arc<SeqRunner>, request: RevisionRequest) -> LoopFlow {
        let mut jobs = Jobs::inline(&GistCatalog::default(), runner.clone());
        dispatch(&mut jobs, state, request);
        jobs.absorb(state, &None).expect("absorb")
    }

    fn scripted(outputs: Vec<CommandOutput>) -> Arc<SeqRunner> {
        Arc::new(SeqRunner::new(outputs))
    }

    fn commits_json() -> String {
        r#"[{"version":"v2","committed_at":"2026-06-10T00:00:00Z","user":{"login":"alice"},
             "change_status":{"total":1,"additions":1,"deletions":0}},
            {"version":"v1","committed_at":"2026-06-01T00:00:00Z","user":{"login":"alice"},
             "change_status":{"total":2,"additions":2,"deletions":0}}]"#
            .to_string()
    }

    fn revision_json(filename: &str, content: &str) -> String {
        serde_json::json!({ "files": { filename: { "filename": filename, "content": content } } })
            .to_string()
    }

    fn revisions_screen(gist_id: &str) -> AppState {
        let mut state = state_with_gists();
        state.screen = Screen::Revisions(Box::new(RevisionState {
            gist_id: Some(gist_id.into()),
            target_file: "a.txt".into(),
            ..RevisionState::default()
        }));
        state
    }

    fn target() -> RevisionTarget {
        RevisionTarget::new(GistFileRef::id_name("g1", "a.txt"), "alice".into())
    }

    fn entry(state: &AppState) -> DeferredEntry {
        state.defer_entry()
    }

    // ---- fetch history ---------------------------------------------------

    #[test]
    fn fetch_history_loads_entries_and_clears_the_error_state() {
        let mut state = revisions_screen("g1");
        state.revision_mut().expect("revisions").fetch_error = Some("old".into());
        let runner = scripted(vec![CommandOutput::ok(commits_json())]);

        run(
            &mut state,
            &runner,
            RevisionRequest::FetchHistory {
                gist_id: "g1".into(),
            },
        );

        let rev = state.revision().expect("revisions");
        assert_eq!(rev.entries.as_ref().map(Vec::len), Some(2));
        assert!(rev.fetch_error.is_none());
        assert_eq!(
            runner.calls(),
            vec![crate::gh::gist_commits_plan("g1")],
            "history comes from the commits API"
        );
    }

    #[test]
    fn fetch_history_parse_failure_becomes_the_screen_fetch_error() {
        let mut state = revisions_screen("g1");
        let runner = scripted(vec![CommandOutput::ok("not json")]);

        run(
            &mut state,
            &runner,
            RevisionRequest::FetchHistory {
                gist_id: "g1".into(),
            },
        );

        let rev = state.revision().expect("revisions");
        assert_eq!(rev.entries, Some(Vec::new()));
        assert!(rev
            .fetch_error
            .as_deref()
            .is_some_and(|e| e.contains("parse gh gist commits JSON")));
    }

    #[test]
    fn fetch_history_for_another_gist_is_ignored() {
        let mut state = revisions_screen("g1");
        let runner = scripted(vec![CommandOutput::ok(commits_json())]);

        let flow = run(
            &mut state,
            &runner,
            RevisionRequest::FetchHistory {
                gist_id: "other".into(),
            },
        );

        assert!(
            matches!(flow, LoopFlow::SkipIteration),
            "a stale gist aborts the rest of the tick"
        );
        assert!(
            state.revision().expect("revisions").entries.is_none(),
            "a response for a different gist must not replace the visible screen"
        );
    }

    #[test]
    fn fetch_history_clamps_an_out_of_range_cursor_and_clears_its_stale_hscroll() {
        let mut state = revisions_screen("g1");
        state.revision_mut().expect("revisions").cursor = ListCursor {
            index: 3,
            hscroll: 7,
        };
        let runner = scripted(vec![CommandOutput::ok(commits_json())]);

        run(
            &mut state,
            &runner,
            RevisionRequest::FetchHistory {
                gist_id: "g1".into(),
            },
        );

        let rev = state.revision().expect("revisions");
        assert_eq!(rev.cursor.index, 1, "clamped into range");
        assert_eq!(rev.cursor.hscroll, 0, "stale hscroll cleared");
    }

    #[test]
    fn fetch_history_keeps_an_in_range_cursor() {
        let mut state = revisions_screen("g1");
        state.revision_mut().expect("revisions").cursor = ListCursor {
            index: 1,
            hscroll: 4,
        };
        let runner = scripted(vec![CommandOutput::ok(commits_json())]);

        run(
            &mut state,
            &runner,
            RevisionRequest::FetchHistory {
                gist_id: "g1".into(),
            },
        );

        let rev = state.revision().expect("revisions");
        assert_eq!(rev.cursor.index, 1);
        assert_eq!(rev.cursor.hscroll, 4);
    }

    // ---- adjacent diff ---------------------------------------------------

    #[test]
    fn adjacent_diff_fetches_the_child_then_its_parent_and_enters_diff() {
        let mut state = revisions_screen("g1");
        let e = entry(&state);
        let runner = scripted(vec![
            CommandOutput::ok(revision_json("a.txt", "new\n")),
            CommandOutput::ok(revision_json("a.txt", "old\n")),
        ]);

        run(
            &mut state,
            &runner,
            RevisionRequest::DiffAdjacent {
                entry: e,
                target: target(),
                child_version: "v2".into(),
                parent_version: Some("v1".into()),
                old_label: "revision v1".into(),
                new_label: "revision v2".into(),
            },
        );

        assert_eq!(
            runner.calls(),
            vec![
                crate::gh::gist_revision_plan("g1", "v2"),
                crate::gh::gist_revision_plan("g1", "v1"),
            ],
            "selected child first, then its parent"
        );
        let diff = state.diff().expect("Screen::Diff");
        assert!(diff.body.text.contains("revision v1"));
        assert!(diff.body.text.contains("+new"));
        assert!(!diff.identical);
    }

    #[test]
    fn the_initial_revision_compares_against_empty_content() {
        let mut state = revisions_screen("g1");
        let e = entry(&state);
        let runner = scripted(vec![CommandOutput::ok(revision_json("a.txt", "first\n"))]);

        run(
            &mut state,
            &runner,
            RevisionRequest::DiffAdjacent {
                entry: e,
                target: target(),
                child_version: "v1".into(),
                parent_version: None,
                old_label: "(initial)".into(),
                new_label: "revision v1".into(),
            },
        );

        assert_eq!(
            runner.calls(),
            vec![crate::gh::gist_revision_plan("g1", "v1")],
            "no parent version means no second fetch"
        );
        let diff = state.diff().expect("Screen::Diff");
        assert!(diff.body.text.contains("+first"));
    }

    /// An optional side missing from a revision snapshot is empty content, not an error,
    /// on either side of the comparison.
    #[test]
    fn an_absent_historical_file_reads_as_empty_on_either_adjacent_side() {
        for absent_child in [true, false] {
            let mut state = revisions_screen("g1");
            let e = entry(&state);
            let present = CommandOutput::ok(revision_json("a.txt", "body\n"));
            let missing = CommandOutput::ok(revision_json("other.txt", "unrelated\n"));
            let runner = if absent_child {
                scripted(vec![missing, present])
            } else {
                scripted(vec![present, missing])
            };

            run(
                &mut state,
                &runner,
                RevisionRequest::DiffAdjacent {
                    entry: e,
                    target: target(),
                    child_version: "v2".into(),
                    parent_version: Some("v1".into()),
                    old_label: "revision v1".into(),
                    new_label: "revision v2".into(),
                },
            );

            let diff = state.diff().expect("Screen::Diff");
            let marker = if absent_child { "-body" } else { "+body" };
            assert!(
                diff.body.text.contains(marker),
                "absent side must read as empty content ({marker})"
            );
        }
    }

    // ---- the size gate ---------------------------------------------------

    /// The cap applies independently to **every** fetched buffer: both sides of the
    /// adjacent comparison, and both sides of each selected-versus-current workflow. An
    /// oversized side fails the whole request — no Diff, no Confirm (issue #222).
    #[test]
    fn the_size_gate_applies_to_every_side_of_every_two_content_workflow() {
        let big = "x".repeat(crate::domain::MAX_TEXT_FILE_BYTES as usize + 1);
        let small = revision_json("a.txt", "small\n");

        // (label, scripted outputs, request builder) — one case per fetched buffer.
        #[allow(clippy::type_complexity)]
        let cases: Vec<(
            &str,
            Vec<CommandOutput>,
            fn(DeferredEntry) -> RevisionRequest,
        )> = vec![
            (
                "adjacent child",
                vec![CommandOutput::ok(revision_json("a.txt", &big))],
                |entry| RevisionRequest::DiffAdjacent {
                    entry,
                    target: target(),
                    child_version: "v2".into(),
                    parent_version: Some("v1".into()),
                    old_label: "revision v1".into(),
                    new_label: "revision v2".into(),
                },
            ),
            (
                "adjacent parent",
                vec![
                    CommandOutput::ok(small.clone()),
                    CommandOutput::ok(revision_json("a.txt", &big)),
                ],
                |entry| RevisionRequest::DiffAdjacent {
                    entry,
                    target: target(),
                    child_version: "v2".into(),
                    parent_version: Some("v1".into()),
                    old_label: "revision v1".into(),
                    new_label: "revision v2".into(),
                },
            ),
            (
                "selected-versus-current historical",
                vec![CommandOutput::ok(revision_json("a.txt", &big))],
                |entry| RevisionRequest::DiffAgainstCurrent {
                    entry,
                    target: target(),
                    version: "v1".into(),
                    old_label: "revision v1".into(),
                    new_label: "current a.txt".into(),
                },
            ),
            (
                "selected-versus-current current",
                vec![CommandOutput::ok(small.clone()), CommandOutput::ok(&big)],
                |entry| RevisionRequest::DiffAgainstCurrent {
                    entry,
                    target: target(),
                    version: "v1".into(),
                    old_label: "revision v1".into(),
                    new_label: "current a.txt".into(),
                },
            ),
            (
                "restore-preview historical",
                vec![CommandOutput::ok(revision_json("a.txt", &big))],
                |entry| RevisionRequest::PreviewRestore {
                    entry,
                    target: target(),
                    version: "v1".into(),
                    version_label: "v1 (3d ago)".into(),
                },
            ),
            (
                "restore-preview current",
                vec![CommandOutput::ok(small), CommandOutput::ok(&big)],
                |entry| RevisionRequest::PreviewRestore {
                    entry,
                    target: target(),
                    version: "v1".into(),
                    version_label: "v1 (3d ago)".into(),
                },
            ),
        ];

        for (side, outputs, build) in cases {
            let mut state = revisions_screen("g1");
            let request = build(entry(&state));
            run(&mut state, &scripted(outputs), request);

            assert!(state.screen.is_revisions(), "{side}: must not navigate");
            assert_eq!(
                state.status.as_deref(),
                Some(
                    crate::domain::ensure_text_size(crate::domain::MAX_TEXT_FILE_BYTES + 1)
                        .unwrap_err()
                        .as_str()
                ),
                "{side}: reports the oversize error"
            );
        }
    }

    // ---- selected vs current ---------------------------------------------

    #[test]
    fn selected_versus_current_fetches_history_first_then_current_content() {
        let mut state = revisions_screen("g1");
        let e = entry(&state);
        let runner = scripted(vec![
            CommandOutput::ok(revision_json("a.txt", "old\n")),
            CommandOutput::ok("current\n"),
        ]);

        run(
            &mut state,
            &runner,
            RevisionRequest::DiffAgainstCurrent {
                entry: e,
                target: target(),
                version: "v1".into(),
                old_label: "revision v1".into(),
                new_label: "current a.txt".into(),
            },
        );

        assert_eq!(
            runner.calls(),
            vec![
                crate::gh::gist_revision_plan("g1", "v1"),
                crate::gh::gist_view_plan("g1", "a.txt"),
            ]
        );
        let diff = state.diff().expect("Screen::Diff");
        assert!(diff.body.text.contains("current a.txt"));
    }

    #[test]
    fn a_missing_historical_file_stays_an_error_for_selected_versus_current() {
        let mut state = revisions_screen("g1");
        let e = entry(&state);
        let runner = scripted(vec![CommandOutput::ok(revision_json("other.txt", "x"))]);

        run(
            &mut state,
            &runner,
            RevisionRequest::DiffAgainstCurrent {
                entry: e,
                target: target(),
                version: "v1".into(),
                old_label: "revision v1".into(),
                new_label: "current a.txt".into(),
            },
        );

        assert!(state.diff().is_none());
        assert_eq!(
            state.status.as_deref(),
            Some("a.txt not present in this revision")
        );
    }

    /// The historical fetch keeps its entry-raw-URL fallback, and the current fetch keeps
    /// its command-to-raw-URL fallback.
    #[test]
    fn both_sides_keep_their_raw_url_fallbacks() {
        let mut state = revisions_screen("g1");
        let e = entry(&state);
        let entry_raw = "https://gist.githubusercontent.com/alice/g1/raw/v1/a.txt";
        let current_raw = "https://gist.githubusercontent.com/alice/g1/raw/a.txt";
        let truncated = serde_json::json!({
            "files": { "a.txt": { "filename": "a.txt", "truncated": true, "raw_url": entry_raw } }
        })
        .to_string();
        let runner = scripted(vec![
            CommandOutput::ok(truncated),
            CommandOutput::ok("old from raw\n"),
            CommandOutput::err("HTTP 502"),
            CommandOutput::ok("current from raw\n"),
        ]);
        let target = RevisionTarget::new(
            GistFileRef::new("g1", "a.txt", Some(current_raw.into())),
            "alice".into(),
        );

        run(
            &mut state,
            &runner,
            RevisionRequest::DiffAgainstCurrent {
                entry: e,
                target,
                version: "v1".into(),
                old_label: "revision v1".into(),
                new_label: "current a.txt".into(),
            },
        );

        assert_eq!(
            runner.calls(),
            vec![
                crate::gh::gist_revision_plan("g1", "v1"),
                crate::gh::raw_url_fetch_plan(entry_raw),
                crate::gh::gist_view_plan("g1", "a.txt"),
                crate::gh::raw_url_fetch_plan(current_raw),
            ]
        );
        let diff = state.diff().expect("Screen::Diff");
        assert!(diff.body.text.contains("+current from raw"));
    }

    /// A failed revision API call falls back to the owner-aware canonical raw URL.
    #[test]
    fn a_failed_revision_api_falls_back_to_the_canonical_raw_url() {
        let mut state = revisions_screen("g1");
        let e = entry(&state);
        let runner = scripted(vec![
            CommandOutput::err("HTTP 502"),
            CommandOutput::ok("old\n"),
            CommandOutput::ok("current\n"),
        ]);

        run(
            &mut state,
            &runner,
            RevisionRequest::DiffAgainstCurrent {
                entry: e,
                target: target(),
                version: "v1".into(),
                old_label: "revision v1".into(),
                new_label: "current a.txt".into(),
            },
        );

        assert_eq!(
            runner.calls()[1],
            crate::gh::raw_url_fetch_plan(&crate::gh::build_gist_revision_raw_url(
                "alice", "g1", "v1", "a.txt"
            ))
        );
        assert!(state.diff().is_some());
    }

    /// A superseded generation applies nothing: no navigation, no status change.
    #[test]
    fn a_superseded_diff_result_is_dropped() {
        let mut state = revisions_screen("g1");
        let e = entry(&state);
        let runner = scripted(vec![
            CommandOutput::ok(revision_json("a.txt", "old\n")),
            CommandOutput::ok("current\n"),
        ]);
        let mut jobs = Jobs::inline(&GistCatalog::default(), runner.clone());

        dispatch(
            &mut jobs,
            &mut state,
            RevisionRequest::DiffAgainstCurrent {
                entry: e,
                target: target(),
                version: "v1".into(),
                old_label: "revision v1".into(),
                new_label: "current a.txt".into(),
            },
        );
        state.invalidate_bg_task();
        jobs.absorb(&mut state, &None).expect("absorb");

        assert!(
            state.screen.is_revisions(),
            "a newer intent wins — the stale diff must not navigate"
        );
    }

    // ---- restore preview -------------------------------------------------

    #[test]
    fn an_identical_restore_preview_reports_that_nothing_needs_restoring() {
        let mut state = revisions_screen("g1");
        let e = entry(&state);
        let runner = scripted(vec![
            CommandOutput::ok(revision_json("a.txt", "same\n")),
            CommandOutput::ok("same\n"),
        ]);

        let flow = run(
            &mut state,
            &runner,
            RevisionRequest::PreviewRestore {
                entry: e,
                target: target(),
                version: "v1".into(),
                version_label: "v1 (3d ago)".into(),
            },
        );

        assert!(
            matches!(flow, LoopFlow::SkipIteration),
            "nothing to restore aborts the rest of the tick"
        );
        assert_eq!(
            state.status.as_deref(),
            Some("revision matches current — nothing to restore")
        );
        assert!(!state.screen.is_confirm());
    }

    #[test]
    fn a_changed_restore_preview_enters_confirm_with_the_historical_content() {
        let mut state = revisions_screen("g1");
        let e = entry(&state);
        let runner = scripted(vec![
            CommandOutput::ok(revision_json("a.txt", "old\n")),
            CommandOutput::ok("current\n"),
        ]);

        run(
            &mut state,
            &runner,
            RevisionRequest::PreviewRestore {
                entry: e,
                target: target(),
                version: "v1".into(),
                version_label: "v1 (3d ago)".into(),
            },
        );

        assert!(matches!(
            state.pending_action(),
            Some(PendingAction::RestoreRevision { content, version_label, .. })
                if content == "old\n" && version_label == "v1 (3d ago)"
        ));
    }

    #[test]
    fn a_restore_preview_reports_a_historical_or_current_fetch_failure() {
        for outputs in [
            vec![
                CommandOutput::err("HTTP 500"),
                CommandOutput::err("HTTP 500"),
            ],
            vec![
                CommandOutput::ok(revision_json("a.txt", "old\n")),
                CommandOutput::err("gist view failed"),
            ],
        ] {
            let mut state = revisions_screen("g1");
            let e = entry(&state);
            let runner = scripted(outputs);

            run(
                &mut state,
                &runner,
                RevisionRequest::PreviewRestore {
                    entry: e,
                    target: target(),
                    version: "v1".into(),
                    version_label: "v1 (3d ago)".into(),
                },
            );

            assert!(!state.screen.is_confirm());
            assert!(state.status.is_some());
        }
    }

    // ---- execute restore -------------------------------------------------

    fn confirmed_restore(state: &mut AppState) {
        state.enter_confirm(
            PendingAction::RestoreRevision {
                gist_id: "g1".into(),
                filename: "a.txt".into(),
                version: "v1".into(),
                version_label: "v1 (3d ago)".into(),
                content: "old\n".into(),
            },
            String::new(),
        );
    }

    #[test]
    fn a_successful_restore_patches_the_gist_and_refreshes_everything_it_invalidated() {
        let mut state = revisions_screen("g1");
        state.revision_mut().expect("revisions").cursor = ListCursor {
            index: 3,
            hscroll: 2,
        };
        state.revision_mut().expect("revisions").entries = Some(Vec::new());
        state.revision_mut().expect("revisions").fetch_error = Some("old".into());
        confirmed_restore(&mut state);
        let file = GistFileRef::id_name("g1", "a.txt");
        state.gist_content_store.insert(&file, "stale".into());
        let runner = scripted(vec![CommandOutput::ok("")]);

        run(
            &mut state,
            &runner,
            RevisionRequest::ExecuteRestore {
                target: target(),
                content: "old\n".into(),
            },
        );

        let recorded = runner.recorded();
        assert_eq!(recorded[0].plan.program, "gh");
        assert_eq!(
            recorded[0].plan.args[..4],
            ["api", "--method", "PATCH", "/gists/g1"],
            "restore patches the gist — it never rewrites an existing revision"
        );
        assert_eq!(
            recorded[0].input_body.as_deref(),
            Some(crate::actions::restore_revision_json("a.txt", "old\n").as_str()),
            "the payload rewrites one file's content"
        );

        assert_eq!(
            state.status.as_deref(),
            Some("Restored a.txt from old revision (new revision created)")
        );
        assert!(state.screen.is_revisions());
        let rev = state.revision().expect("revisions");
        assert_eq!(rev.cursor, ListCursor::default());
        assert!(rev.entries.is_none());
        assert!(rev.fetch_error.is_none());
        assert!(matches!(
            state
                .gist_content_store
                .lookup(&state.gist_catalog, file, FetchPolicy::PreferCache),
            ContentLookup::Miss(_)
        ));
    }

    /// `Jobs::absorb` consumes the refresh markers apply set, in order: the Gist catalog
    /// first, then that Gist's revision history.
    #[test]
    fn a_successful_restore_starts_both_follow_ups_in_order() {
        let mut state = revisions_screen("g1");
        confirmed_restore(&mut state);
        let runner = scripted(vec![
            CommandOutput::ok(""),
            CommandOutput::ok(commits_json()),
        ]);

        run(
            &mut state,
            &runner,
            RevisionRequest::ExecuteRestore {
                target: target(),
                content: "old\n".into(),
            },
        );

        assert!(!state.gist_list_stale, "the catalog marker was consumed");
        assert!(state.revisions_stale.is_none(), "the history marker too");
        assert!(state.loading, "the catalog refresh started");
        assert_eq!(
            runner.calls().last(),
            Some(&crate::gh::gist_commits_plan("g1")),
            "the revision history refetch ran after the restore"
        );
    }

    #[test]
    fn a_failed_restore_stays_on_confirm_and_marks_no_refresh() {
        let mut state = revisions_screen("g1");
        confirmed_restore(&mut state);
        let runner = scripted(vec![CommandOutput::err("HTTP 422")]);

        run(
            &mut state,
            &runner,
            RevisionRequest::ExecuteRestore {
                target: target(),
                content: "old\n".into(),
            },
        );

        assert_eq!(state.status.as_deref(), Some("restore failed: HTTP 422"));
        assert!(
            state.screen.is_confirm(),
            "the confirmation keeps its context"
        );
        assert!(!state.gist_list_stale);
        assert!(state.revisions_stale.is_none());
    }

    /// The scratch directory is the worker's, and its RAII drop runs on every completed
    /// path — success, command failure, and a completion the generation guard ignores.
    #[test]
    fn the_restore_scratch_is_removed_after_every_completed_worker_path() {
        for (outputs, supersede) in [
            (vec![CommandOutput::ok("")], false),
            (vec![CommandOutput::err("HTTP 422")], false),
            (vec![CommandOutput::ok("")], true),
        ] {
            let mut state = revisions_screen("g1");
            confirmed_restore(&mut state);
            let runner = scripted(outputs);
            let mut jobs = Jobs::inline(&GistCatalog::default(), runner.clone());

            dispatch(
                &mut jobs,
                &mut state,
                RevisionRequest::ExecuteRestore {
                    target: target(),
                    content: "old\n".into(),
                },
            );
            if supersede {
                state.invalidate_bg_task();
            }
            jobs.absorb(&mut state, &None).expect("absorb");

            let recorded = runner.recorded();
            let json_path = recorded
                .first()
                .and_then(|c| crate::actions::test_support::input_path(&c.plan))
                .map(std::path::PathBuf::from)
                .expect("restore ran with an --input payload");
            assert!(!json_path.exists(), "the payload file is cleaned up");
            assert!(
                !json_path.parent().expect("scratch dir").exists(),
                "the scratch directory is cleaned up"
            );
            if supersede {
                assert!(
                    state.screen.is_confirm(),
                    "a superseded restore applies nothing"
                );
            }
        }
    }

    // ---- job identity ----------------------------------------------------

    /// Semantic revision job identity stays observable without invoking `gh` (issue #422).
    #[test]
    fn revision_jobs_stage_their_kind_and_progress_label() {
        let cases: Vec<(RevisionRequest, ActionJobSpec)> = vec![
            (
                RevisionRequest::FetchHistory {
                    gist_id: "g1".into(),
                },
                job_spec(
                    RevisionJobKind::FetchHistory {
                        gist_id: "g1".into(),
                    },
                    "Loading revisions…",
                ),
            ),
            (
                RevisionRequest::DiffAdjacent {
                    entry: initial_state().defer_entry(),
                    target: target(),
                    child_version: "v2".into(),
                    parent_version: Some("v1".into()),
                    old_label: String::new(),
                    new_label: String::new(),
                },
                job_spec(
                    RevisionJobKind::DiffAdjacent {
                        file: GistFileRef::id_name("g1", "a.txt"),
                        child_version: "v2".into(),
                        parent_version: Some("v1".into()),
                    },
                    "Loading diff…",
                ),
            ),
            (
                RevisionRequest::DiffAgainstCurrent {
                    entry: initial_state().defer_entry(),
                    target: target(),
                    version: "v1".into(),
                    old_label: String::new(),
                    new_label: String::new(),
                },
                job_spec(
                    RevisionJobKind::DiffAgainstCurrent {
                        file: GistFileRef::id_name("g1", "a.txt"),
                        version: "v1".into(),
                    },
                    "Loading diff…",
                ),
            ),
            (
                RevisionRequest::PreviewRestore {
                    entry: initial_state().defer_entry(),
                    target: target(),
                    version: "v1".into(),
                    version_label: "v1 (3d ago)".into(),
                },
                job_spec(
                    RevisionJobKind::PreviewRestore {
                        file: GistFileRef::id_name("g1", "a.txt"),
                        version: "v1".into(),
                    },
                    "Loading revision…",
                ),
            ),
        ];

        for (request, expected) in cases {
            let mut state = initial_state();
            let (mut jobs, started) = crate::tui::test_support::recording_jobs();
            dispatch(&mut jobs, &mut state, request);
            assert_eq!(started.take(), vec![expected]);
        }
    }

    /// Restore prepares its payload before staging, so its spec is asserted separately.
    #[test]
    fn execute_restore_stages_its_kind_and_progress_label() {
        let mut state = initial_state();
        let (mut jobs, started) = crate::tui::test_support::recording_jobs();

        dispatch(
            &mut jobs,
            &mut state,
            RevisionRequest::ExecuteRestore {
                target: target(),
                content: "old\n".into(),
            },
        );

        assert_eq!(
            started.take(),
            vec![job_spec(
                RevisionJobKind::ExecuteRestore {
                    file: GistFileRef::id_name("g1", "a.txt"),
                },
                "Restoring revision…",
            )]
        );
    }

    /// The workflow reads nothing mutable off the screen after the request is built: an
    /// intent-time target still drives the fetch when the screen has moved on.
    #[test]
    fn the_workflow_ignores_screen_changes_made_after_the_request(
    ) -> std::result::Result<(), String> {
        let mut state = revisions_screen("g1");
        let e = entry(&state);
        let runner = scripted(vec![
            CommandOutput::ok(revision_json("a.txt", "old\n")),
            CommandOutput::ok("current\n"),
        ]);
        let mut jobs = Jobs::inline(&GistCatalog::default(), runner.clone());
        let request = RevisionRequest::DiffAgainstCurrent {
            entry: e,
            target: target(),
            version: "v1".into(),
            old_label: "revision v1".into(),
            new_label: "current a.txt".into(),
        };
        // Move the selection somewhere else before the work is staged and run.
        state.revision_mut().expect("revisions").target_file = "b.txt".into();
        state.gist_catalog.owned = vec![GistFile::fixture("g1", "b.txt")];

        dispatch(&mut jobs, &mut state, request);
        jobs.absorb(&mut state, &None).expect("absorb");

        assert_eq!(runner.calls()[1], crate::gh::gist_view_plan("g1", "a.txt"));
        Ok(())
    }
}
