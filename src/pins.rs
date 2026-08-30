//! The pinned-mapping list and the one key that decides what a pin *is* (issue #424).
//!
//! A pin is identified by **all three** of `local_path`, `gist_id`, and `gist_filename` —
//! `docs/design.md` defines a pin as "a persistent local-file to gist-file mapping", and
//! `gist_id` alone cannot name a file inside a gist. One local file may therefore be pinned
//! to several gist files at once; those siblings are legitimate, not a corrupt state.
//!
//! Every read and write of `AppConfig::pinned` goes through [`PinKey`] and the operations
//! below, so the invariant is stated once here instead of being re-derived per call site.
//! `crate::actions` holds the thin wrappers that also persist the config.
//!
//! Exactly-duplicate triples are degenerate input — `config.toml` is user-editable and
//! `crate::config::load_config` deliberately stays a parser rather than a silent rewriter.
//! [`find`], [`find_mut`], and [`remove`] all act on the first match.

use crate::domain::{PinnedMapping, SyncDirection};
use std::path::Path;

/// What makes a pinned mapping unique. Borrowed so callers that already hold a
/// [`PinnedMapping`] can pass [`PinnedMapping::key`] without cloning three fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinKey<'a> {
    pub local_path: &'a Path,
    pub gist_id: &'a str,
    pub gist_filename: &'a str,
}

impl<'a> PinKey<'a> {
    pub fn new(local_path: &'a Path, gist_id: &'a str, gist_filename: &'a str) -> Self {
        Self {
            local_path,
            gist_id,
            gist_filename,
        }
    }

    fn matches(&self, mapping: &PinnedMapping) -> bool {
        mapping.local_path == self.local_path
            && mapping.gist_id == self.gist_id
            && mapping.gist_filename == self.gist_filename
    }
}

impl PinnedMapping {
    /// This mapping's identity. Use it instead of re-listing the three fields at a call
    /// site — dropping one is exactly how unpin used to remove the wrong row.
    pub fn key(&self) -> PinKey<'_> {
        PinKey {
            local_path: &self.local_path,
            gist_id: &self.gist_id,
            gist_filename: &self.gist_filename,
        }
    }
}

pub fn find<'a>(pinned: &'a [PinnedMapping], key: PinKey<'_>) -> Option<&'a PinnedMapping> {
    pinned.iter().find(|m| key.matches(m))
}

pub fn find_mut<'a>(
    pinned: &'a mut [PinnedMapping],
    key: PinKey<'_>,
) -> Option<&'a mut PinnedMapping> {
    pinned.iter_mut().find(|m| key.matches(m))
}

/// Insert `key`'s mapping, or update the one already there. Siblings sharing `local_path`
/// with a different gist file are left alone.
///
/// `None` means **leave the stored value alone**, matching the semantics
/// `crate::actions::record_sync` already documents: a caller that does not know the
/// direction or hash must not erase what an earlier sync learned.
pub fn upsert(
    pinned: &mut Vec<PinnedMapping>,
    key: PinKey<'_>,
    direction: Option<SyncDirection>,
    last_seen_hash: Option<String>,
) {
    if let Some(existing) = find_mut(pinned, key) {
        if direction.is_some() {
            existing.direction = direction;
        }
        if last_seen_hash.is_some() {
            existing.last_seen_hash = last_seen_hash;
        }
        return;
    }
    pinned.push(PinnedMapping {
        local_path: key.local_path.to_path_buf(),
        gist_id: key.gist_id.to_string(),
        gist_filename: key.gist_filename.to_string(),
        direction,
        last_seen_hash,
    });
}

