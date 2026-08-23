//! Dual-pane list-pane ranking (issue #313): filters, ranks, and sorts the local/gist
//! panes. Cross-cutting rather than `Screen::List`-scoped — `bg.rs`'s background-task
//! absorb handlers call [`AppState::selected_local`] / [`AppState::ranked_gists`] directly,
//! not gated on `Screen::List` (`List` stays a unit tag; dual-pane selection/filters/sorts
//! are session-global on `AppState`; see `docs/agents/architecture.md`).
//!
//! Named `list_ranking`, not `ranking`, to avoid colliding with the top-level pure module
//! `crate::ranking` — the scoring algorithm these methods call into.
//!
//! Cost model:
//! - [`AppState::ranked_gists`] / [`AppState::visible_locals`] build **only the side they
//!   return** (plus a minimal driver peek for reverse-rank when the other pane is anchor).
//! - [`AppState::list_pane_snapshots`] builds **both** once — prefer when a handler needs
//!   both sides or both selections.
//! - [`AppState::selected_local`] / [`AppState::selected_gist`] use the single-side paths.

use crate::domain::{GistFile, LocalCandidate};
use crate::ranking::{
    rank_gist_files, rank_local_files, unranked_gist, unranked_local, RankedGistFile, RankedLocal,
};
use crate::tui::{AppState, FocusPane};

impl AppState {
    /// Filtered owned/starred gist file rows (no ranking/sort).
    fn filtered_gist_files(&self) -> Vec<GistFile> {
        let query = self.filter_query.to_lowercase();
        self.list_gist_source()
            .iter()
            .filter(|g| self.gist_type_filter.matches_file(g))
            .filter(|g| {
                query.is_empty()
                    || g.filename.to_lowercase().contains(&query)
                    || g.description.to_lowercase().contains(&query)
            })
            .cloned()
            .collect()
    }

    /// Rank/sort gist files for a known local path (or unranked when `local_path` is
    /// `None` / anchor is Gist). Does **not** call `selected_local` / `visible_locals`.
    fn rank_gist_files_for(&self, local_path: Option<&std::path::Path>) -> Vec<RankedGistFile> {
        let gists = self.filtered_gist_files();
        let mut ranked = match local_path {
            Some(path) => rank_gist_files(path, &gists, &self.pinned),
            None => gists.into_iter().map(unranked_gist).collect(),
        };
        self.gist_sort.apply(&mut ranked);
        ranked
    }

    /// Local rows with optional reverse-rank against a known gist file. Does **not**
    /// call `selected_gist` / `ranked_gists`.
    fn rank_local_files_for(&self, gist: Option<&GistFile>) -> Vec<RankedLocal> {
        let mut ranked = match gist {
            Some(file) => rank_local_files(file, &self.locals, &self.pinned),
            None => self.locals.iter().cloned().map(unranked_local).collect(),
        };
        let query = self.local_filter_query.to_lowercase();
        if !query.is_empty() {
            ranked.retain(|r| {
                crate::tui::text::local_row_label(&r.candidate.path, &self.cwd)
                    .to_lowercase()
                    .contains(&query)
            });
        }
        self.local_sort.apply(&mut ranked);
        ranked
    }

    /// Gist pane order. Single-side: only builds gists (plus a locals peek when the local
    /// pane is the ranking anchor). Prefer [`Self::list_pane_snapshots`] when both panes
    /// are needed.
    pub fn ranked_gists(&self) -> Vec<RankedGistFile> {
        // Anchor-driven ranking: gists rank against the selected local only while LOCAL
        // is the anchor. When Gist is the anchor, no reverse rank — also breaks the
        // mutual dependency with `visible_locals`.
        // Only evaluate local selection inside the Local-anchor branch (eager compute
        // would recurse: selected_local → visible_locals → selected_gist → ranked_gists).
        let local_path = if self.anchor == FocusPane::Local {
            self.rank_local_files_for(None)
                .get(self.local_index)
                .map(|r| r.candidate.path.clone())
        } else {
            None
        };
        self.rank_gist_files_for(local_path.as_deref())
    }

    /// Local pane order. Single-side: only builds locals (plus a gists peek when the gist
    /// pane is the ranking anchor). Prefer [`Self::list_pane_snapshots`] when both panes
    /// are needed.
    pub fn visible_locals(&self) -> Vec<RankedLocal> {
        let gist = if self.anchor == FocusPane::Gist {
            self.rank_gist_files_for(None)
                .into_iter()
                .nth(self.gist_index)
                .map(|r| r.file)
        } else {
            None
        };
        self.rank_local_files_for(gist.as_ref())
    }

