//! Pin-sync presentation cache (issue #313). Cross-cutting — called from `dispatch.rs`,
//! `bg.rs`, and `run_loop.rs` (pre-draw refresh); only [`AppState::cached_pin_sync_entry`]
//! is called from `screens/pins.rs`. See `docs/agents/architecture.md`'s "Pin-sync
//! presentation" section for the refresh-timing invariant.

use crate::tui::AppState;

/// One pin's presentation-derived sync facts, computed off the draw path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinSyncCacheEntry {
    pub status: crate::domain::SyncStatus,
    pub local_ts: Option<u64>,
    pub remote_ts: Option<u64>,
}

impl Default for PinSyncCacheEntry {
    fn default() -> Self {
        Self {
            status: crate::domain::SyncStatus::Unknown,
            local_ts: None,
            remote_ts: None,
        }
    }
}

impl AppState {
    /// `(local_ts, remote_ts)` Unix-seconds for `pinned[index]`. The remote side comes
    /// from the matching gist's in-memory `updated_at`; the local side prefers the
    /// discovered candidate's mtime and falls back to stat-ing the path on disk.
    pub fn pin_mtimes(&self, index: usize) -> (Option<u64>, Option<u64>) {
        let Some(m) = self.pinned.get(index) else {
            return (None, None);
        };
        let local_abs = m.resolve_against(&self.cwd);
        let local_ts = self
            .locals
            .iter()
            .find_map(|c| {
                // A `LocalCandidate` is not a pin, so this deliberately does not borrow
                // `crate::pins`' resolution rule.
                let cabs = if c.path.is_absolute() {
                    c.path.clone()
                } else {
                    self.cwd.join(&c.path)
                };
                (cabs == local_abs).then_some(c.modified).flatten()
            })
            // Pins can point outside cwd (or into skipped/too-deep dirs), so they
            // never appear in `self.locals`. Fall back to stat-ing the path so the
            // Pins list and sync status still reflect the real mtime.
            .or_else(|| crate::local::file_mtime_secs(&local_abs));
        let remote_ts = self.gist_catalog.owned.iter().find_map(|g| {
            (g.gist_id == m.gist_id && g.filename == m.gist_filename)
                .then(|| crate::domain::parse_rfc3339_to_unix(&g.updated_at))
                .flatten()
        });
        (local_ts, remote_ts)
    }

    /// The blob sha the in-memory catalog shows for an owned gist file (from its `raw_url`).
    pub(crate) fn catalog_blob_sha(&self, gist_id: &str, filename: &str) -> Option<&str> {
        self.gist_catalog
            .owned
            .iter()
            .find(|g| g.gist_id == gist_id && g.filename == filename)
            .and_then(|g| g.raw_url.as_deref())
            .and_then(crate::domain::raw_url_blob_sha)
    }

