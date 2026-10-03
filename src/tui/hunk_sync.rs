//! Save the staged contents of one Sync pair, after showing each side's write diff.
use super::bg::{ActionJobKind, ActionJobSpec, Jobs, LoopFlow};
use super::{AppState, KeyOutcome, PendingAction};
use crate::domain::SyncPair;
use crate::merge::Merge;
use crate::sync_content::SyncPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkExit {
    Back,
    Pins,
    Gists,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRequest {
    pub pair: SyncPair,
    pub baseline_local: Option<String>,
    pub baseline_gist: String,
    pub local: Option<String>,
    pub gist: Option<String>,
    pub exit: Option<HunkExit>,
}

impl SaveRequest {
    fn new(pair: SyncPair, merge: &Merge, policy: SyncPolicy, exit: Option<HunkExit>) -> Self {
        Self {
            pair,
            baseline_local: merge.baseline_local.clone(),
            baseline_gist: merge.baseline_gist.clone(),
            local: merge
                .local_dirty()
                .then(|| policy.to_disk(&merge.local).into_owned()),
            gist: merge
                .gist_dirty()
                .then(|| policy.outbound(&merge.gist).into_owned()),
            exit,
        }
    }

    fn preview(&self, policy: SyncPolicy) -> String {
        let mut result = String::new();
        if let Some(local) = &self.local {
            result += &policy.diff(
                "Local (saved)",
                self.baseline_local.as_deref().unwrap_or_default(),
                "Local (staged)",
                local,
            );
        }
        if let Some(gist) = &self.gist {
            if !result.is_empty() {
                result.push('\n');
            }
            result += &policy.diff("Gist (saved)", &self.baseline_gist, "Gist (staged)", gist);
        }
        result
    }

    pub fn sides(&self) -> &'static str {
        match (self.local.is_some(), self.gist.is_some()) {
            (true, true) => "Local and Gist",
            (true, false) => "Local",
            _ => "Gist",
        }
    }
}

impl AppState {
    pub(crate) fn hunks_dirty(&self) -> bool {
        self.diff()
            .and_then(|d| d.merge.as_ref())
            .is_some_and(Merge::dirty)
    }

    pub(crate) fn confirm_hunk_save(&mut self, exit: Option<HunkExit>) {
        let Some(pair) = self.sync_pair().cloned() else {
            return;
        };
        let Some(merge) = self
            .diff()
            .and_then(|d| d.merge.as_ref())
            .filter(|m| m.dirty())
        else {
            return;
        };
        let policy = self.settings.sync_policy();
        let request = SaveRequest::new(pair, merge, policy, exit);
        let preview = request.preview(policy);
        self.enter_confirm_from_diff(PendingAction::SaveHunks(Box::new(request)));
        if let Some(body) = self.scroll_body_mut() {
            body.text = preview;
            body.scroll = 0;
            body.hscroll = 0;
        }
    }

    pub(crate) fn gate_hunk_exit(&mut self, exit: HunkExit) -> bool {
        if !self.hunks_dirty() {
            return false;
        }
        if self.screen.is_palette() {
            self.close_palette();
        }
        while !self.screen.is_diff() && !self.nav_stack.is_empty() {
            self.leave();
        }
        if self.screen.is_diff() {
            self.enter_confirm_from_diff(PendingAction::LeaveHunks(exit));
        }
        true
    }

    pub(crate) fn request_hunk_back(&mut self) {
        if !self.gate_hunk_exit(HunkExit::Back) {
            self.leave();
        }
    }

    pub(crate) fn request_hunk_quit(&mut self) -> KeyOutcome {
        if self.gate_hunk_exit(HunkExit::Quit) {
            KeyOutcome::None
        } else {
            KeyOutcome::Quit
        }
    }

    pub(crate) fn finish_hunk_exit(&mut self, exit: HunkExit) -> KeyOutcome {
        self.leave();
        match exit {
            HunkExit::Back => {}
            HunkExit::Pins => self.open_pins(),
            HunkExit::Gists => self.open_gist_manager(),
            HunkExit::Quit => return KeyOutcome::Quit,
        }
        KeyOutcome::None
    }
}

#[derive(Debug, Default)]
struct Saved {
    local: Option<String>,
    gist: Option<String>,
    error: Option<String>,
}