/// Remove the first mapping matching `key`. Returns whether anything was removed, so a
/// caller can skip persisting an unchanged config.
pub fn remove(pinned: &mut Vec<PinnedMapping>, key: PinKey<'_>) -> bool {
    let Some(index) = pinned.iter().position(|m| key.matches(m)) else {
        return false;
    };
    pinned.remove(index);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mapping(local: &str, gist_id: &str, filename: &str) -> PinnedMapping {
        PinnedMapping {
            local_path: PathBuf::from(local),
            gist_id: gist_id.into(),
            gist_filename: filename.into(),
            direction: None,
            last_seen_hash: None,
        }
    }

    fn key<'a>(local: &'a Path, gist_id: &'a str, filename: &'a str) -> PinKey<'a> {
        PinKey::new(local, gist_id, filename)
    }

    // ---- the key --------------------------------------------------------

    #[test]
    fn a_mappings_key_round_trips_to_itself() {
        let m = mapping("/a.txt", "g1", "a.txt");
        assert!(find(std::slice::from_ref(&m), m.key()).is_some());
    }

    #[test]
    fn differing_in_the_gist_filename_alone_is_a_different_pin() {
        let pinned = vec![mapping("/a.txt", "g1", "a.txt")];
        let other = key(Path::new("/a.txt"), "g1", "b.txt");
        assert!(find(&pinned, other).is_none());
    }

    // ---- upsert ---------------------------------------------------------

    #[test]
    fn upsert_keeps_siblings_sharing_a_local_path() {
        let mut pinned = vec![mapping("/a.txt", "g1", "a.txt")];

        upsert(
            &mut pinned,
            key(Path::new("/a.txt"), "g1", "b.txt"),
            None,
            None,
        );

        assert_eq!(pinned.len(), 2);
        assert_eq!(pinned[0].gist_filename, "a.txt");
        assert_eq!(pinned[1].gist_filename, "b.txt");
    }

    #[test]
    fn upsert_of_an_existing_pin_does_not_duplicate_it() {
        let mut pinned = vec![mapping("/a.txt", "g1", "a.txt")];

        upsert(
            &mut pinned,
            key(Path::new("/a.txt"), "g1", "a.txt"),
            None,
            None,
        );

        assert_eq!(pinned.len(), 1);
    }

    #[test]
    fn upsert_with_none_leaves_a_recorded_hash_and_direction_alone() {
        let mut pinned = vec![PinnedMapping {
            direction: Some(SyncDirection::Upload),
            last_seen_hash: Some("known".into()),
            ..mapping("/a.txt", "g1", "a.txt")
        }];

        upsert(
            &mut pinned,
            key(Path::new("/a.txt"), "g1", "a.txt"),
            None,
            None,
        );

        assert_eq!(pinned[0].direction, Some(SyncDirection::Upload));
        assert_eq!(pinned[0].last_seen_hash.as_deref(), Some("known"));
    }

    #[test]
    fn upsert_with_some_overwrites_the_stored_values() {
        let mut pinned = vec![PinnedMapping {
            direction: Some(SyncDirection::Upload),
            last_seen_hash: Some("old".into()),
            ..mapping("/a.txt", "g1", "a.txt")
        }];

        upsert(
            &mut pinned,
            key(Path::new("/a.txt"), "g1", "a.txt"),
            Some(SyncDirection::Download),
            Some("new".into()),
        );

        assert_eq!(pinned[0].direction, Some(SyncDirection::Download));
        assert_eq!(pinned[0].last_seen_hash.as_deref(), Some("new"));
    }

    // ---- remove ---------------------------------------------------------

    #[test]
    fn remove_takes_only_the_named_pin_and_spares_its_siblings() {
        let mut pinned = vec![
            mapping("/a.txt", "g1", "a.txt"),
            mapping("/a.txt", "g1", "b.txt"),
            mapping("/a.txt", "g2", "a.txt"),
        ];

        assert!(remove(&mut pinned, key(Path::new("/a.txt"), "g1", "b.txt")));

        assert_eq!(pinned.len(), 2);
        assert_eq!(pinned[0].gist_filename, "a.txt");
        assert_eq!(pinned[1].gist_id, "g2");
    }

    #[test]
    fn remove_reports_no_match_without_touching_the_list() {
        let mut pinned = vec![mapping("/a.txt", "g1", "a.txt")];

        assert!(!remove(
            &mut pinned,
            key(Path::new("/a.txt"), "nope", "a.txt")
        ));

        assert_eq!(pinned.len(), 1);
    }

    #[test]
    fn remove_takes_the_first_of_two_exact_duplicates() {
        // Degenerate input a hand-edited config.toml can produce; documented, not repaired.
        let mut pinned = vec![
            PinnedMapping {
                last_seen_hash: Some("first".into()),
                ..mapping("/a.txt", "g1", "a.txt")
            },
            PinnedMapping {
                last_seen_hash: Some("second".into()),
                ..mapping("/a.txt", "g1", "a.txt")
            },
        ];

        assert!(remove(&mut pinned, key(Path::new("/a.txt"), "g1", "a.txt")));

        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].last_seen_hash.as_deref(), Some("second"));
    }
}
