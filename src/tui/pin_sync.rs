//! Pin-sync presentation cache (issue #313). Cross-cutting — called from `dispatch.rs`,
//! `bg.rs`, and `run_loop.rs` (pre-draw refresh); only [`AppState::cached_pin_sync_entry`]
//! is called from `screens/pins.rs`. See `docs/agents/architecture.md`'s "Pin-sync
//! presentation" section for the refresh-timing invariant.

use crate::tui::AppState;
use std::path::PathBuf;

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
    /// Resolve a pin's absolute local path against `cwd`.
    fn pin_local_abs(&self, m: &crate::domain::PinnedMapping) -> PathBuf {
        if m.local_path.is_absolute() {
            m.local_path.clone()
        } else {
            self.cwd.join(&m.local_path)
        }
    }

    /// `(local_ts, remote_ts)` Unix-seconds for `pinned[index]`. The remote side comes
    /// from the matching gist's in-memory `updated_at`; the local side prefers the
    /// discovered candidate's mtime and falls back to stat-ing the path on disk.
    pub fn pin_mtimes(&self, index: usize) -> (Option<u64>, Option<u64>) {
        let Some(m) = self.pinned.get(index) else {
            return (None, None);
        };
        let local_abs = self.pin_local_abs(m);
        let local_ts = self
            .locals
            .iter()
            .find_map(|c| {
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
        let remote_ts = self.gists.iter().find_map(|g| {
            (g.gist_id == m.gist_id && g.filename == m.gist_filename)
                .then(|| crate::domain::parse_rfc3339_to_unix(&g.updated_at))
                .flatten()
        });
        (local_ts, remote_ts)
    }

    /// Impure single-pin status: in-memory mtimes plus a content-hash fallback when
    /// timestamps disagree (`Push`/`Pull`) but the local file still matches `last_seen_hash`.
    /// Used by [`Self::refresh_pin_sync_cache`] and by action dispatch (smart-sync); **not**
    /// for paint — presentation reads [`Self::cached_pin_sync_status`] (issue #241).
    pub(crate) fn compute_pin_sync_status(&self, index: usize) -> crate::domain::SyncStatus {
        let (local_ts, remote_ts) = self.pin_mtimes(index);
        let status = crate::domain::sync_status(local_ts, remote_ts);
        if !matches!(
            status,
            crate::domain::SyncStatus::Push | crate::domain::SyncStatus::Pull
        ) {
            return status;
        }
        let Some(m) = self.pinned.get(index) else {
            return status;
        };
        let Some(baseline) = m.last_seen_hash.as_deref() else {
            return status;
        };
        let local_abs = self.pin_local_abs(m);
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
