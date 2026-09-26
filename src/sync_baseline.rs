//! The **Sync baseline** (issue #499): what a pin remembers of its last sync, one value per
//! side, and how comparing it with now decides which side changed — Push, Pull, Conflict, or
//! in sync.
//!
//! This is the one place that knows which hash covers which side: the local file's bytes on
//! disk as a SHA-256, the gist file's content as its git blob SHA-1 (the sha in its
//! `raw_url`). Callers do the IO — read the file, look the sha up in the catalog, store the
//! value — and never hash or compare for themselves.
//!
//! A pin recorded before #466 carries only the local side; it is classified by modification
//! times until its next sync fills in the rest.

use serde::{Deserialize, Serialize};

use crate::domain::{git_blob_sha1, sha256_hex, SyncStatus};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncBaseline {
    /// Local side: SHA-256 of the local file's bytes on disk at the last sync.
    #[serde(rename = "last_seen_hash", default)]
    pub local_sha256: Option<String>,
    /// Remote side: git blob SHA-1 of the gist file's content at the last sync. Absent on pins
    /// recorded before #466; the next sync fills it in.
    #[serde(default)]
    pub remote_blob_sha: Option<String>,
}

/// The local file as it is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalFile<'a> {
    Bytes(&'a [u8]),
    /// It no longer exists.
    Missing,
    /// It exists but could not be read.
    Unreadable,
}

/// Both sides as they are now, for [`SyncBaseline::status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observed<'a> {
    pub local: LocalFile<'a>,
    /// The gist file's current blob sha, from the catalog's `raw_url`; `None` when the
    /// catalog has none for it.
    pub remote_blob_sha: Option<&'a str>,
    /// Modification times as Unix seconds, for a pin without a full baseline.
    pub local_ts: Option<u64>,
    pub remote_ts: Option<u64>,
}

impl SyncBaseline {
    /// The baseline after a sync that left the local file as `local` on disk and the gist
    /// file holding `remote`.
    pub fn after_sync(local: &[u8], remote: &[u8]) -> Self {
        Self {
            local_sha256: Some(sha256_hex(local)),
            remote_blob_sha: Some(git_blob_sha1(remote)),
        }
    }

    /// Which side changed since this baseline. With both sides recorded and a current blob
    /// sha, compare hashes (issue #466); otherwise fall back to modification times, trusting
    /// a local hash that still matches over a timestamp that says otherwise.
    pub fn status(&self, now: Observed<'_>) -> SyncStatus {
        if let (Some(local_base), Some(remote_base), Some(remote_now)) = (
            self.local_sha256.as_deref(),
            self.remote_blob_sha.as_deref(),
            now.remote_blob_sha,
        ) {
            return match now.local {
                LocalFile::Bytes(bytes) => {
                    changed_status(sha256_hex(bytes) != local_base, remote_now != remote_base)
                }
                LocalFile::Missing => SyncStatus::Missing,
                LocalFile::Unreadable => SyncStatus::Unknown,
            };
        }

        let status = newer_status(now.local_ts, now.remote_ts);
        if !matches!(status, SyncStatus::Push | SyncStatus::Pull) {
            return status;
        }
        match (self.local_sha256.as_deref(), now.local) {
            (Some(base), LocalFile::Bytes(bytes)) if sha256_hex(bytes) == base => {
                SyncStatus::InSync
            }
            _ => status,
        }
    }
}

/// Which sides changed since the last sync?
fn changed_status(local_changed: bool, remote_changed: bool) -> SyncStatus {
    match (local_changed, remote_changed) {
        (false, false) => SyncStatus::InSync,
        (true, false) => SyncStatus::Push,
        (false, true) => SyncStatus::Pull,
        (true, true) => SyncStatus::Conflict,
    }
}

