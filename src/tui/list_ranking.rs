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