    /// Impure single-pin status: read the local file, look up the gist file's blob sha and
    /// both timestamps, and let the pin's Sync baseline decide
    /// ([`SyncBaseline::status`](crate::sync_baseline::SyncBaseline::status)).
    /// Used by [`Self::refresh_pin_sync_cache`] and by action dispatch (smart-sync); **not**
    /// for paint — presentation reads [`Self::cached_pin_sync_status`] (issue #241).
    pub(crate) fn compute_pin_sync_status(&self, index: usize) -> crate::domain::SyncStatus {
        use crate::sync_baseline::{LocalFile, Observed};

        let Some(m) = self.pinned.get(index) else {
            return crate::domain::SyncStatus::Unknown;
        };
        let read = std::fs::read(m.resolve_against(&self.cwd));
        let local = match &read {
            Ok(bytes) => LocalFile::Bytes(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => LocalFile::Missing,
            Err(_) => LocalFile::Unreadable,
        };
        let (local_ts, remote_ts) = self.pin_mtimes(index);
        m.baseline.status(Observed {
            local,
            remote_blob_sha: self.catalog_blob_sha(&m.gist_id, &m.gist_filename),
            local_ts,
            remote_ts,
        })
    }

    /// Rebuild [`Self::pin_sync_cache`] for every pin (may stat / read local files). Clears
    /// the dirty flag. Call from run_loop before drawing Pins, after pin-list changes, and
    /// after successful pin sync absorb — not from pure `handle_key` or the view-model builder.
    pub fn refresh_pin_sync_cache(&mut self) {
        self.pin_sync_cache = (0..self.pinned.len())
            .map(|i| {
                let (local_ts, remote_ts) = self.pin_mtimes(i);
                PinSyncCacheEntry {
                    status: self.compute_pin_sync_status(i),
                    local_ts,
                    remote_ts,
                }
            })
            .collect();
        self.pin_sync_cache_dirty = false;
    }

    /// Mark the pin presentation cache dirty so the next Pins draw refreshes it.
    pub fn mark_pin_sync_cache_dirty(&mut self) {
        self.pin_sync_cache_dirty = true;
    }

    /// Pure read of cached pin sync status. Missing / short cache → [`SyncStatus::Unknown`]
    /// (refresh invariant should have filled the cache before Pins paint).
    pub fn cached_pin_sync_status(&self, index: usize) -> crate::domain::SyncStatus {
        self.pin_sync_cache
            .get(index)
            .map(|e| e.status)
            .unwrap_or(crate::domain::SyncStatus::Unknown)
    }

    /// Pure read of a full cache entry; default [`PinSyncCacheEntry`] when missing.
    pub fn cached_pin_sync_entry(&self, index: usize) -> PinSyncCacheEntry {
        self.pin_sync_cache.get(index).copied().unwrap_or_default()
    }
}

// ---- pin write operations (moved from bg.rs and sync.rs) ----------------------------
//
// Every change to what a pin *is* or *believes* lands here, beside the cache it dirties:
// pin / unpin replace the status line; a recorded sync only appends to it (issue #432).

/// One rendering of a pinned pair for the status line, so pin and unpin cannot drift apart
/// in how they name the same thing (issue #424). `display_path` abbreviates `$HOME`; the raw
/// `Display` would make the pin message disagree with the unpin message about one path.
fn pin_pair_label(local_path: &std::path::Path, filename: &str) -> String {
    format!(
        "{} <-> {}",
        crate::config::display_path(local_path),
        filename
    )
}

pub(super) fn pin_paths(
    state: &mut AppState,
    local_path: &std::path::Path,
    gist_id: &str,
    filename: &str,
) {
    let result = state.config_store.pin(
        &state.cwd,
        crate::pins::PinKey::new(local_path, gist_id, filename),
    );
    match result {
        Ok(change) => {
            apply_pin_change(state, change);
            state.set_status(format!("Pinned {}", pin_pair_label(local_path, filename)));
        }
        Err(error) => state.set_status(format!("pin failed: {error}")),
    }
}

pub(super) fn unpin_path(
    state: &mut AppState,
    local_path: &std::path::Path,
    gist_id: &str,
    filename: &str,
) {
    let result = state.config_store.unpin(
        &state.cwd,
        crate::pins::PinKey::new(local_path, gist_id, filename),
    );
    apply_unpin(state, result, pin_pair_label(local_path, filename));
}

/// Absorb an unpin result. [`Unpinned::NotFound`] means the stored config no longer held
/// that pair, so the status must not claim one was removed (issue #424).
fn apply_unpin(
    state: &mut AppState,
    result: anyhow::Result<(
        crate::config_store::PinChange,
        crate::config_store::Unpinned,
    )>,
    label: String,
) {
    match result {
        Ok((change, outcome)) => {
            apply_pin_change(state, change);
            state.set_status(match outcome {
                crate::config_store::Unpinned::Removed => format!("Unpinned {label}"),
                crate::config_store::Unpinned::NotFound => format!("{label} is not pinned"),
            });
        }
        Err(error) => state.set_status(format!("unpin failed: {error}")),
    }
}

/// Returns whether the pin was removed, so the caller can clamp whatever cursor pointed at it.
pub(super) fn unpin_at_pin_index(state: &mut AppState, idx: usize) -> bool {
    if idx >= state.pinned.len() {
        return false;
    }
    // A row index is a filtered-view concept: resolve it into a `PinKey` here, so the
    // persistence interface never sees one (issue #432).
    let mapping = state.pinned[idx].clone();
    let label = pin_pair_label(&mapping.local_path, &mapping.gist_filename);
    let result = state.config_store.unpin(&state.cwd, mapping.key());
    let ok = result.is_ok();
    apply_unpin(state, result, label);
    ok
}

/// A local file and a gist file found identical under the Sync policy are in sync: if they
/// are a pinned pair, confirm its Sync baseline from the content already in hand, so the Pins
/// list stays correct even if either side changed since the last real sync (issues #466,
/// #492, #493). `local` is the file's raw bytes on disk (not the normalized comparison);
/// `remote` is the gist content as fetched. A passive confirmation: the pin's recorded
/// direction is left alone. The one home of this rule — every flow that finds a pair
/// identical calls it.
pub(super) fn confirm_sync_baseline(
    state: &mut AppState,
    local_abs: &std::path::Path,
    file: &crate::domain::GistFileRef,
    local: &str,
    remote: &str,
) {
    record_pin_sync(
        state,
        local_abs,
        &file.gist_id,
        &file.filename,
        &crate::sync_baseline::SyncBaseline::after_sync(local.as_bytes(), remote.as_bytes()),
        None,
    );
}

/// If `pair` is a pinned pair, record `baseline` for it and project the result onto `AppState`.
///
/// The in-memory check comes **first and gates the file access entirely**: a download of a
/// file nobody pinned must not read the config, and so cannot report a config problem the
/// user did not provoke. Only a pair this session believes is pinned is worth the IO.
pub(super) fn record_pin_sync(
    state: &mut AppState,
    local_abs: &std::path::Path,
    gist_id: &str,
    filename: &str,
    baseline: &crate::sync_baseline::SyncBaseline,
    direction: Option<crate::domain::SyncDirection>,
) {
    let pair = crate::pins::PinKey::new(local_abs, gist_id, filename);
    if crate::pins::position(&state.pinned, &state.cwd, pair).is_none() {
        return;
    }
    let result = state
        .config_store
        .record_sync(&state.cwd, pair, baseline, direction);
    apply_pin_sync(state, result);
}

/// Absorb a `record_sync` result.
///
/// A failure is **appended** to whatever status the surrounding action already set — the
/// caller has usually just reported "Downloaded a.txt", and that matters more than this
/// does (issue #432; same rule as `refresh_locals`). Before #432 all three failure modes
/// were discarded and the user kept a silently stale sync badge.
///
/// `NotPinned` here means the stored config disagrees with what this session believes
/// (a hand edit between the two). Nothing was persisted, so nothing is projected and
/// nothing is said.
fn apply_pin_sync(
    state: &mut AppState,
    result: anyhow::Result<(
        crate::config_store::PinChange,
        crate::config_store::SyncRecord,
    )>,
) {
    match result {
        Ok((change, crate::config_store::SyncRecord::Recorded)) => apply_pin_change(state, change),
        Ok((_, crate::config_store::SyncRecord::NotPinned)) => {}
        Err(error) => state.append_status(format!("pin sync not recorded: {error}")),
    }
}

/// Project a completed persistence operation onto `AppState`. Both fields travel together
/// because "what was just read" is the correct value for both, even after a hand edit.
/// [`pin_paths`] and [`apply_unpin`] share this same projection for pin / unpin.
pub(super) fn apply_pin_change(state: &mut AppState, change: crate::config_store::PinChange) {
    state.pinned = change.pinned;
    state.skip_dirs = change.skip_dirs;
    state.mark_pin_sync_cache_dirty();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::*;

    #[test]
    fn pin_mtimes_local_falls_back_to_disk_when_not_discovered() {
        // A pin pointing outside cwd is absent from state.locals, but the Pins list
        // and sync status should still reflect the file's real mtime by stat-ing it.
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("settings.json");
        std::fs::write(&outside, "{}").unwrap();

        let mut state = initial_state();
        state.locals.clear();
        state.pinned = vec![crate::domain::PinnedMapping::fixture(
            outside.clone(),
            "g1",
            "settings.json",
        )];

        let (local_ts, _remote_ts) = state.pin_mtimes(0);
        assert!(
            local_ts.is_some(),
            "local mtime should fall back to disk for pins outside cwd"
        );
    }

    #[test]
    fn pin_sync_status_is_missing_when_local_file_absent() {
        // A pinned local path that doesn't exist on disk should report Missing,
        // not the generic Unknown ambiguity used when a timestamp is merely
        // unavailable for other reasons.
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("settings.json");
        // Deliberately never created — this path must not exist.

        let mut state = initial_state();
        state.locals.clear();
        state.pinned = vec![crate::domain::PinnedMapping::fixture(
            gone,
            "g1",
            "settings.json",
        )];
        state.gist_catalog.owned = vec![GistFile {
            updated_at: "2026-01-01T00:00:00Z".into(),
            ..GistFile::fixture("g1", "settings.json")
        }];

        assert_eq!(
            {
                state.refresh_pin_sync_cache();
                state.cached_pin_sync_status(0)
            },
            crate::domain::SyncStatus::Missing,
            "a pin whose local file doesn't exist must report Missing even though \
             the gist side has a known mtime"
        );
    }

    #[test]
    fn pin_sync_status_reads_the_timestamps_without_a_baseline() {
        // A pin that was never synced is classified by timestamps: the local file's mtime
        // (just written) against the catalog's `updated_at` (far in the past).
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("settings.json");
        std::fs::write(&local, b"{\"key\":\"value\"}").unwrap();

        let mut state = initial_state();
        state.locals.clear();
        state.pinned = vec![crate::domain::PinnedMapping::fixture(
            local,
            "g1",
            "settings.json",
        )];
        state.gist_catalog.owned = vec![GistFile {
            updated_at: "2020-01-01T00:00:00Z".into(),
            ..GistFile::fixture("g1", "settings.json")
        }];

        state.refresh_pin_sync_cache();
        assert_eq!(
            state.cached_pin_sync_status(0),
            crate::domain::SyncStatus::Push
        );
    }

    const OLD_SHA: &str = "1111111111111111111111111111111111111111";
    const NEW_SHA: &str = "2222222222222222222222222222222222222222";

    /// A pin with a full Sync baseline over `a.txt` = "synced", and a catalog whose `raw_url`
    /// carries `catalog_sha`. `updated_at` is far in the future, so a status other than Pull
    /// shows the file and the catalog sha were read. The rules are `sync_baseline`'s tests.
    fn baseline_state(
        local_content: Option<&str>,
        catalog_sha: &str,
    ) -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("a.txt");
        if let Some(content) = local_content {
            std::fs::write(&local, content).unwrap();
        }
        let mut state = initial_state();
        state.locals.clear();
        state.pinned = vec![crate::domain::PinnedMapping {
            baseline: crate::sync_baseline::SyncBaseline {
                local_sha256: Some(crate::domain::sha256_hex(b"synced")),
                remote_blob_sha: Some(OLD_SHA.into()),
            },
            ..crate::domain::PinnedMapping::fixture(local, "g1", "a.txt")
        }];
        state.gist_catalog.owned = vec![GistFile {
            updated_at: "2999-01-01T00:00:00Z".into(),
            raw_url: Some(format!(
                "https://gist.githubusercontent.com/u/g1/raw/{catalog_sha}/a.txt"
            )),
            ..GistFile::fixture("g1", "a.txt")
        }];
        (dir, state)
    }

