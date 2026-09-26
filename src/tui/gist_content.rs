//! In-memory Gist content lookup, request hydration, and invalidation policy.

use crate::domain::{GistCatalog, GistFileRef};

type ContentKey = (String, String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ContentLookup {
    Hit(String),
    Miss(GistFileRef),
}

#[derive(Debug, Clone)]
pub(super) struct GistContentStore {
    cache: crate::lru::LruCache<ContentKey, String>,
}

impl GistContentStore {
    /// Cached content for `file`, or — on a miss — the fetch target to request it with.
    pub fn lookup(&mut self, catalog: &GistCatalog, file: GistFileRef) -> ContentLookup {
        match self.cache.get(&key(&file)).cloned() {
            Some(content) => ContentLookup::Hit(content),
            None => ContentLookup::Miss(Self::fetch_target(catalog, file)),
        }
    }

    /// `file` ready to fetch fresh, bypassing the cache: its `raw_url` filled in from the
    /// catalog when the caller didn't have one, so a truncated file can still be read whole.
    /// Nothing is cached until a Preview fetch succeeds (`on_preview_content`), so a failed
    /// refresh keeps the last-known-good content.
    pub fn fetch_target(catalog: &GistCatalog, mut file: GistFileRef) -> GistFileRef {
        if file.raw_url.is_none() {
            file.raw_url = catalog
                .owned
                .iter()
                .chain(&catalog.starred)
                .find(|gist| gist.gist_id == file.gist_id && gist.filename == file.filename)
                .and_then(|gist| gist.raw_url.clone());
        }
        file
    }

    pub fn insert(&mut self, file: &GistFileRef, content: String) {
        self.cache.insert(key(file), content);
    }

    pub fn invalidate_file(&mut self, file: &GistFileRef) {
        self.cache.remove(&key(file));
    }

    pub fn invalidate_gist(&mut self, gist_id: &str) {
        self.cache.remove_where(|(id, _)| id == gist_id);
    }
}

impl Default for GistContentStore {
    fn default() -> Self {
        Self {
            cache: crate::lru::LruCache::new(64),
        }
    }
}

fn key(file: &GistFileRef) -> ContentKey {
    (file.gist_id.clone(), file.filename.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::GistFile;

    fn catalog() -> GistCatalog {
        GistCatalog {
            owned: vec![GistFile {
                raw_url: Some("https://example.test/a.txt".into()),
                ..GistFile::fixture("g1", "a.txt")
            }],
            ..GistCatalog::default()
        }
    }

    #[test]
    fn lookup_hits_while_fetch_target_hydrates_the_request() {
        let file = GistFileRef::id_name("g1", "a.txt");
        let mut store = GistContentStore::default();
        store.insert(&file, "cached".into());
        assert_eq!(
            store.lookup(&catalog(), file.clone()),
            ContentLookup::Hit("cached".into())
        );
        assert_eq!(
            GistContentStore::fetch_target(&catalog(), file),
            GistFileRef::new("g1", "a.txt", Some("https://example.test/a.txt".into()))
        );
    }

    #[test]
    fn refresh_keeps_last_known_good_until_insert() {
        let file = GistFileRef::id_name("g1", "a.txt");
        let mut store = GistContentStore::default();
        store.insert(&file, "old".into());
        let _target = GistContentStore::fetch_target(&catalog(), file.clone());
        assert_eq!(
            store.lookup(&catalog(), file),
            ContentLookup::Hit("old".into())
        );
    }

    #[test]
    fn invalidation_can_target_one_file_or_a_whole_gist() {
        let a = GistFileRef::id_name("g1", "a.txt");
        let b = GistFileRef::id_name("g1", "b.txt");
        let c = GistFileRef::id_name("g2", "c.txt");
        let mut store = GistContentStore::default();
        for file in [&a, &b, &c] {
            store.insert(file, file.filename.clone());
        }
        store.invalidate_file(&a);
        assert!(matches!(
            store.lookup(&catalog(), a),
            ContentLookup::Miss(_)
        ));
        store.invalidate_gist("g1");
        assert!(matches!(
            store.lookup(&catalog(), b),
            ContentLookup::Miss(_)
        ));
        assert_eq!(
            store.lookup(&catalog(), c),
            ContentLookup::Hit("c.txt".into())
        );
    }
}