/// Preflight both live sides, bypassing content-cache and immutable raw-URL fallback.
/// GitHub's Gist update has no transaction with the filesystem; a failure after the local
/// write reports that partial success and keeps the Gist buffer dirty for a later retry.
fn execute(runner: &dyn crate::actions::CommandRunner, request: &SaveRequest) -> Saved {
    let mut saved = Saved::default();
    let result = (|| -> anyhow::Result<()> {
        let gist = super::bg::fetch_gist_content(
            runner,
            &request.pair.gist.gist_id,
            &request.pair.gist.filename,
            None,
        )
        .map_err(|e| anyhow::anyhow!("cannot recheck Gist: {e}"))?;
        let local = match crate::domain::read_text_file_capped(&request.pair.local) {
            Ok(content) => Some(content),
            Err(_) if !request.pair.local.exists() => None,
            Err(error) => anyhow::bail!("cannot read Local: {error}"),
        };
        if local != request.baseline_local || gist != request.baseline_gist {
            anyhow::bail!("Local or Gist changed since the preview; staged changes kept — reopen the diff before saving");
        }
        if let Some(content) = &request.local {
            // The request is created only after the per-side preview and explicit `y save`.
            let mode = if request.baseline_local.is_some() {
                crate::actions::DownloadMode::overwrite_after_user_confirm()
            } else {
                crate::actions::DownloadMode::CreateNew
            };
            crate::actions::execute_download(&request.pair.local, content, mode)?;
            saved.local = Some(content.clone());
        }
        if let Some(content) = &request.gist {
            let scratch = crate::temp_dir::ScratchDir::create("hunks")?;
            let payload =
                serde_json::json!({"files": {&request.pair.gist.filename: {"content": content}}});
            let path = scratch.create_file("update.json", &serde_json::to_vec(&payload)?)?;
            // Same exact-content PATCH plan as revision restore; no editor or newline rewrite.
            let plan = crate::gh::restore_revision_command(&request.pair.gist.gist_id, &path);
            crate::actions::run_command(runner, &plan)?;
            saved.gist = Some(content.clone());
        }
        Ok(())
    })();
    if let Err(error) = result {
        saved.error = Some(error.to_string());
    }
    saved
}

pub(super) fn dispatch(jobs: &mut Jobs, state: &mut AppState, request: Box<SaveRequest>) {
    let runner = jobs.command_runner();
    let file = request.pair.gist.clone();
    let apply_request = request.clone();
    jobs.spawn_action(
        state,
        ActionJobSpec::new(ActionJobKind::SaveHunks(file), "Saving staged changes…"),
        move || execute(runner.as_ref(), &request),
        move |saved, state| on_saved(state, &apply_request, saved),
    );
}