    /// Build both list-pane orderings with **one** construction of each list (issue #224).
    /// Prefer when a key/palette/render path needs both sides or both selections.
    ///
    /// Expansion order follows the anchor so mutual recursion is not entered:
    /// - `Local` anchor: locals (driver) first, then gists ranked on the selected local
    /// - `Gist` anchor: gists (driver) first, then locals reverse-ranked on the selected gist
    pub fn list_pane_snapshots(&self) -> (Vec<RankedLocal>, Vec<RankedGistFile>) {
        match self.anchor {
            FocusPane::Local => {
                let locals = self.rank_local_files_for(None);
                let path = locals
                    .get(self.local_index)
                    .map(|r| r.candidate.path.as_path());
                let gists = self.rank_gist_files_for(path);
                (locals, gists)
            }
            FocusPane::Gist => {
                let gists = self.rank_gist_files_for(None);
                let gist = gists.get(self.gist_index).map(|r| &r.file);
                let locals = self.rank_local_files_for(gist);
                (locals, gists)
            }
        }
    }

    pub fn selected_local(&self) -> Option<LocalCandidate> {
        self.visible_locals()
            .into_iter()
            .nth(self.local_index)
            .map(|r| r.candidate)
    }

    pub fn selected_gist(&self) -> Option<RankedGistFile> {
        self.ranked_gists().into_iter().nth(self.gist_index)
    }

    /// Both selections from one dual-pane build. Prefer over calling
    /// [`Self::selected_local`] and [`Self::selected_gist`] separately.
    pub fn selected_pair(&self) -> (Option<LocalCandidate>, Option<RankedGistFile>) {
        let (locals, gists) = self.list_pane_snapshots();
        let local = locals
            .into_iter()
            .nth(self.local_index)
            .map(|r| r.candidate);
        let gist = gists.into_iter().nth(self.gist_index);
        (local, gist)
    }
}

#[cfg(test)]
mod tests {
    use crate::tui::test_support::list_state_with_matches;
    use crate::tui::*;
    use crossterm::event::KeyCode;
    use std::path::PathBuf;

    #[test]
    fn gist_ranking_follows_anchor_not_focus() {
        let mut state = list_state_with_matches();
        state.anchor = FocusPane::Local;
        state.local_index = 0; // settings.json
        state.focus = FocusPane::Gist; // focus moved away, but anchor still Local
        let ranked = state.ranked_gists();
        assert_eq!(ranked[0].file.filename, "settings.json");
    }

