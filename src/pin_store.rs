//! Pinned-mapping **persistence** (issue #432): the one interface that owns a complete
//! read-modify-write of the pins stored in `config.toml`.
//!
//! [`crate::pins`] stays pure — it defines what a pin *is* (the three-part key) and the
//! list operations. This module owns everything around that: resolving the config path,
//! loading, applying the stored-versus-absolute path rule, hashing sync content, saving,
//! and describing what changed so a caller can project it without re-deriving policy.
//!
//! It deliberately knows nothing about the TUI. Status wording, cursor policy, and the
//! `AppState` projection stay with the screens; a Pins-screen row index is a filtered-view
//! concept and never reaches this interface — callers resolve one into a [`PinKey`] first.
//!
//! Every operation performs a full load → mutate → save cycle. `config.toml` is
//! hand-editable, so writing from a cached copy would silently clobber an edit made
//! between two pin operations.

use crate::config::{load_config, save_config, AppConfig};
use crate::domain::{PinnedMapping, SyncDirection};
use crate::pins::{self, PinKey};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// The config fields a successful operation leaves behind, for the caller to project.
///
/// Both fields travel together on every operation: after a load, "what was just read" is
/// the correct value for both, even when only one of them could have changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinChange {
    pub pinned: Vec<PinnedMapping>,
    pub skip_dirs: Vec<String>,
}

impl From<AppConfig> for PinChange {
    fn from(config: AppConfig) -> Self {
        Self {
            pinned: config.pinned,
            skip_dirs: config.skip_dirs,
        }
    }
}

/// What [`PinStore::unpin`] found. The status line already distinguishes these two, so
/// the fact travels as a value rather than a discarded `bool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unpinned {
    Removed,
    /// Nothing matched the key — the stored config never held that pair.
    NotFound,
}

/// What [`PinStore::record_sync`] did. Confirming a sync must never *create* a pin, so an
/// unpinned pair is a normal outcome, not a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncRecord {
    Recorded,
    NotPinned,
}

/// The pins stored in one `config.toml`.
pub struct PinStore {
    config_path: PathBuf,
}

impl PinStore {
    /// The user's real config file.
    pub fn in_default_location() -> Result<Self> {
        Ok(Self::at(crate::config::config_path()?))
    }