fn on_saved(state: &mut AppState, request: &SaveRequest, saved: Saved) -> LoopFlow {
    if state.screen.is_confirm() {
        state.cancel_confirm();
    }
    let local_saved = saved.local.is_some();
    let gist_saved = saved.gist.is_some();
    if let Some(gist) = &saved.gist {
        state.gist_content_store.invalidate_file(&request.pair.gist);
        state.gist_list_stale = true;
        let sha =
            crate::sync_baseline::SyncBaseline::after_sync(b"", gist.as_bytes()).remote_blob_sha;
        super::sync::patch_catalog_blob_sha(state, &request.pair.gist, sha.as_deref());
    }
    let policy = state.settings.sync_policy();
    if let Some(diff) = state.diff_mut() {
        if let Some(merge) = &mut diff.merge {
            if local_saved || gist_saved {
                merge.saved(saved.local, saved.gist);
            }
            if let super::DiffKind::Sync { remote, .. } = &mut diff.kind {
                *remote = merge.baseline_gist.clone();
            }
        }
        diff.refresh_merge_preview(policy);
    }
    state.reveal_hunk();
    state.mark_pin_sync_cache_dirty();
    // A partial merge is not a completed Sync: only equal saved buffers can advance a pin.
    let baseline = state
        .diff()
        .and_then(|d| d.merge.as_ref())
        .filter(|m| !m.dirty() && m.baseline_local.is_some())
        .filter(|m| state.settings.sync_policy().identical(&m.local, &m.gist))
        .map(|m| (m.local.clone(), m.gist.clone()));
    let failed = saved.error.is_some();
    let status = if let Some(error) = saved.error {
        let prefix = if local_saved {
            "Saved Local; Gist save failed"
        } else {
            "Save failed"
        };
        format!("{prefix}: {error}")
    } else {
        format!("Saved staged changes to {}", request.sides())
    };
    state.set_status(status);
    if local_saved {
        super::sync::refresh_locals(state, Some(&request.pair.local));
    }
    if let Some((local, gist)) = baseline.filter(|_| !failed) {
        super::pin_sync::confirm_sync_baseline(
            state,
            &request.pair.local,
            &request.pair.gist,
            &local,
            &gist,
        );
    }
    if !failed {
        if let Some(exit) = request.exit {
            if state.finish_hunk_exit(exit) == KeyOutcome::Quit {
                return LoopFlow::Quit;
            }
        }
    }
    LoopFlow::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{test_support::SeqRunner, CommandOutput};
    use crate::domain::{GistFile, GistFileRef};
    use crossterm::event::KeyCode;
    use std::sync::Arc;

    const LOCAL: &str = "local\nsame\nold\nsame\nkeep local\n";
    const GIST: &str = "old\nsame\ngist\nsame\nkeep gist\n";

    fn response(content: &str) -> CommandOutput {
        CommandOutput::ok(
            serde_json::json!({"files":{"a.txt":{"content":content,"truncated":false}}})
                .to_string(),
        )
    }

    fn open(path: &std::path::Path, runner: Arc<SeqRunner>) -> (AppState, Jobs) {
        let mut state = super::super::initial_state();
        state.gist_catalog.owned = vec![GistFile::fixture("g1", "a.txt")];
        let mut jobs = Jobs::inline(&state.gist_catalog, runner);
        let entry = state.defer_entry();
        super::super::sync::dispatch(
            &mut jobs,
            &mut state,
            super::super::sync::SyncRequest {
                entry,
                pair: SyncPair {
                    local: path.into(),
                    gist: GistFileRef::id_name("g1", "a.txt"),
                },
                intent: super::super::sync::SyncIntent::Compare {
                    upload_orientation: false,
                },
            },
        );
        jobs.on_action_outcome(&mut state);
        assert!(state.screen.is_diff());
        (state, jobs)
    }

    fn save(state: &mut AppState, jobs: &mut Jobs) {
        state.handle_key(KeyCode::Char('s'));
        let vm = super::super::screens::confirm::build_confirm_vm(state);
        let super::super::screens::confirm::ConfirmBackgroundVm::Diff(diff) = vm.background else {
            panic!("write preview")
        };
        assert!(diff.body.contains("Local (saved)") || diff.body.contains("Gist (saved)"));
        assert!(
            diff.sides.is_none(),
            "confirm shows original-to-staged write diffs"
        );
        let KeyOutcome::SaveHunks(request) = state.handle_key(KeyCode::Char('y')) else {
            panic!("save intent")
        };
        dispatch(jobs, state, request);
        jobs.on_action_outcome(state);
    }

    #[test]
    fn bidirectional_partial_save_retains_failed_side_and_retry_only_writes_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, LOCAL).unwrap();
        let runner = Arc::new(SeqRunner::new(vec![
            response(GIST),
            response(GIST),
            CommandOutput::err("HTTP 502"),
            response(GIST),
            CommandOutput::ok(""),
        ]));
        let (mut state, mut jobs) = open(&path, runner.clone());
        state.handle_key(KeyCode::Char(']'));
        state.handle_key(KeyCode::Char('['));
        save(&mut state, &mut jobs);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "local\nsame\ngist\nsame\nkeep local\n"
        );
        let merge = state.diff().unwrap().merge.as_ref().unwrap();
        assert!(!merge.local_dirty() && merge.gist_dirty());
        assert!(
            !merge.can_undo(),
            "cannot undo committed Local through a stale snapshot"
        );
        assert!(state
            .status
            .as_deref()
            .unwrap()
            .contains("Saved Local; Gist save failed"));
        assert!(state.status.as_deref().unwrap().contains("HTTP 502"));
        save(&mut state, &mut jobs);
        assert!(!state.hunks_dirty());
        assert_eq!(
            state.diff().unwrap().merge.as_ref().unwrap().hunk_count(),
            1,
            "unselected difference remains"
        );
        assert!(state.gist_list_stale);
        let calls = runner.recorded();
        assert_eq!(calls.len(), 5);
        let payload: serde_json::Value =
            serde_json::from_str(calls[4].input_body.as_deref().unwrap()).unwrap();
        assert_eq!(
            payload["files"]["a.txt"]["content"],
            "local\nsame\ngist\nsame\nkeep gist\n"
        );
    }

    #[test]
    fn changes_to_either_live_side_stop_before_any_write_and_keep_staging() {
        for change_local in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("a.txt");
            std::fs::write(&path, LOCAL).unwrap();
            let runner = Arc::new(SeqRunner::new(vec![
                response(GIST),
                response(if change_local {
                    GIST
                } else {
                    "external edit\n"
                }),
            ]));
            let (mut state, mut jobs) = open(&path, runner.clone());
            state.handle_key(KeyCode::Char(']'));
            state.handle_key(KeyCode::Char('['));
            if change_local {
                std::fs::write(&path, "external edit\n").unwrap();
            }
            save(&mut state, &mut jobs);
            assert_eq!(runner.calls().len(), 2, "no PATCH after stale preview");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                if change_local {
                    "external edit\n"
                } else {
                    LOCAL
                }
            );
            assert!(state.hunks_dirty());
            assert!(state
                .status
                .as_deref()
                .unwrap()
                .contains("changed since the preview"));
        }
    }

    #[test]
    fn save_only_local_rechecks_remote_but_never_patches_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, LOCAL).unwrap();
        let runner = Arc::new(SeqRunner::new(vec![response(GIST), response(GIST)]));
        let (mut state, mut jobs) = open(&path, runner.clone());
        state.handle_key(KeyCode::Char('['));
        save(&mut state, &mut jobs);
        assert_eq!(runner.calls().len(), 2);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "old\nsame\nold\nsame\nkeep local\n"
        );
        assert!(!state.hunks_dirty());
        assert!(!state.gist_list_stale);
    }

    #[test]
    fn a_failed_preflight_keeps_both_buffers_and_creates_no_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        let runner = Arc::new(SeqRunner::new(vec![
            response("a\n"),
            CommandOutput::err("HTTP 403"),
        ]));
        let (mut state, mut jobs) = open(&path, runner.clone());
        state.handle_key(KeyCode::Char('['));
        save(&mut state, &mut jobs);
        assert!(!path.exists());
        assert!(state.hunks_dirty());
        assert!(state
            .status
            .as_deref()
            .unwrap()
            .contains("cannot recheck Gist"));
    }
    #[test]
    fn saving_before_quit_applies_the_write_and_then_quits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, LOCAL).unwrap();
        let runner = Arc::new(SeqRunner::new(vec![response(GIST), response(GIST)]));
        let (mut state, mut jobs) = open(&path, runner);
        state.handle_key(KeyCode::Char('['));
        assert_eq!(state.request_hunk_quit(), KeyOutcome::None);
        state.handle_key(KeyCode::Char('s'));
        let KeyOutcome::SaveHunks(request) = state.handle_key(KeyCode::Char('y')) else {
            panic!("save")
        };
        dispatch(&mut jobs, &mut state, request);
        assert!(matches!(jobs.on_action_outcome(&mut state), LoopFlow::Quit));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "old\nsame\nold\nsame\nkeep local\n"
        );
    }

    #[test]
    fn unselected_differences_do_not_advance_a_pins_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, LOCAL).unwrap();
        let stored = super::super::test_support::state_with_stored_pin(
            dir.path(),
            crate::domain::PinnedMapping::fixture(&path, "g1", "a.txt"),
        );
        let runner = Arc::new(SeqRunner::new(vec![
            response(GIST),
            response(GIST),
            CommandOutput::ok(""),
        ]));
        let (mut state, mut jobs) = open(&path, runner);
        state.pinned = stored.pinned;
        state.config_store = stored.config_store;
        let before = state.pinned.clone();
        state.handle_key(KeyCode::Char(']'));
        state.handle_key(KeyCode::Char('['));
        save(&mut state, &mut jobs);
        assert_eq!(state.pinned, before);
        assert_eq!(
            state.diff().unwrap().merge.as_ref().unwrap().hunk_count(),
            1
        );
    }
}