    #[test]
    fn reverse_ranking_orders_locals_by_selected_gist() {
        let mut state = initial_state();
        state.anchor = FocusPane::Gist;
        state.gist_catalog.owned = vec![GistFile {
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "settings.json")
        }];
        state.locals = vec![
            LocalCandidate {
                path: PathBuf::from("other.txt"),
                pinned: false,
                modified: None,
            },
            LocalCandidate {
                path: PathBuf::from("settings.json"),
                pinned: false,
                modified: None,
            },
        ];
        // The local pane reverse-ranks against the selected gist (gist_index 0).
        let visible = state.visible_locals();
        assert_eq!(visible[0].candidate.path, PathBuf::from("settings.json"));
        assert_ne!(visible[0].mark, crate::ranking::MatchMark::None);
    }

    #[test]
    fn local_sort_name_orders_by_filename() {
        let mut state = initial_state(); // focus Local -> no reverse ranking
        state.local_sort = LocalSort::Name;
        state.locals = vec![
            LocalCandidate {
                path: PathBuf::from("zeta.txt"),
                pinned: false,
                modified: None,
            },
            LocalCandidate {
                path: PathBuf::from("alpha.txt"),
                pinned: false,
                modified: None,
            },
        ];
        assert_eq!(
            state.visible_locals()[0].candidate.path,
            PathBuf::from("alpha.txt")
        );
    }

    #[test]
    fn local_sort_recent_orders_by_mtime_desc_none_last() {
        let mut state = initial_state();
        state.local_sort = LocalSort::Recent;
        state.locals = vec![
            LocalCandidate {
                path: PathBuf::from("old"),
                pinned: false,
                modified: Some(100),
            },
            LocalCandidate {
                path: PathBuf::from("none"),
                pinned: false,
                modified: None,
            },
            LocalCandidate {
                path: PathBuf::from("new"),
                pinned: false,
                modified: Some(500),
            },
        ];
        let paths: Vec<_> = state
            .visible_locals()
            .into_iter()
            .map(|r| r.candidate.path)
            .collect();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("new"),
                PathBuf::from("old"),
                PathBuf::from("none")
            ]
        );
    }

    #[test]
    fn ranking_helpers_terminate_in_either_anchor() {
        // Regression: eagerly evaluating the cross-pane selection caused the two
        // anchor-driven rankings to recurse into each other.
        let mut state = initial_state();
        state.gist_catalog.owned = vec![GistFile {
            updated_at: "x".into(),
            created_at: "x".into(),
            ..GistFile::fixture("a", "f")
        }];
        state.locals = vec![LocalCandidate {
            path: PathBuf::from("f"),
            pinned: false,
            modified: None,
        }];
        for anchor in [FocusPane::Local, FocusPane::Gist] {
            state.anchor = anchor;
            let _ = state.ranked_gists();
            let _ = state.visible_locals();
            let _ = state.selected_local();
            let _ = state.selected_gist();
        }
    }

    #[test]
    fn sort_by_name_and_recent_reorders_gists() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![
            GistFile {
                public: true,
                updated_at: "2026-01-01T00:00:00Z".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                ..GistFile::fixture("z", "zeta.json")
            },
            GistFile {
                public: true,
                updated_at: "2026-09-09T00:00:00Z".into(),
                created_at: "2026-09-09T00:00:00Z".into(),
                ..GistFile::fixture("a", "alpha.json")
            },
        ];
        // No local selected -> Match keeps gh list order (zeta, alpha).
        assert_eq!(state.ranked_gists()[0].file.filename, "zeta.json");

        state.gist_sort = GistSort::Name;
        assert_eq!(state.ranked_gists()[0].file.filename, "alpha.json");

        state.gist_sort = GistSort::Recent;
        assert_eq!(state.ranked_gists()[0].file.filename, "alpha.json");
        assert_eq!(state.ranked_gists()[1].file.filename, "zeta.json");
    }

    #[test]
    fn gist_type_filter_limits_ranked_gists() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![
            GistFile {
                description: "p".into(),
                public: true,
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("pub", "a.json")
            },
            GistFile {
                description: "s".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("sec", "b.json")
            },
        ];
        assert_eq!(state.ranked_gists().len(), 2);

        state.gist_type_filter = GistTypeFilter::Public;
        let only_public = state.ranked_gists();
        assert_eq!(only_public.len(), 1);
        assert_eq!(only_public[0].file.gist_id, "pub");

        state.gist_type_filter = GistTypeFilter::Secret;
        let only_secret = state.ranked_gists();
        assert_eq!(only_secret.len(), 1);
        assert_eq!(only_secret[0].file.gist_id, "sec");
    }

    #[test]
    fn empty_state_has_no_ranked_gists() {
        let state = initial_state();
        assert!(state.ranked_gists().is_empty());
    }

    #[test]
    fn no_local_selected_lists_all_gists_unranked() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![
            GistFile {
                description: "first".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("a", "alpha.json")
            },
            GistFile {
                description: "second".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("b", "beta.json")
            },
        ];
        let ranked = state.ranked_gists();
        assert_eq!(ranked.len(), 2);
        // Order preserved (unranked) and no scoring applied.
        assert_eq!(ranked[0].file.filename, "alpha.json");
        assert_eq!(ranked[0].mark, crate::ranking::MatchMark::None);
    }

    #[test]
    fn local_selection_changes_ranked_gists() {
        let mut state = initial_state();
        state.locals = vec![
            LocalCandidate {
                path: PathBuf::from("/tmp/settings.json"),
                pinned: false,
                modified: None,
            },
            LocalCandidate {
                path: PathBuf::from("/tmp/statusline.sh"),
                pinned: false,
                modified: None,
            },
        ];
        state.gist_catalog.owned = vec![
            GistFile {
                description: "settings".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("a", "settings.json")
            },
            GistFile {
                description: "status".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("b", "statusline.sh")
            },
        ];

        assert_eq!(state.ranked_gists()[0].file.filename, "settings.json");
        state.handle_key(KeyCode::Down);
        assert_eq!(state.ranked_gists()[0].file.filename, "statusline.sh");
    }

    /// Public `ranked_gists` / `visible_locals` / `selected_*` stay pure recomputes (no
    /// content-hash / epoch memo — #154 closed that approach). Hot paths use
    /// `list_pane_snapshots()` (#224 shape #1) which builds each list once without caching
    /// across mutations. `selected_gist` / `selected_local` must still equal `list[index]`
    /// after an earlier read and an input mutation — a future silent cache would break here.
    #[test]
    fn selected_accessors_track_recomputed_lists_with_no_cache() {
        let mut state = initial_state();
        state.locals = vec![
            LocalCandidate {
                path: PathBuf::from("/tmp/settings.json"),
                pinned: false,
                modified: None,
            },
            LocalCandidate {
                path: PathBuf::from("/tmp/statusline.sh"),
                pinned: false,
                modified: None,
            },
        ];
        state.gist_catalog.owned = vec![
            GistFile {
                description: "settings".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("a", "settings.json")
            },
            GistFile {
                description: "status".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("b", "statusline.sh")
            },
        ];

        // Read both lists first — this would warm any hypothetical cache.
        let _ = state.ranked_gists();
        let _ = state.visible_locals();
        // Accessors equal a fresh recompute at the current indices.
        assert_eq!(
            state.selected_gist().map(|g| g.file.filename),
            state
                .ranked_gists()
                .into_iter()
                .nth(state.gist_index)
                .map(|g| g.file.filename),
        );
        assert_eq!(
            state.selected_local().map(|l| l.path),
            state
                .visible_locals()
                .into_iter()
                .nth(state.local_index)
                .map(|r| r.candidate.path),
        );
        assert_eq!(state.ranked_gists()[0].file.filename, "settings.json");

        // Move the local selection: ranking must reflect the *new* state, not the earlier read.
        state.handle_key(KeyCode::Down);
        assert_eq!(state.ranked_gists()[0].file.filename, "statusline.sh");
        // The accessors still match a fresh recompute after the mutation.
        assert_eq!(
            state.selected_gist().map(|g| g.file.filename),
            state
                .ranked_gists()
                .into_iter()
                .nth(state.gist_index)
                .map(|g| g.file.filename),
        );
        assert_eq!(
            state.selected_local().map(|l| l.path),
            state
                .visible_locals()
                .into_iter()
                .nth(state.local_index)
                .map(|r| r.candidate.path),
        );
    }

    #[test]
    fn list_pane_snapshots_match_public_accessors() {
        let mut state = initial_state();
        state.locals = vec![
            LocalCandidate {
                path: PathBuf::from("/tmp/settings.json"),
                pinned: false,
                modified: None,
            },
            LocalCandidate {
                path: PathBuf::from("/tmp/statusline.sh"),
                pinned: false,
                modified: None,
            },
        ];
        state.gist_catalog.owned = vec![
            GistFile {
                description: "settings".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("a", "settings.json")
            },
            GistFile {
                description: "status".into(),
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("b", "statusline.sh")
            },
        ];

        for anchor in [FocusPane::Local, FocusPane::Gist] {
            state.anchor = anchor;
            let (locals, gists) = state.list_pane_snapshots();
            assert_eq!(
                locals
                    .iter()
                    .map(|r| r.candidate.path.clone())
                    .collect::<Vec<_>>(),
                state
                    .visible_locals()
                    .into_iter()
                    .map(|r| r.candidate.path)
                    .collect::<Vec<_>>(),
                "locals mismatch for {anchor:?}"
            );
            assert_eq!(
                gists
                    .iter()
                    .map(|g| g.file.filename.clone())
                    .collect::<Vec<_>>(),
                state
                    .ranked_gists()
                    .into_iter()
                    .map(|g| g.file.filename)
                    .collect::<Vec<_>>(),
                "gists mismatch for {anchor:?}"
            );
        }
    }

    #[test]
    fn forked_filter_shows_only_forks() {
        let mut state = initial_state();
        state.gist_catalog.owned = vec![
            GistFile {
                description: "mine".into(),
                public: true,
                updated_at: "x".into(),
                created_at: "x".into(),
                owner_login: "me".into(),
                ..GistFile::fixture("owned", "a.txt")
            },
            GistFile {
                description: "fork".into(),
                public: true,
                updated_at: "x".into(),
                created_at: "x".into(),
                owner_login: "me".into(),
                fork_of_id: Some("upstream".into()),
                ..GistFile::fixture("forked", "b.txt")
            },
        ];
        state.gist_catalog.user_login = Some("me".into());
        state.gist_type_filter = GistTypeFilter::Forked;
        let ids: Vec<_> = state
            .ranked_gists()
            .into_iter()
            .map(|g| g.file.gist_id)
            .collect();
        assert_eq!(ids, vec!["forked"]);
    }

    #[test]
    fn starred_filter_lists_only_starred_gists() {
        // With the Starred type filter active, ranked_gists must draw from starred_gists, not the
        // owned list — exercises the owned/starred source switch with data on both sides.
        let mut state = initial_state();
        state.gist_catalog.owned = vec![GistFile {
            description: "mine".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: "me".into(),
            ..GistFile::fixture("owned", "a.txt")
        }];
        state.gist_catalog.starred = vec![GistFile {
            description: "theirs".into(),
            public: true,
            updated_at: "x".into(),
            created_at: "x".into(),
            owner_login: "other".into(),
            ..GistFile::fixture("starred", "b.txt")
        }];
        state.gist_type_filter = GistTypeFilter::Starred;

        let ranked = state.ranked_gists();
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].file.gist_id, "starred");
    }
}