/// Which side is newer? `None` means the timestamp was unavailable. A missing `local_ts`
/// always means `Missing`, regardless of `remote_ts` — that's a stronger, more actionable
/// fact than "remote timestamp unknown" (which stays `Unknown`).
fn newer_status(local_ts: Option<u64>, remote_ts: Option<u64>) -> SyncStatus {
    match (local_ts, remote_ts) {
        (None, _) => SyncStatus::Missing,
        (Some(l), Some(r)) if l > r => SyncStatus::Push,
        (Some(l), Some(r)) if r > l => SyncStatus::Pull,
        (Some(_), Some(_)) => SyncStatus::InSync,
        (Some(_), None) => SyncStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNCED: &[u8] = b"synced\n";

    fn full() -> SyncBaseline {
        SyncBaseline::after_sync(SYNCED, SYNCED)
    }

    fn local_only() -> SyncBaseline {
        SyncBaseline {
            remote_blob_sha: None,
            ..full()
        }
    }

    fn observed<'a>(local: LocalFile<'a>, remote_blob_sha: Option<&'a str>) -> Observed<'a> {
        Observed {
            local,
            remote_blob_sha,
            local_ts: Some(10),
            remote_ts: Some(10),
        }
    }

    #[test]
    fn after_sync_hashes_each_side_its_own_way() {
        let baseline = SyncBaseline::after_sync(b"local", b"hello\n");
        assert_eq!(
            baseline.local_sha256.as_deref(),
            Some(sha256_hex(b"local").as_str())
        );
        assert_eq!(
            baseline.remote_blob_sha.as_deref(),
            // `printf 'hello\n' | git hash-object --stdin`
            Some("ce013625030ba8dba906f756967f9e9ca394464a")
        );
    }

    #[test]
    fn a_full_baseline_compares_both_sides_by_hash() {
        let same = git_blob_sha1(SYNCED);
        let other = git_blob_sha1(b"changed\n");
        for (local, remote, expected) in [
            (SYNCED, &same, SyncStatus::InSync),
            (&b"edited\n"[..], &same, SyncStatus::Push),
            (SYNCED, &other, SyncStatus::Pull),
            (&b"edited\n"[..], &other, SyncStatus::Conflict),
        ] {
            let now = observed(LocalFile::Bytes(local), Some(remote));
            assert_eq!(full().status(now), expected, "{local:?} / {remote}");
        }
    }

    /// Timestamps can't overrule hashes: a file touched without changing stays in sync.
    #[test]
    fn a_full_baseline_ignores_timestamps() {
        let sha = git_blob_sha1(SYNCED);
        let now = Observed {
            local_ts: Some(99),
            remote_ts: Some(1),
            ..observed(LocalFile::Bytes(SYNCED), Some(&sha))
        };
        assert_eq!(full().status(now), SyncStatus::InSync);
    }

    #[test]
    fn a_full_baseline_reports_a_local_file_it_cannot_read() {
        let sha = git_blob_sha1(SYNCED);
        assert_eq!(
            full().status(observed(LocalFile::Missing, Some(&sha))),
            SyncStatus::Missing
        );
        assert_eq!(
            full().status(observed(LocalFile::Unreadable, Some(&sha))),
            SyncStatus::Unknown
        );
    }

    #[test]
    fn without_a_full_baseline_the_newer_side_wins() {
        for (local_ts, remote_ts, expected) in [
            (Some(20), Some(10), SyncStatus::Push),
            (Some(10), Some(20), SyncStatus::Pull),
            (Some(15), Some(15), SyncStatus::InSync),
            // Local missing takes priority, even when the remote mtime is also unknown.
            (None, Some(10), SyncStatus::Missing),
            (None, None, SyncStatus::Missing),
            (Some(10), None, SyncStatus::Unknown),
        ] {
            let now = Observed {
                local_ts,
                remote_ts,
                ..observed(LocalFile::Bytes(b"edited\n"), None)
            };
            assert_eq!(
                SyncBaseline::default().status(now),
                expected,
                "{local_ts:?} / {remote_ts:?}"
            );
        }
    }

    /// A pre-#466 pin, or a gist file the catalog has no sha for, still has its local hash:
    /// a local file that matches it is in sync whatever the timestamps say.
    #[test]
    fn a_matching_local_hash_overrules_the_timestamps() {
        for (baseline, remote_blob_sha) in [(local_only(), Some("0".repeat(40))), (full(), None)] {
            for (local_ts, remote_ts) in [(Some(20), Some(10)), (Some(10), Some(20))] {
                let now = Observed {
                    local_ts,
                    remote_ts,
                    ..observed(LocalFile::Bytes(SYNCED), remote_blob_sha.as_deref())
                };
                assert_eq!(baseline.status(now), SyncStatus::InSync);
            }
        }
    }

    #[test]
    fn a_differing_local_hash_keeps_the_timestamp_verdict() {
        for local in [
            LocalFile::Bytes(b"edited\n"),
            LocalFile::Missing,
            LocalFile::Unreadable,
        ] {
            let now = Observed {
                local_ts: Some(10),
                remote_ts: Some(20),
                ..observed(local, None)
            };
            assert_eq!(local_only().status(now), SyncStatus::Pull, "{local:?}");
        }
    }

    #[test]
    fn toml_keeps_the_flat_keys() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Pin {
            gist_id: String,
            #[serde(flatten)]
            baseline: SyncBaseline,
        }
        let pin = Pin {
            gist_id: "g1".into(),
            baseline: SyncBaseline {
                local_sha256: Some("l".into()),
                remote_blob_sha: None,
            },
        };
        let text = toml::to_string(&pin).unwrap();
        assert_eq!(text, "gist_id = \"g1\"\nlast_seen_hash = \"l\"\n");
        assert_eq!(toml::from_str::<Pin>(&text).unwrap(), pin);
        assert_eq!(
            toml::from_str::<Pin>("gist_id = \"g1\"\n")
                .unwrap()
                .baseline,
            SyncBaseline::default()
        );
    }
}