    #[test]
    fn pin_sync_status_reads_the_file_and_the_catalog_sha() {
        use crate::domain::SyncStatus::*;
        for (local, catalog_sha, expected) in [
            (Some("synced"), OLD_SHA, InSync),
            (Some("edited"), OLD_SHA, Push),
            (Some("synced"), NEW_SHA, Pull),
            (None, OLD_SHA, Missing),
        ] {
            let (_dir, state) = baseline_state(local, catalog_sha);
            assert_eq!(
                state.compute_pin_sync_status(0),
                expected,
                "local={local:?} catalog_sha={catalog_sha}"
            );
        }
    }

    use crate::domain::PinnedMapping;
    use crate::sync_baseline::SyncBaseline;
    use std::path::PathBuf;

    fn change(pinned: Vec<PinnedMapping>) -> crate::config_store::PinChange {
        crate::config_store::PinChange {
            pinned,
            skip_dirs: vec!["node_modules".into()],
        }
    }

    #[test]
    fn apply_unpin_reports_the_pair_it_removed() {
        let mut state = crate::tui::initial_state();

        apply_unpin(
            &mut state,
            Ok((change(Vec::new()), crate::config_store::Unpinned::Removed)),
            "~/a.txt <-> a.txt".into(),
        );

        assert_eq!(state.status.as_deref(), Some("Unpinned ~/a.txt <-> a.txt"));
    }

