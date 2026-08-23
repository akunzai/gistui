//! Whole-list Gist refresh: base catalog fetch, progressive enrichment, and supersession.

use crate::actions::SystemRunner;
use crate::domain::{GistCatalog, GistFile};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver};

struct ListLeg {
    files: Vec<GistFile>,
    comments: HashMap<String, u32>,
    raw: String,
}

struct StarredLeg {
    list: ListLeg,
    ids: HashSet<String>,
}

struct BaseResult {
    generation: u64,
    owned: Result<ListLeg, ()>,
    starred: Result<StarredLeg, ()>,
    user_login: Result<String, ()>,
    gh_unavailable: bool,
}

type CountRx = Option<Receiver<(u64, crate::gh::CountCollection)>>;
type ForkMetaResult = Result<HashMap<String, Option<String>>, String>;
type ForkMetaMessage = (u64, ForkMetaResult);
type ForkMetaRx = Option<Receiver<ForkMetaMessage>>;

#[derive(Debug)]
pub(super) struct GistRefreshUpdate {
    pub catalog: GistCatalog,
    pub base_ready: bool,
    pub persist: bool,
    pub status: Option<String>,
}

pub(super) struct GistRefresh {
    generation: u64,
    catalog: GistCatalog,
    base: Option<Receiver<BaseResult>>,
    fork_counts: CountRx,
    star_counts: CountRx,
    fork_meta: ForkMetaRx,
    remaining_enrichments: u8,
    failures: Vec<&'static str>,
}

impl GistRefresh {
    pub fn new(catalog: &GistCatalog, start: bool) -> Self {
        let mut refresh = Self {
            generation: 0,
            catalog: catalog.clone(),
            base: None,
            fork_counts: None,
            star_counts: None,
            fork_meta: None,
            remaining_enrichments: 0,
            failures: Vec::new(),
        };
        if start {
            refresh.start(catalog);
        }
        refresh
    }

    pub fn start(&mut self, catalog: &GistCatalog) {
        self.generation = self.generation.wrapping_add(1);
        self.catalog = catalog.clone();
        self.base = Some(spawn_base_fetch(self.generation));
        self.fork_counts = None;
        self.star_counts = None;
        self.fork_meta = None;
        self.remaining_enrichments = 0;
        self.failures.clear();
    }

    pub fn poll(&mut self) -> Vec<GistRefreshUpdate> {
        let mut updates = Vec::new();
        if let Some(result) = poll(&mut self.base) {
            if result.generation == self.generation {
                self.apply_base(result, &mut updates);
            }
        }
        self.poll_fork_counts(&mut updates);
        self.poll_star_counts(&mut updates);
        self.poll_fork_meta(&mut updates);
        updates
    }

    fn apply_base(&mut self, result: BaseResult, updates: &mut Vec<GistRefreshUpdate>) {
        let old_fork_of: HashMap<_, _> = self
            .catalog
            .owned
            .iter()
            .map(|gist| (gist.gist_id.clone(), gist.fork_of_id.clone()))
            .collect();
        let mut owned_raw = None;
        let mut starred_raw = None;
        let owned_ok = match result.owned {
            Ok(mut leg) => {
                for gist in &mut leg.files {
                    if let Some(fork_of) = old_fork_of.get(&gist.gist_id) {
                        gist.fork_of_id = fork_of.clone();
                    }
                }
                self.catalog.owned = leg.files;
                owned_raw = Some(leg.raw);
                Some(leg.comments)
            }
            Err(()) => {
                self.failures.push("owned gists");
                None
            }
        };
        let starred_ok = match result.starred {
            Ok(leg) => {
                self.catalog.starred = leg.list.files;
                self.catalog.starred_ids = leg.ids;
                starred_raw = Some(leg.list.raw);
                Some(leg.list.comments)
            }
            Err(()) => {
                self.failures.push("starred gists");
                None
            }
        };
        if let (Some(mut owned), Some(starred)) = (owned_ok, starred_ok) {
            owned.extend(starred);
            self.catalog.comment_counts = owned;
        }
        match result.user_login {
            Ok(login) => self.catalog.user_login = Some(login),
            Err(()) => self.failures.push("current user"),
        }

        updates.push(self.update(true, true, None));
        if result.gh_unavailable {
            updates.push(self.finish());
            return;
        }

        let gist_ids: HashSet<_> = self
            .catalog
            .owned
            .iter()
            .chain(&self.catalog.starred)
            .map(|gist| gist.gist_id.clone())
            .collect();
        self.fork_counts = Some(spawn_count(self.generation, move || {
            crate::gh::collect_gist_fork_counts(
                &SystemRunner,
                owned_raw.as_deref(),
                starred_raw.as_deref(),
                gist_ids,
            )
        }));
        let node_ids =
            crate::gh::merge_gist_node_id_maps(&self.catalog.owned, &self.catalog.starred);
        self.star_counts = Some(spawn_count(self.generation, move || {
            crate::gh::collect_gist_star_counts(&SystemRunner, node_ids)
        }));
        let owned_ids = self
            .catalog
            .owned
            .iter()
            .map(|gist| gist.gist_id.clone())
            .collect();
        self.fork_meta = Some(spawn_fork_meta(self.generation, owned_ids));
        self.remaining_enrichments = 3;
    }

