//! Dual-pane list-pane ranking (issue #313): filters, ranks, and sorts the local/gist
//! panes. Cross-cutting rather than `Screen::List`-scoped — `bg.rs`'s background-task
//! absorb handlers call [`AppState::selected_local`] / [`AppState::ranked_gists`] directly,
//! not gated on `Screen::List` (`List` stays a unit tag; dual-pane selection/filters/sorts
//! are session-global on `AppState`; see `docs/agents/architecture.md`).
//!
//! Named `list_ranking`, not `ranking`, to avoid colliding with the top-level pure module
//! `crate::ranking` — the scoring algorithm these methods call into.

use crate::domain::{GistFile, LocalCandidate};
use crate::ranking::{rank_gist_files, rank_local_files, RankedGistFile, RankedLocal};
use crate::tui::{AppState, FocusPane};

fn unranked_gists(gists: Vec<GistFile>) -> Vec<RankedGistFile> {
    gists
        .into_iter()
        .map(|file| RankedGistFile {
            file,
            score: 0,
            reasons: Vec::new(),
        })
        .collect()
}

fn unranked_locals(locals: &[LocalCandidate]) -> Vec<RankedLocal> {
    locals
        .iter()
        .cloned()
        .map(|candidate| RankedLocal {
            candidate,
            score: 0,
            reasons: Vec::new(),
        })
        .collect()
}

impl AppState {
    /// Filtered owned/starred gist file rows (no ranking/sort). Shared by
    /// [`Self::ranked_gists`] and [`Self::list_pane_snapshots`].
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
            None => unranked_gists(gists),
        };
        self.gist_sort.apply(&mut ranked);
        ranked
    }

    /// Local rows with optional reverse-rank against a known gist file. Does **not**
    /// call `selected_gist` / `ranked_gists`.
    fn rank_local_files_for(&self, gist: Option<&GistFile>) -> Vec<RankedLocal> {
        let mut ranked = match gist {
            Some(file) => rank_local_files(file, &self.locals, &self.pinned),
            None => unranked_locals(&self.locals),
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

    pub fn ranked_gists(&self) -> Vec<RankedGistFile> {
        // Anchor-driven ranking: the gist pane is ranked against the selected local file
        // only while the LOCAL pane is the anchor (anchor == Local). When the gist pane
        // is the anchor it uses its own sort (no ranking), which also breaks the
        // otherwise-mutual dependency with `visible_locals`.
        // NOTE: only evaluate the local selection inside the anchor==Local branch.
        // Computing it eagerly would recurse: selected_local -> visible_locals ->
        // selected_gist -> ranked_gists.
        let local_path = if self.anchor == FocusPane::Local {
            self.rank_local_files_for(None)
                .get(self.local_index)
                .map(|r| r.candidate.path.clone())
        } else {
            None
        };
        self.rank_gist_files_for(local_path.as_deref())
    }

    /// The local file list after sorting (and, while the gist pane drives, reverse ranking
    /// against the selected gist). Single source of truth for the local pane's order,
    /// selection, and rendering — mirrors `ranked_gists`.
    pub fn visible_locals(&self) -> Vec<RankedLocal> {
        // Mirror of `ranked_gists`: only evaluate the gist selection in the anchor==Gist
        // branch to avoid recursing back through `ranked_gists` -> `selected_local`.
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

    /// Build both list-pane orderings with **one** construction of each list (issue #224 /
    /// shape #1 from #154). Prefer this when a key/palette/render path needs both sides or
    /// multiple selected rows — avoids N full filter+rank+sort passes per call site.
    ///
    /// Expansion order follows the anchor so the mutual recursion is not entered:
    /// - `Local` anchor: locals (driver) first, then gists ranked on the selected local
    /// - `Gist` anchor: gists (driver) first, then locals reverse-ranked on the selected gist
    ///
    /// Public `ranked_gists` / `visible_locals` / `selected_*` stay pure recomputes so unit
    /// tests keep the no-stale-cache contract; hot handlers should call this instead.
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
}