    /// The key is exact now, so a stored config that no longer holds the pair is reachable.
    /// Saying "Unpinned" there would be a lie (issue #424).
    #[test]
    fn apply_unpin_does_not_claim_a_removal_that_did_not_happen() {
        let mut state = crate::tui::initial_state();

        apply_unpin(
            &mut state,
            Ok((change(Vec::new()), crate::config_store::Unpinned::NotFound)),
            "~/a.txt <-> a.txt".into(),
        );

        assert_eq!(
            state.status.as_deref(),
            Some("~/a.txt <-> a.txt is not pinned")
        );
    }

    #[test]
    fn apply_unpin_surfaces_a_failure_without_touching_the_pins() {
        let mut state = crate::tui::initial_state();
        state.pinned = vec![crate::domain::PinnedMapping::fixture(
            "/a.txt", "g1", "a.txt",
        )];

        apply_unpin(
            &mut state,
            Err(anyhow::anyhow!("boom")),
            "~/a.txt <-> a.txt".into(),
        );

        assert_eq!(state.status.as_deref(), Some("unpin failed: boom"));
        assert_eq!(state.pinned.len(), 1);
    }

    /// A persistence failure must not erase the feedback the surrounding action already
    /// set. Before #432 all three failure modes were discarded entirely.
    #[test]
    fn apply_pin_sync_appends_a_failure_to_the_existing_status() {
        let mut state = initial_state();
        state.set_status("Downloaded a.txt");

        apply_pin_sync(&mut state, Err(anyhow::anyhow!("permission denied")));

        assert_eq!(
            state.status.as_deref(),
            Some("Downloaded a.txt; pin sync not recorded: permission denied")
        );
    }

