//! `KeyOutcome` → IO side effects for the TUI event loop.
//! Extracted from `run_loop` (issue #225). Outcomes carry payloads (issue #244) so this
//! layer does not re-resolve list/detail selection.

use super::bg::*;
use super::pin_sync::{pin_paths, unpin_at_pin_index, unpin_path};
use super::sync::download;
use super::*;
use editor::{edit_local_path, edit_upload_buffer};
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
        KeyOutcome::Download { mode } => download(state, mode),
        KeyOutcome::DownloadRequested { target } => {
            if target.exists() {
                state.enter_confirm_from_diff(PendingAction::Download);
            } else {
                download(state, crate::actions::DownloadMode::CreateNew);
            }
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
            let runner = jobs.command_runner();
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
                    let result = load_initial_comments(runner.as_ref(), &fetch_id);
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
            let runner = jobs.command_runner();
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
                        runner.as_ref(),
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
        } => gist_mutation::analyze_compact(jobs, state, entry, gist_id, label),
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
        KeyOutcome::RefreshLocals => {
            jobs.request_local_scan(state);
        }
        KeyOutcome::UnpinAtPin { index } => {
            if unpin_at_pin_index(state, index) {
                let len = state.visible_pin_indices().len();
                if let Some(pins) = state.pins_mut() {
                    pins.cursor.clamp_len(len);
                }
                // No filesystem rescan: unpin never touches the filesystem, and ranking reads
                // `PinnedMapping` directly — a forced-flat rescan here used to make the local
                // list drift back to cwd-only even while recursive mode was active (issue #409).
            }
        }
        KeyOutcome::Revision(request) => gist_revision::dispatch(jobs, state, request),
        KeyOutcome::Mutation(request) => gist_mutation::dispatch(jobs, state, request),
        KeyOutcome::Sync(request) => crate::tui::sync::dispatch(jobs, state, request),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_support::SeqRunner;
    use crate::actions::CommandOutput;
    use crate::domain::{GistFile, PinnedMapping};
    use crate::tui::gist_mutation::MutationJobKind;
    use crate::tui::gist_mutation::MutationRequest;
    use crate::tui::test_support::{idle_jobs, recording_jobs};
    use crossterm::event::KeyCode;

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
            KeyOutcome::Mutation(MutationRequest::Fork {
                gist_id: "not-owned".into(),
            }),
            &mut state,
            &mut jobs,
        );

        assert_eq!(
            started.take(),
            vec![ActionJobSpec::new(
                ActionJobKind::Mutation(MutationJobKind::Fork {
                    gist_id: "not-owned".into(),
                }),
                "Forking…",
            )]
        );
    }

    // ---- early returns ----------------------------------------------------

    /// The listed-not-wildcarded arm guards new variants; this guards the other
    /// direction, where an arm is dropped from `dispatch_outcome` and would otherwise
    /// become a silent no-op.
    #[test]
    #[should_panic(expected = "must keep its arm")]
    fn a_terminal_bearing_outcome_reaching_route_outcome_trips_the_assert() {
        let mut state = initial_state();
        route(&mut state, KeyOutcome::EditUpload);
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

        fn fetch_raw(&self, url: &str) -> anyhow::Result<Vec<u8>> {
            anyhow::bail!("no raw fetch expected: {url}")
        }
    }

    /// Issue #460: a pin push confirmed from Pins → Confirm must record the pin sync and
    /// return to Pins, even after the upload arm has left Confirm and a setting changes while
    /// the job runs. Issue #465: it sends the normalized bytes, but the pin baseline is the
    /// local file as it sits on disk.
    #[test]
    fn upload_from_pin_push_records_pin_sync_and_returns_to_pins() {
        use crate::domain::SyncDirection;

        let dir = tempfile::tempdir().unwrap();

        let local_path = dir.path().join("a.txt");
        std::fs::write(&local_path, "a\r\nb\r\n").unwrap();
        let mapping = PinnedMapping::fixture(local_path.clone(), "g1", "a.txt");

        let mut state = test_support::state_with_stored_pin(dir.path(), mapping);
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
        let confirmed = state.handle_key(KeyCode::Char('y'));
        route_outcome(confirmed, &mut state, &mut jobs);
        // A setting flipped mid-upload must not change what the pin records.
        state
            .settings
            .adjust(crate::tui::ConfigField::NormalizeLineEndings, true);
        // Drain only the action outcome: `absorb` would start a real gist-list refresh.
        jobs.on_action_outcome(&mut state);

        let sent = runner.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, vec!["a\nb\n".to_string()]);
        assert_eq!(state.status.as_deref(), Some("Uploaded a.txt to gist g1"));
        assert_eq!(state.pinned[0].direction, Some(SyncDirection::Upload));
        assert_eq!(
            state.pinned[0].baseline.local_sha256.as_deref(),
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

            let create = KeyOutcome::Mutation(MutationRequest::Create {
                local_path: local_path.clone(),
                public: false,
                description: String::new(),
            });
            route_outcome(create, &mut state, &mut jobs);

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

    // ---- reads through the injected runner (issue #511) ------------------

    fn scripted(outputs: Vec<crate::actions::CommandOutput>) -> std::sync::Arc<SeqRunner> {
        std::sync::Arc::new(SeqRunner::new(outputs))
    }

    /// A failed preview refresh reports the error and keeps the last-known-good content:
    /// nothing is cached until a fetch succeeds.
    #[test]
    fn a_failed_preview_refresh_keeps_the_cached_content() {
        let mut state = test_support::state_with_gists();
        let file = crate::domain::GistFileRef::id_name("g1", "a.txt");
        state.gist_content_store.insert(&file, "last good".into());
        let runner = scripted(vec![CommandOutput::err("HTTP 502")]);
        let mut jobs = Jobs::inline(&state.gist_catalog.clone(), runner.clone());
        let entry = state.defer_entry();

        route_outcome(
            KeyOutcome::RefreshPreview {
                entry,
                file: file.clone(),
            },
            &mut state,
            &mut jobs,
        );
        jobs.on_action_outcome(&mut state);

        assert_eq!(runner.calls(), vec![crate::gh::gist_get_plan("g1")]);
        assert!(
            state
                .status
                .as_deref()
                .is_some_and(|s| s.contains("HTTP 502")),
            "{:?}",
            state.status
        );
        assert_eq!(
            state.gist_content_store.lookup(&state.gist_catalog, file),
            crate::tui::gist_content::ContentLookup::Hit("last good".into())
        );
    }

    /// The first comments load probes the total, then fetches the newest page.
    #[test]
    fn the_first_comments_load_probes_then_fetches_the_newest_page() {
        let comments = include_str!("../../tests/fixtures/gh/gist-comments.json");
        let mut state = initial_state();
        state.enter(Screen::GistDetail(Box::new(DetailState {
            gist_id: Some("g1".into()),
            ..DetailState::default()
        })));
        let runner = scripted(vec![
            CommandOutput::ok(format!("HTTP/2.0 200 OK\n\n{comments}")),
            CommandOutput::ok(comments),
        ]);
        let mut jobs = Jobs::inline(&state.gist_catalog.clone(), runner.clone());

        route_outcome(
            KeyOutcome::FetchComments {
                gist_id: "g1".into(),
            },
            &mut state,
            &mut jobs,
        );
        jobs.on_action_outcome(&mut state);

        assert_eq!(
            runner.calls(),
            vec![
                crate::gh::gist_comments_probe_plan("g1"),
                crate::gh::gist_comments_page_plan("g1", 1, crate::gh::COMMENTS_PAGE_SIZE),
            ]
        );
        assert_eq!(
            state
                .detail()
                .and_then(|d| d.comments.as_ref())
                .map(Vec::len),
            Some(3)
        );
    }
}
