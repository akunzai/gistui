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

    /// Impure single-pin status. With a full Sync baseline and a catalog blob sha, it compares
    /// both sides against the baseline (issue #466): local SHA-256 of the file on disk, remote
    /// blob sha from the catalog — no timestamps. Otherwise (a pin recorded before #466, or a
    /// file the catalog has no sha for) it falls back to in-memory mtimes plus a local
    /// content-hash check when timestamps disagree (`Push`/`Pull`).
    /// Used by [`Self::refresh_pin_sync_cache`] and by action dispatch (smart-sync); **not**
    /// for paint — presentation reads [`Self::cached_pin_sync_status`] (issue #241).
    pub(crate) fn compute_pin_sync_status(&self, index: usize) -> crate::domain::SyncStatus {
        let Some(m) = self.pinned.get(index) else {
            return crate::domain::SyncStatus::Unknown;
        };
        let local_abs = m.resolve_against(&self.cwd);
        if let (Some(local_base), Some(remote_base), Some(remote_now)) = (
            m.last_seen_hash.as_deref(),
            m.remote_blob_sha.as_deref(),
            self.catalog_blob_sha(&m.gist_id, &m.gist_filename),
        ) {
            return match std::fs::read(&local_abs) {
                Ok(bytes) => crate::domain::baseline_status(
                    crate::domain::sha256_hex(&bytes) != local_base,
                    remote_now != remote_base,
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    crate::domain::SyncStatus::Missing
                }
                Err(_) => crate::domain::SyncStatus::Unknown,
            };
        }

        let (local_ts, remote_ts) = self.pin_mtimes(index);
        let status = crate::domain::sync_status(local_ts, remote_ts);
        if !matches!(
            status,
            crate::domain::SyncStatus::Push | crate::domain::SyncStatus::Pull
        ) {
            return status;
        }
        let Some(baseline) = m.last_seen_hash.as_deref() else {
            return status;
        };
        match std::fs::read(&local_abs) {
            Ok(bytes) if crate::domain::sha256_hex(&bytes) == baseline => {
                crate::domain::SyncStatus::InSync
            }
            _ => status,
        }
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

#[cfg(test)]
mod tests {
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
        state.pinned = vec![crate::domain::PinnedMapping {
            local_path: outside.clone(),
            gist_id: "g1".into(),
            gist_filename: "settings.json".into(),
            direction: None,
            last_seen_hash: None,
            remote_blob_sha: None,
        }];

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
        state.pinned = vec![crate::domain::PinnedMapping {
            local_path: gone,
            gist_id: "g1".into(),
            gist_filename: "settings.json".into(),
            direction: None,
            last_seen_hash: None,
            remote_blob_sha: None,
        }];
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
    fn pin_sync_status_upgrades_to_in_sync_when_content_hash_matches_baseline() {
        // Timestamps disagree (forcing Push), but the content hash still matches what was
        // last recorded as synced — the Pins list should show synced (✓), not a misleading
        // push arrow, since nothing has actually changed content-wise.
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("settings.json");
        let content = b"{\"key\":\"value\"}";
        std::fs::write(&local, content).unwrap();
        let hash = crate::domain::sha256_hex(content);

        let mut state = initial_state();
        state.locals.clear();
        state.pinned = vec![crate::domain::PinnedMapping {
            local_path: local,
            gist_id: "g1".into(),
            gist_filename: "settings.json".into(),
            direction: None,
            last_seen_hash: Some(hash),
            remote_blob_sha: None,
        }];
        state.gist_catalog.owned = vec![GistFile {
            // Far in the past, so the just-written local file (mtime ~ now) reads as newer —
            // sync_status(Some(local_ts), Some(remote_ts)) would normally resolve to Push.
            updated_at: "2020-01-01T00:00:00Z".into(),
            ..GistFile::fixture("g1", "settings.json")
        }];

        assert_eq!(
            {
                state.refresh_pin_sync_cache();
                state.cached_pin_sync_status(0)
            },
            crate::domain::SyncStatus::InSync,
            "a matching content hash must override a stale-timestamp Push into InSync"
        );
    }

    #[test]
    fn pin_sync_status_keeps_push_when_content_hash_does_not_match_baseline() {
        // Same timestamp setup as above, but the recorded baseline hash doesn't match the
        // file's actual current content — a real, unrecorded local change. Must stay Push.
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("settings.json");
        std::fs::write(&local, b"{\"key\":\"value\"}").unwrap();

        let mut state = initial_state();
        state.locals.clear();
        state.pinned = vec![crate::domain::PinnedMapping {
            local_path: local,
            gist_id: "g1".into(),
            gist_filename: "settings.json".into(),
            direction: None,
            last_seen_hash: Some("does-not-match-anything".into()),
            remote_blob_sha: None,
        }];
        state.gist_catalog.owned = vec![GistFile {
            updated_at: "2020-01-01T00:00:00Z".into(),
            ..GistFile::fixture("g1", "settings.json")
        }];

        assert_eq!(
            {
                state.refresh_pin_sync_cache();
                state.cached_pin_sync_status(0)
            },
            crate::domain::SyncStatus::Push,
            "a non-matching baseline hash must not mask a real content change"
        );
    }

    #[test]
    fn pin_sync_status_keeps_push_when_no_baseline_hash_recorded() {
        // Regression guard: a pin that was never synced (no baseline hash at all) must fall
        // back to the plain timestamp-based status, not attempt a hash comparison.
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("settings.json");
        std::fs::write(&local, b"{\"key\":\"value\"}").unwrap();

        let mut state = initial_state();
        state.locals.clear();
        state.pinned = vec![crate::domain::PinnedMapping {
            local_path: local,
            gist_id: "g1".into(),
            gist_filename: "settings.json".into(),
            direction: None,
            last_seen_hash: None,
            remote_blob_sha: None,
        }];
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
    /// carries `catalog_sha`. `updated_at` is far in the future, so the timestamp path would
    /// always say Pull — the baseline path must not look at it (issue #466).
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
            local_path: local,
            gist_id: "g1".into(),
            gist_filename: "a.txt".into(),
            direction: None,
            last_seen_hash: Some(crate::domain::sha256_hex(b"synced")),
            remote_blob_sha: Some(OLD_SHA.into()),
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
    fn pin_sync_status_from_the_sync_baseline() {
        use crate::domain::SyncStatus::*;
        for (local, catalog_sha, expected) in [
            (Some("synced"), OLD_SHA, InSync),
            (Some("edited"), OLD_SHA, Push),
            (Some("synced"), NEW_SHA, Pull),
            (Some("edited"), NEW_SHA, Conflict),
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

    #[test]
    fn pin_sync_status_without_a_remote_baseline_falls_back_to_timestamps() {
        let (_dir, mut state) = baseline_state(Some("edited"), OLD_SHA);
        state.pinned[0].remote_blob_sha = None;
        // Timestamps: the gist's updated_at is newer, and the local hash moved → Pull.
        assert_eq!(
            state.compute_pin_sync_status(0),
            crate::domain::SyncStatus::Pull
        );
    }
}