    #[test]
    fn apply_pin_sync_reports_a_failure_on_its_own_when_nothing_was_said() {
        let mut state = initial_state();

        apply_pin_sync(&mut state, Err(anyhow::anyhow!("boom")));

        assert_eq!(state.status.as_deref(), Some("pin sync not recorded: boom"));
    }

    /// `NotPinned` persisted nothing, so it must project nothing and say nothing.
    #[test]
    fn apply_pin_sync_applies_nothing_when_the_pair_was_not_pinned() {
        let mut state = initial_state();
        state.set_status("Downloaded a.txt");
        let before = state.skip_dirs.clone();

        apply_pin_sync(
            &mut state,
            Ok((
                change(vec![PinnedMapping::fixture(
                    "/ignored.txt",
                    "g9",
                    "ignored.txt",
                )]),
                crate::config_store::SyncRecord::NotPinned,
            )),
        );

        assert_eq!(state.status.as_deref(), Some("Downloaded a.txt"));
        assert!(state.pinned.is_empty(), "nothing was persisted to project");
        assert_eq!(state.skip_dirs, before);
    }

    /// A pair this session does not believe is pinned must not reach the filesystem at
    /// all — otherwise a routine download of an unpinned file could report a config
    /// problem the user never provoked.
    #[test]
    fn record_pin_sync_on_an_unpinned_pair_touches_nothing() {
        let mut state = initial_state();
        state.cwd = PathBuf::from("/cwd");
        state.set_status("Downloaded a.txt");

        record_pin_sync(
            &mut state,
            std::path::Path::new("/cwd/a.txt"),
            "g1",
            "a.txt",
            &SyncBaseline::after_sync(b"body", b"body"),
            Some(crate::domain::SyncDirection::Download),
        );

        assert_eq!(state.status.as_deref(), Some("Downloaded a.txt"));
        assert!(state.pinned.is_empty());
    }

    /// Pin, unpin and a recorded sync all land through `apply_pin_change`: both fields
    /// project, so a hand edit picked up by the load is not dropped (issue #432), and the
    /// Pins cache is rebuilt.
    #[test]
    fn apply_pin_change_projects_both_fields_and_dirties_the_cache() {
        let mut state = initial_state();
        state.pin_sync_cache_dirty = false;
        let mapping = PinnedMapping::fixture("/b.txt", "g2", "b.txt");

        apply_pin_change(&mut state, change(vec![mapping.clone()]));

        assert_eq!(state.pinned, vec![mapping]);
        assert_eq!(state.skip_dirs, vec!["node_modules".to_string()]);
        assert!(state.pin_sync_cache_dirty);
    }
}