    fn poll_fork_counts(&mut self, updates: &mut Vec<GistRefreshUpdate>) {
        let Some((generation, result)) = poll(&mut self.fork_counts) else {
            return;
        };
        if generation != self.generation {
            return;
        }
        if result.incomplete {
            self.failures.push("fork counts");
        } else {
            self.catalog.fork_counts = result.counts;
            updates.push(self.update(false, true, None));
        }
        self.finish_enrichment(updates);
    }

    fn poll_star_counts(&mut self, updates: &mut Vec<GistRefreshUpdate>) {
        let Some((generation, result)) = poll(&mut self.star_counts) else {
            return;
        };
        if generation != self.generation {
            return;
        }
        if result.incomplete {
            self.failures.push("star counts");
        } else {
            self.catalog.star_counts = result.counts;
            updates.push(self.update(false, true, None));
        }
        self.finish_enrichment(updates);
    }

    fn poll_fork_meta(&mut self, updates: &mut Vec<GistRefreshUpdate>) {
        let Some((generation, result)) = poll(&mut self.fork_meta) else {
            return;
        };
        if generation != self.generation {
            return;
        }
        match result {
            Ok(fork_of) => {
                crate::gh::apply_fork_of_ids(&mut self.catalog.owned, &fork_of);
                updates.push(self.update(false, true, None));
            }
            Err(_) => self.failures.push("fork detection"),
        }
        self.finish_enrichment(updates);
    }

    fn finish_enrichment(&mut self, updates: &mut Vec<GistRefreshUpdate>) {
        self.remaining_enrichments = self.remaining_enrichments.saturating_sub(1);
        if self.remaining_enrichments == 0 && !self.failures.is_empty() {
            updates.push(self.finish());
        }
    }

    fn finish(&self) -> GistRefreshUpdate {
        self.update(
            false,
            false,
            Some(format!(
                "refresh incomplete: {} unavailable",
                self.failures.join(", ")
            )),
        )
    }

    fn update(&self, base_ready: bool, persist: bool, status: Option<String>) -> GistRefreshUpdate {
        GistRefreshUpdate {
            catalog: self.catalog.clone(),
            base_ready,
            persist,
            status,
        }
    }
}

fn spawn_base_fetch(generation: u64) -> Receiver<BaseResult> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = match crate::gh::check_gh_ready(&SystemRunner) {
            Ok(()) => {
                let (owned, starred, user_login) = std::thread::scope(|scope| {
                    let owned = scope.spawn(fetch_owned);
                    let starred = scope.spawn(fetch_starred);
                    let user = scope.spawn(|| {
                        crate::gh::fetch_current_user_login(&SystemRunner).map_err(|_| ())
                    });
                    (
                        owned.join().unwrap_or(Err(())),
                        starred.join().unwrap_or(Err(())),
                        user.join().unwrap_or(Err(())),
                    )
                });
                BaseResult {
                    generation,
                    owned,
                    starred,
                    user_login,
                    gh_unavailable: false,
                }
            }
            Err(_) => BaseResult {
                generation,
                owned: Err(()),
                starred: Err(()),
                user_login: Err(()),
                gh_unavailable: true,
            },
        };
        let _ = tx.send(result);
    });
    rx
}

fn fetch_owned() -> Result<ListLeg, ()> {
    let raw = crate::gh::fetch_gist_list_json(&SystemRunner).map_err(|_| ())?;
    Ok(ListLeg {
        files: crate::gh::parse_gist_list_json(&raw).map_err(|_| ())?,
        comments: crate::gh::parse_gist_comment_counts(&raw).map_err(|_| ())?,
        raw,
    })
}

fn fetch_starred() -> Result<StarredLeg, ()> {
    let raw = crate::gh::fetch_gist_starred_list_json(&SystemRunner).map_err(|_| ())?;
    Ok(StarredLeg {
        ids: crate::gh::parse_starred_gist_ids(&raw).map_err(|_| ())?,
        list: ListLeg {
            files: crate::gh::parse_gist_list_json(&raw).map_err(|_| ())?,
            comments: crate::gh::parse_gist_comment_counts(&raw).map_err(|_| ())?,
            raw,
        },
    })
}

