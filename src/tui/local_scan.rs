//! Pure TUI orchestration for local-candidate discovery (issue #409): a shared request
//! snapshot, one candidate-application operation, and the scan generation/in-flight
//! lifecycle. Filesystem walking stays in `crate::local`; thread/channel plumbing (spawning
//! the background scan, polling its channel) stays in `bg::Jobs`.

use crate::domain::{LocalCandidate, PinnedMapping};
use crate::tui::AppState;
use std::path::{Path, PathBuf};

/// Flat (cwd-only) vs. recursive discovery, snapshotted from `AppState::local_recursive` at
/// request time — not a live read, so a scan already in flight keeps the mode it started
/// with even if the user flips `local_recursive` again before it completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScanMode {
    Flat,
    Recursive,
}

impl ScanMode {
    pub(super) fn from_active(recursive: bool) -> Self {
        if recursive {
            ScanMode::Recursive
        } else {
            ScanMode::Flat
        }
    }

    fn is_recursive(self) -> bool {
        matches!(self, ScanMode::Recursive)
    }
}

/// Everything `crate::local::discover_local_candidates` needs, snapshotted once so startup,
/// background scans, and synchronous refreshes build equivalent requests and share one
/// discovery contract.
#[derive(Debug, Clone)]
pub(super) struct ScanRequest {
    cwd: PathBuf,
    pinned: Vec<PinnedMapping>,
    mode: ScanMode,
    skip_dirs: Vec<String>,
    max_depth: u32,
}

impl ScanRequest {
    pub(super) fn run(&self) -> anyhow::Result<Vec<LocalCandidate>> {
        crate::local::discover_local_candidates(
            &self.cwd,
            &self.pinned,
            self.mode.is_recursive(),
            &self.skip_dirs,
            self.max_depth,
        )
    }
}

/// Generation + in-flight lifecycle for local-candidate scans. Every mutation goes through
/// `begin`/`end_if_current`, so a stale generation can never end in-flight state or have its
/// result applied — "new requests supersede the previous generation."
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct LocalScan {
    generation: u64,
    in_flight: bool,
}

impl LocalScan {
    /// Start a new scan, superseding any in-flight one. Returns its generation id.
    fn begin(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.in_flight = true;
        self.generation
    }

    fn is_current(&self, generation: u64) -> bool {
        generation == self.generation
    }

    /// End in-flight state for a current-generation success/failure/disconnect. Returns
    /// `false` (leaving state untouched) for a stale generation.
    fn end_if_current(&mut self, generation: u64) -> bool {
        if !self.is_current(generation) {
            return false;
        }
        self.in_flight = false;
        true
    }
}

/// Status text set while a scan is in flight — completion clears only an *unchanged* status
/// still equal to this, so it never erases feedback a newer action wrote in the meantime.
pub(super) const SCANNING_STATUS: &str = "Scanning files…";

impl AppState {
    /// Snapshot a [`ScanRequest`] from current state. Startup passes `ScanMode::Flat`
    /// explicitly; interactive scans and synchronous post-mutation refreshes pass the active
    /// `ScanMode::from_active(self.local_recursive)`.
    pub(super) fn local_scan_request(&self, mode: ScanMode) -> ScanRequest {
        ScanRequest {
            cwd: self.cwd.clone(),
            pinned: self.pinned.clone(),
            mode,
            skip_dirs: self.skip_dirs.clone(),
            max_depth: self.settings.scan_depth(),
        }
    }

    /// Start a new local scan, marking it in-flight and superseding any previous one.
    /// Returns its generation id.
    pub(super) fn begin_local_scan(&mut self) -> u64 {
        self.local_scan.begin()
    }

    /// Whether a local scan is currently in flight (background or synchronous).
    pub(super) fn local_scanning(&self) -> bool {
        self.local_scan.in_flight
    }

    /// Clear the scan's own [`SCANNING_STATUS`] placeholder, but leave any different status a
    /// newer action set in the meantime untouched.
    pub(super) fn clear_scan_status(&mut self) {
        if self.status.as_deref() == Some(SCANNING_STATUS) {
            self.status = None;
        }
    }

