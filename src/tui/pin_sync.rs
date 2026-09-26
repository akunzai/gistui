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
}