fn spawn_count(
    generation: u64,
    fetch: impl FnOnce() -> crate::gh::CountCollection + Send + 'static,
) -> Receiver<(u64, crate::gh::CountCollection)> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send((generation, fetch()));
    });
    rx
}

fn spawn_fork_meta(generation: u64, owned_ids: HashSet<String>) -> Receiver<ForkMetaMessage> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = crate::gh::collect_owned_fork_of_ids(&SystemRunner, owned_ids);
        let _ = tx.send((generation, result));
    });
    rx
}

fn poll<T>(slot: &mut Option<Receiver<T>>) -> Option<T> {
    let value = slot.as_ref()?.try_recv().ok()?;
    *slot = None;
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::CountCollection;

    fn gist(id: &str) -> GistFile {
        GistFile::fixture(id, format!("{id}.txt"))
    }

    fn old_catalog() -> GistCatalog {
        GistCatalog {
            owned: vec![gist("old-owned")],
            starred: vec![gist("old-starred")],
            user_login: Some("octocat".into()),
            comment_counts: HashMap::from([("old-owned".into(), 4)]),
            fork_counts: HashMap::from([("old-owned".into(), 2)]),
            star_counts: HashMap::from([("old-owned".into(), 7)]),
            ..GistCatalog::default()
        }
    }

    #[test]
    fn partial_base_failure_retains_last_known_good_and_reports_once() {
        let mut refresh = GistRefresh::new(&old_catalog(), false);
        refresh.generation = 1;
        let (tx, rx) = mpsc::channel();
        tx.send(BaseResult {
            generation: 1,
            owned: Err(()),
            starred: Ok(StarredLeg {
                list: ListLeg {
                    files: vec![gist("new-starred")],
                    comments: HashMap::from([("new-starred".into(), 1)]),
                    raw: "[]".into(),
                },
                ids: HashSet::from(["new-starred".into()]),
            }),
            user_login: Err(()),
            gh_unavailable: true,
        })
        .unwrap();
        refresh.base = Some(rx);

        let updates = refresh.poll();

        assert_eq!(updates.len(), 2);
        assert!(updates[0].base_ready);
        assert_eq!(updates[0].catalog.owned[0].gist_id, "old-owned");
        assert_eq!(updates[0].catalog.starred[0].gist_id, "new-starred");
        assert_eq!(updates[0].catalog.user_login.as_deref(), Some("octocat"));
        assert_eq!(updates[0].catalog.comment_counts["old-owned"], 4);
        assert_eq!(
            updates[1].status.as_deref(),
            Some("refresh incomplete: owned gists, current user unavailable")
        );
    }

    #[test]
    fn stale_generation_is_ignored() {
        let catalog = old_catalog();
        let mut refresh = GistRefresh::new(&catalog, false);
        refresh.generation = 2;
        let (tx, rx) = mpsc::channel();
        tx.send(BaseResult {
            generation: 1,
            owned: Err(()),
            starred: Err(()),
            user_login: Err(()),
            gh_unavailable: true,
        })
        .unwrap();
        refresh.base = Some(rx);

        assert!(refresh.poll().is_empty());
        assert_eq!(refresh.catalog, catalog);
    }

    #[test]
    fn enrichment_publishes_one_coherent_catalog_stage() {
        let mut refresh = GistRefresh::new(&old_catalog(), false);
        refresh.generation = 1;
        refresh.remaining_enrichments = 1;
        let (tx, rx) = mpsc::channel();
        tx.send((
            1,
            CountCollection {
                counts: HashMap::from([("old-owned".into(), 9)]),
                incomplete: false,
            },
        ))
        .unwrap();
        refresh.fork_counts = Some(rx);

        let updates = refresh.poll();

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].catalog.fork_counts["old-owned"], 9);
        assert_eq!(updates[0].catalog.star_counts["old-owned"], 7);
        assert!(updates[0].persist);
        assert!(updates[0].status.is_none());
    }

    #[test]
    fn incomplete_enrichment_retains_cache_and_reports_at_end() {
        let mut refresh = GistRefresh::new(&old_catalog(), false);
        refresh.generation = 1;
        refresh.remaining_enrichments = 1;
        let (tx, rx) = mpsc::channel();
        tx.send((
            1,
            CountCollection {
                counts: HashMap::from([("old-owned".into(), 99)]),
                incomplete: true,
            },
        ))
        .unwrap();
        refresh.star_counts = Some(rx);

        let updates = refresh.poll();

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].catalog.star_counts["old-owned"], 7);
        assert!(!updates[0].persist);
        assert_eq!(
            updates[0].status.as_deref(),
            Some("refresh incomplete: star counts unavailable")
        );
    }
}