    /// A store over an explicit path. Tests point this at a temporary directory, which is
    /// why no filesystem seam is needed.
    pub fn at(config_path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: config_path.into(),
        }
    }

    /// Pin the pair named by `key`, or leave an existing pin alone. Siblings sharing the
    /// local path are untouched — see [`crate::pins`] for why that is the invariant.
    ///
    /// The stored `local_path` is exactly what `key` carries: this is a parser-and-writer
    /// of the user's file, not a normaliser of it.
    pub fn pin(&self, key: PinKey<'_>) -> Result<PinChange> {
        let mut config = load_config(&self.config_path)?;
        pins::upsert(&mut config.pinned, key);
        save_config(&self.config_path, &config)?;
        Ok(config.into())
    }

    /// Remove the one pin named by `key`. Persists only when something was removed, so an
    /// unmatched key does not rewrite the file.
    pub fn unpin(&self, key: PinKey<'_>) -> Result<(PinChange, Unpinned)> {
        let mut config = load_config(&self.config_path)?;
        let outcome = if pins::remove(&mut config.pinned, key) {
            save_config(&self.config_path, &config)?;
            Unpinned::Removed
        } else {
            Unpinned::NotFound
        };
        Ok((config.into(), outcome))
    }

    /// Record that `content` is now known to match the gist side for `pair`.
    ///
    /// `pair.local_path` is an absolute path the app resolved for itself; the stored entry
    /// may be relative, so the pin is *found* by its resolved form and *written* by its
    /// stored form. `direction: None` means the match was passively confirmed (a diff
    /// turned out identical) and leaves the recorded direction alone.
    ///
    /// Hashing lives here because `last_seen_hash` being a hex SHA-256 is a fact about
    /// this file format, not something each caller should know.
    pub fn record_sync(
        &self,
        cwd: &Path,
        pair: PinKey<'_>,
        content: &str,
        direction: Option<SyncDirection>,
    ) -> Result<(PinChange, SyncRecord)> {
        let mut config = load_config(&self.config_path)?;
        let Some(index) = pins::find_by_resolved_path(&config.pinned, cwd, pair) else {
            return Ok((config.into(), SyncRecord::NotPinned));
        };
        let mapping = &mut config.pinned[index];
        mapping.last_seen_hash = Some(crate::domain::sha256_hex(content.as_bytes()));
        if let Some(direction) = direction {
            mapping.direction = Some(direction);
        }
        save_config(&self.config_path, &config)?;
        Ok((config.into(), SyncRecord::Recorded))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::sha256_hex;
    use std::path::PathBuf;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: PinStore,
        path: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        Fixture {
            _dir: dir,
            store: PinStore::at(&path),
            path,
        }
    }

    fn stored(f: &Fixture) -> Vec<PinnedMapping> {
        load_config(&f.path).expect("reload").pinned
    }

    fn key<'a>(local: &'a Path, gist_id: &'a str, filename: &'a str) -> PinKey<'a> {
        PinKey::new(local, gist_id, filename)
    }

    fn seed(f: &Fixture, pinned: Vec<PinnedMapping>) {
        let mut config = load_config(&f.path).expect("load");
        config.pinned = pinned;
        save_config(&f.path, &config).expect("seed");
    }

    fn mapping(local: &str, gist_id: &str, filename: &str) -> PinnedMapping {
        PinnedMapping {
            local_path: PathBuf::from(local),
            gist_id: gist_id.into(),
            gist_filename: filename.into(),
            direction: None,
            last_seen_hash: None,
        }
    }

    // ---- pin -------------------------------------------------------------

    #[test]
    fn pin_persists_the_pair_it_was_given() {
        let f = fixture();

        let change = f
            .store
            .pin(key(
                Path::new("/abs/settings.json"),
                "abc123",
                "settings.json",
            ))
            .expect("pin");

        assert_eq!(change.pinned.len(), 1);
        let stored = stored(&f);
        assert_eq!(stored[0].local_path, PathBuf::from("/abs/settings.json"));
        assert_eq!(stored[0].gist_id, "abc123");
        assert_eq!(stored[0].gist_filename, "settings.json");
    }

    /// The bug #424 fixed: pinning one local file to a second gist file used to drop the
    /// first pin. Persistence must not reintroduce it.
    #[test]
    fn pin_keeps_a_sibling_on_the_same_local_path() {
        let f = fixture();
        let local = Path::new("/abs/settings.json");

        f.store.pin(key(local, "abc123", "a.txt")).expect("first");
        let change = f.store.pin(key(local, "abc123", "b.txt")).expect("second");

        assert_eq!(change.pinned.len(), 2);
        let stored = stored(&f);
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].gist_filename, "a.txt");
        assert_eq!(stored[1].gist_filename, "b.txt");
    }

    /// A `None` direction and hash mean "leave what is stored alone" — re-pinning an
    /// already-pinned pair must not erase what an earlier sync learned.
    #[test]
    fn pinning_an_existing_pair_preserves_its_recorded_sync_state() {
        let f = fixture();
        seed(
            &f,
            vec![PinnedMapping {
                direction: Some(SyncDirection::Upload),
                last_seen_hash: Some("known".into()),
                ..mapping("/abs/a.txt", "g1", "a.txt")
            }],
        );

        f.store
            .pin(key(Path::new("/abs/a.txt"), "g1", "a.txt"))
            .expect("pin");

        let stored = stored(&f);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].direction, Some(SyncDirection::Upload));
        assert_eq!(stored[0].last_seen_hash.as_deref(), Some("known"));
    }

    // ---- unpin -----------------------------------------------------------

    #[test]
    fn unpin_persists_the_removal_and_reports_it() {
        let f = fixture();
        let local = Path::new("/abs/a.txt");
        f.store.pin(key(local, "g1", "a.txt")).expect("pin");

        let (change, outcome) = f.store.unpin(key(local, "g1", "a.txt")).expect("unpin");

        assert_eq!(outcome, Unpinned::Removed);
        assert!(change.pinned.is_empty());
        assert!(stored(&f).is_empty());
    }

    #[test]
    fn unpin_reports_a_key_the_stored_config_never_held() {
        let f = fixture();
        seed(&f, vec![mapping("/abs/a.txt", "g1", "a.txt")]);

        let (change, outcome) = f
            .store
            .unpin(key(Path::new("/abs/a.txt"), "g1", "b.txt"))
            .expect("unpin");

        assert_eq!(outcome, Unpinned::NotFound);
        assert_eq!(
            change.pinned.len(),
            1,
            "the untouched pin is still reported"
        );
        assert_eq!(stored(&f).len(), 1);
    }

    #[test]
    fn unpin_spares_siblings_on_the_same_local_path() {
        let f = fixture();
        let local = Path::new("/abs/a.txt");
        f.store.pin(key(local, "g1", "a.txt")).expect("pin a");
        f.store.pin(key(local, "g1", "b.txt")).expect("pin b");

        f.store.unpin(key(local, "g1", "a.txt")).expect("unpin");

        let stored = stored(&f);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].gist_filename, "b.txt");
    }

    // ---- record_sync -----------------------------------------------------

    #[test]
    fn record_sync_stores_the_content_hash_and_direction() {
        let f = fixture();
        seed(&f, vec![mapping("/abs/a.txt", "g1", "a.txt")]);

        let (_, outcome) = f
            .store
            .record_sync(
                Path::new("/cwd"),
                key(Path::new("/abs/a.txt"), "g1", "a.txt"),
                "body\n",
                Some(SyncDirection::Upload),
            )
            .expect("record");

        assert_eq!(outcome, SyncRecord::Recorded);
        let stored = stored(&f);
        assert_eq!(
            stored[0].last_seen_hash.as_deref(),
            Some(sha256_hex(b"body\n").as_str()),
            "the hash is computed here, not by the caller"
        );
        assert_eq!(stored[0].direction, Some(SyncDirection::Upload));
    }

    /// `None` means "confirmed equal", not "synced" — the hash still updates, but the
    /// recorded direction must survive.
    #[test]
    fn record_sync_without_a_direction_updates_the_hash_only() {
        let f = fixture();
        seed(
            &f,
            vec![PinnedMapping {
                direction: Some(SyncDirection::Download),
                last_seen_hash: Some("stale".into()),
                ..mapping("/abs/a.txt", "g1", "a.txt")
            }],
        );

        f.store
            .record_sync(
                Path::new("/cwd"),
                key(Path::new("/abs/a.txt"), "g1", "a.txt"),
                "fresh\n",
                None,
            )
            .expect("record");

        let stored = stored(&f);
        assert_eq!(
            stored[0].last_seen_hash.as_deref(),
            Some(sha256_hex(b"fresh\n").as_str())
        );
        assert_eq!(stored[0].direction, Some(SyncDirection::Download));
    }

    #[test]
    fn record_sync_never_creates_a_pin() {
        let f = fixture();

        let (change, outcome) = f
            .store
            .record_sync(
                Path::new("/cwd"),
                key(Path::new("/abs/a.txt"), "g1", "a.txt"),
                "body\n",
                Some(SyncDirection::Upload),
            )
            .expect("record");

        assert_eq!(outcome, SyncRecord::NotPinned);
        assert!(change.pinned.is_empty());
        assert!(stored(&f).is_empty(), "confirming a sync must not pin");
    }

    /// The stored-versus-absolute rule: a relative entry is found by the absolute path the
    /// app resolved, and stays relative in the file afterwards. Writing back the resolved
    /// form would duplicate the pin.
    #[test]
    fn record_sync_finds_a_relative_pin_and_leaves_it_relative() {
        let f = fixture();
        seed(&f, vec![mapping("a.txt", "g1", "a.txt")]);

        let (_, outcome) = f
            .store
            .record_sync(
                Path::new("/cwd"),
                key(Path::new("/cwd/a.txt"), "g1", "a.txt"),
                "body\n",
                Some(SyncDirection::Upload),
            )
            .expect("record");

        assert_eq!(outcome, SyncRecord::Recorded);
        let stored = stored(&f);
        assert_eq!(stored.len(), 1, "no duplicate absolute entry was created");
        assert_eq!(
            stored[0].local_path,
            PathBuf::from("a.txt"),
            "a portable relative path is not rewritten"
        );
        assert!(stored[0].last_seen_hash.is_some());
    }

    // ---- the projection --------------------------------------------------

    /// Every operation reports both fields, so a caller projects "what was just read"
    /// rather than guessing which one its operation could have touched.
    #[test]
    fn every_operation_reports_both_projected_fields() {
        let f = fixture();
        let mut config = load_config(&f.path).expect("load");
        config.skip_dirs = vec!["node_modules".into()];
        config.pinned = vec![mapping("/abs/a.txt", "g1", "a.txt")];
        save_config(&f.path, &config).expect("seed");
        let local = Path::new("/abs/a.txt");

        let pinned = f.store.pin(key(local, "g1", "b.txt")).expect("pin");
        let (unpinned, _) = f.store.unpin(key(local, "g1", "b.txt")).expect("unpin");
        let (synced, _) = f
            .store
            .record_sync(Path::new("/cwd"), key(local, "g1", "a.txt"), "x", None)
            .expect("record");

        for change in [pinned, unpinned, synced] {
            assert_eq!(change.skip_dirs, vec!["node_modules".to_string()]);
            assert!(!change.pinned.is_empty());
        }
    }

    // ---- failures that used to be silent ---------------------------------

    /// An unparseable `config.toml` is an error, not an empty config. Before #432 this
    /// vanished into a bare `if let Ok(...)` and the user saw nothing.
    #[test]
    fn an_unreadable_config_surfaces_as_an_error() {
        let f = fixture();
        std::fs::write(&f.path, "this is not toml {{{").expect("write");

        assert!(f
            .store
            .pin(key(Path::new("/abs/a.txt"), "g1", "a.txt"))
            .is_err());
        assert!(f
            .store
            .unpin(key(Path::new("/abs/a.txt"), "g1", "a.txt"))
            .is_err());
        assert!(f
            .store
            .record_sync(
                Path::new("/cwd"),
                key(Path::new("/abs/a.txt"), "g1", "a.txt"),
                "x",
                None
            )
            .is_err());
    }

    /// A path that cannot be written surfaces too — the write is the whole point of the
    /// operation, so losing it must not look like success.
    #[test]
    fn an_unwritable_config_path_surfaces_as_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A regular file where the config's parent directory should be, so `create_dir_all`
        // in `save_config` cannot succeed.
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, b"x").expect("write blocker");
        let store = PinStore::at(blocker.join("config.toml"));

        assert!(store
            .pin(key(Path::new("/abs/a.txt"), "g1", "a.txt"))
            .is_err());
    }
}
