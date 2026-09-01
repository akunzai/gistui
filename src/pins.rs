//! The pinned-mapping list and the one key that decides what a pin *is* (issue #424).
//!
//! A pin is identified by **all three** of `local_path`, `gist_id`, and `gist_filename` —
//! `docs/design.md` defines a pin as "a persistent local-file to gist-file mapping", and
//! `gist_id` alone cannot name a file inside a gist. One local file may therefore be pinned
//! to several gist files at once; those siblings are legitimate, not a corrupt state.
//!
//! Every read and write of `AppConfig::pinned` goes through [`PinKey`] and the operations
//! below, so the invariant is stated once here instead of being re-derived per call site.
//! `crate::pin_store` owns persistence; this module stays pure.
//!
//! Exactly-duplicate triples are degenerate input — `config.toml` is user-editable and
//! `crate::config::load_config` deliberately stays a parser rather than a silent rewriter.
//! [`find_mut`] and [`remove`] both act on the first match.

use crate::domain::PinnedMapping;
use std::path::{Path, PathBuf};

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

    /// Whether `mapping` is the pin this key names. Use it instead of spelling the three
    /// comparisons out at a call site — that is how they drifted apart before #424.
    pub fn matches(&self, mapping: &PinnedMapping) -> bool {
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

/// Whether the pair named by `key` is pinned. The presentation layer asks this a lot —
/// row marks, the List `p` toggle — and each caller used to spell the three comparisons
/// out for itself.
pub fn is_pinned(pinned: &[PinnedMapping], key: PinKey<'_>) -> bool {
    pinned.iter().any(|m| key.matches(m))
}

fn find_mut<'a>(pinned: &'a mut [PinnedMapping], key: PinKey<'_>) -> Option<&'a mut PinnedMapping> {
    pinned.iter_mut().find(|m| key.matches(m))
}

/// Insert `key`'s mapping if it is not already there. Siblings sharing `local_path` with a
/// different gist file are left alone.
///
/// An existing pin is left **completely** untouched: pinning is not how a sync direction or
/// hash gets recorded (that is `crate::pin_store::PinStore::record_sync`), so re-pinning
/// must never erase what an earlier sync learned.
pub fn upsert(pinned: &mut Vec<PinnedMapping>, key: PinKey<'_>) {
    if find_mut(pinned, key).is_some() {
        return;
    }
    pinned.push(PinnedMapping {
        local_path: key.local_path.to_path_buf(),
        gist_id: key.gist_id.to_string(),
        gist_filename: key.gist_filename.to_string(),
        direction: None,
        last_seen_hash: None,
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

/// Resolve a stored `local_path` against `cwd`.
///
/// `config.toml` is hand-editable, so a stored path may be relative — a portable config
/// pinned relative to the project root is a legitimate thing to write. Every comparison
/// against a path the app resolved itself has to go through this first. Defined once here
/// (issue #432) because it previously existed as three copies that could drift.
pub fn resolve_against(local_path: &Path, cwd: &Path) -> PathBuf {
    if local_path.is_absolute() {
        local_path.to_path_buf()
    } else {
        cwd.join(local_path)
    }
}

impl PinnedMapping {
    /// This mapping's `local_path` as an absolute path (see [`resolve_against`]).
    pub fn resolve_against(&self, cwd: &Path) -> PathBuf {
        resolve_against(&self.local_path, cwd)
    }
}

/// Index of the pin that `pair` names, comparing local paths **after** resolution.
///
/// `pair.local_path` is an absolute path the app resolved for itself, while a stored entry
/// may be relative — so this is the one lookup that cannot use [`PinKey::matches`]. It
/// returns an index rather than the mapping because both callers need to reach back into
/// the list they searched: one to mutate that entry in place (keeping its **stored** path
/// form, since writing the resolved form back would duplicate the pin), the other to feed
/// the Pins screen's index-keyed sync-status lookup.
pub fn find_by_resolved_path(
    pinned: &[PinnedMapping],
    cwd: &Path,
    pair: PinKey<'_>,
) -> Option<usize> {
    pinned.iter().position(|m| {
        m.gist_id == pair.gist_id
            && m.gist_filename == pair.gist_filename
            && m.resolve_against(cwd) == pair.local_path
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SyncDirection;
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

    // ---- resolve_against ------------------------------------------------

    #[test]
    fn an_absolute_stored_path_passes_through_unchanged() {
        let m = mapping("/abs/a.txt", "g1", "a.txt");
        assert_eq!(
            m.resolve_against(Path::new("/cwd")),
            PathBuf::from("/abs/a.txt")
        );
    }

    #[test]
    fn a_relative_stored_path_joins_the_cwd() {
        let m = mapping("nested/a.txt", "g1", "a.txt");
        assert_eq!(
            m.resolve_against(Path::new("/cwd")),
            PathBuf::from("/cwd/nested/a.txt")
        );
    }

    // ---- find_by_resolved_path ------------------------------------------

    /// A relative entry is reachable by the absolute path the app resolved for itself —
    /// that is the whole reason the stored form cannot be compared directly.
    #[test]
    fn a_relative_pin_is_found_by_its_resolved_absolute_path() {
        let pinned = vec![mapping("a.txt", "g1", "a.txt")];

        let index = find_by_resolved_path(
            &pinned,
            Path::new("/cwd"),
            key(Path::new("/cwd/a.txt"), "g1", "a.txt"),
        )
        .expect("resolved match");

        assert_eq!(index, 0);
        assert_eq!(
            pinned[index].local_path,
            PathBuf::from("a.txt"),
            "the index reaches the entry in its stored form, so a write keys off that"
        );
    }

    #[test]
    fn find_by_resolved_path_still_needs_the_gist_file_to_agree() {
        let pinned = vec![mapping("/cwd/a.txt", "g1", "a.txt")];
        let cwd = Path::new("/cwd");
        let abs = Path::new("/cwd/a.txt");

        assert!(find_by_resolved_path(&pinned, cwd, key(abs, "g1", "a.txt")).is_some());
        assert!(find_by_resolved_path(&pinned, cwd, key(abs, "g2", "a.txt")).is_none());
        assert!(find_by_resolved_path(&pinned, cwd, key(abs, "g1", "b.txt")).is_none());
        assert!(
            find_by_resolved_path(&pinned, cwd, key(Path::new("/cwd/b.txt"), "g1", "a.txt"))
                .is_none()
        );
    }

    #[test]
    fn find_by_resolved_path_reports_the_index_of_a_later_match() {
        let pinned = vec![
            mapping("/cwd/a.txt", "g1", "a.txt"),
            mapping("/cwd/b.txt", "g1", "b.txt"),
        ];

        let index = find_by_resolved_path(
            &pinned,
            Path::new("/cwd"),
            key(Path::new("/cwd/b.txt"), "g1", "b.txt"),
        )
        .expect("match");

        assert_eq!(index, 1);
    }

    // ---- the key --------------------------------------------------------

    #[test]
    fn a_mappings_key_round_trips_to_itself() {
        let m = mapping("/a.txt", "g1", "a.txt");
        assert!(m.key().matches(&m));
    }

    #[test]
    fn differing_in_the_gist_filename_alone_is_a_different_pin() {
        let mut pinned = vec![mapping("/a.txt", "g1", "a.txt")];
        let other = key(Path::new("/a.txt"), "g1", "b.txt");
        assert!(find_mut(&mut pinned, other).is_none());
    }

    // ---- upsert ---------------------------------------------------------

    #[test]
    fn upsert_keeps_siblings_sharing_a_local_path() {
        let mut pinned = vec![mapping("/a.txt", "g1", "a.txt")];

        upsert(&mut pinned, key(Path::new("/a.txt"), "g1", "b.txt"));

        assert_eq!(pinned.len(), 2);
        assert_eq!(pinned[0].gist_filename, "a.txt");
        assert_eq!(pinned[1].gist_filename, "b.txt");
    }

    /// Re-pinning must not duplicate the entry, and must not disturb what an earlier sync
    /// recorded on it — pinning is not how a direction or hash is written.
    #[test]
    fn upsert_of_an_existing_pin_leaves_it_completely_alone() {
        let mut pinned = vec![PinnedMapping {
            direction: Some(SyncDirection::Upload),
            last_seen_hash: Some("known".into()),
            ..mapping("/a.txt", "g1", "a.txt")
        }];

        upsert(&mut pinned, key(Path::new("/a.txt"), "g1", "a.txt"));

        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].direction, Some(SyncDirection::Upload));
        assert_eq!(pinned[0].last_seen_hash.as_deref(), Some("known"));
    }

    // ---- is_pinned ------------------------------------------------------

    #[test]
    fn is_pinned_needs_all_three_components_to_agree() {
        let pinned = vec![mapping("/a.txt", "g1", "a.txt")];

        assert!(is_pinned(&pinned, key(Path::new("/a.txt"), "g1", "a.txt")));
        assert!(!is_pinned(&pinned, key(Path::new("/b.txt"), "g1", "a.txt")));
        assert!(!is_pinned(&pinned, key(Path::new("/a.txt"), "g2", "a.txt")));
        assert!(!is_pinned(&pinned, key(Path::new("/a.txt"), "g1", "b.txt")));
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