    /// Apply a current-generation scan success: preserve the selection (an explicit `target`
    /// — e.g. a just-downloaded file — takes priority over whatever is selected at apply
    /// time), reset local hscroll unless that exact path survives, and re-clamp the gist
    /// cursor (index + hscroll together) if reranking invalidated it. Returns `false` (no
    /// mutation at all) for a stale generation.
    pub(super) fn apply_local_scan(
        &mut self,
        generation: u64,
        candidates: Vec<LocalCandidate>,
        target: Option<&Path>,
    ) -> bool {
        if !self.local_scan.end_if_current(generation) {
            return false;
        }
        let previous = self.selected_local().map(|c| c.path.clone());
        let want = target.map(Path::to_path_buf).or_else(|| previous.clone());
        self.locals = candidates;
        let resolved = want
            .as_deref()
            .and_then(|path| self.locals.iter().position(|c| c.path == path));
        self.local_index = resolved
            .unwrap_or(0)
            .min(self.locals.len().saturating_sub(1));
        if resolved.is_none() || want != previous {
            self.local_hscroll = 0;
        }
        if self.gist_index >= self.ranked_gists().len() {
            self.gist_index = 0;
            self.gist_hscroll = 0;
        }
        true
    }

    /// End a current-generation scan failure/disconnect without touching candidates,
    /// selection, or hscroll — last-known-good stays exactly as it was. Returns `false` for a
    /// stale generation.
    pub(super) fn end_local_scan(&mut self, generation: u64) -> bool {
        self.local_scan.end_if_current(generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::initial_state;

    fn candidate(path: &str) -> LocalCandidate {
        LocalCandidate {
            path: PathBuf::from(path),
            modified: None,
        }
    }

    #[test]
    fn apply_local_scan_ignores_a_stale_generation() {
        let mut state = initial_state();
        state.locals = vec![candidate("/tmp/old.txt")];
        let gen1 = state.begin_local_scan();
        let gen2 = state.begin_local_scan();
        assert_ne!(gen1, gen2);

        assert!(!state.apply_local_scan(gen1, vec![candidate("/tmp/stale.txt")], None));
        assert_eq!(state.locals[0].path, PathBuf::from("/tmp/old.txt"));
        assert!(state.local_scanning(), "gen2 is still in flight");

        assert!(state.apply_local_scan(gen2, vec![candidate("/tmp/fresh.txt")], None));
        assert_eq!(state.locals[0].path, PathBuf::from("/tmp/fresh.txt"));
        assert!(!state.local_scanning());
    }

    #[test]
    fn apply_local_scan_preserves_selection_and_hscroll_when_the_path_survives() {
        let mut state = initial_state();
        state.locals = vec![candidate("a.txt"), candidate("b.txt")];
        state.local_index = 1;
        state.local_hscroll = 5;
        let generation = state.begin_local_scan();

        assert!(state.apply_local_scan(
            generation,
            vec![candidate("a.txt"), candidate("b.txt"), candidate("c.txt")],
            None,
        ));

        assert_eq!(state.locals[state.local_index].path, PathBuf::from("b.txt"));
        assert_eq!(state.local_hscroll, 5, "same path survives, hscroll kept");
    }

    #[test]
    fn apply_local_scan_clears_hscroll_when_selection_falls_back() {
        let mut state = initial_state();
        state.locals = vec![candidate("gone.txt")];
        state.local_index = 0;
        state.local_hscroll = 5;
        let generation = state.begin_local_scan();

        assert!(state.apply_local_scan(generation, vec![candidate("other.txt")], None));

        assert_eq!(state.local_index, 0);
        assert_eq!(
            state.local_hscroll, 0,
            "selection fell back, hscroll cleared"
        );
    }

    #[test]
    fn apply_local_scan_clears_hscroll_when_an_explicit_target_changes_selection() {
        let mut state = initial_state();
        state.locals = vec![candidate("a.txt"), candidate("b.txt")];
        state.local_index = 0;
        state.local_hscroll = 5;
        let generation = state.begin_local_scan();

        assert!(state.apply_local_scan(
            generation,
            vec![candidate("a.txt"), candidate("b.txt")],
            Some(Path::new("b.txt")),
        ));

        assert_eq!(state.locals[state.local_index].path, PathBuf::from("b.txt"));
        assert_eq!(
            state.local_hscroll, 0,
            "target moved selection, hscroll cleared"
        );
    }

    #[test]
    fn end_local_scan_ends_in_flight_only_for_the_current_generation() {
        let mut state = initial_state();
        let stale = state.begin_local_scan();
        let current = state.begin_local_scan();

        assert!(!state.end_local_scan(stale));
        assert!(state.local_scanning());

        assert!(state.end_local_scan(current));
        assert!(!state.local_scanning());
    }

    #[test]
    fn clear_scan_status_only_clears_its_own_placeholder() {
        let mut state = initial_state();
        state.status = Some(SCANNING_STATUS.into());
        state.clear_scan_status();
        assert!(state.status.is_none());

        state.status = Some("a newer status".into());
        state.clear_scan_status();
        assert_eq!(state.status.as_deref(), Some("a newer status"));
    }
}
