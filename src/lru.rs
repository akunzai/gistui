//! A small capacity-bounded LRU cache. Used to cap the in-memory gist content cache so that
//! previewing many (or large) gists cannot grow memory without bound for the session's
//! lifetime. Dependency-free: the recency list is a plain `Vec`, so eviction is O(n) in the
//! capacity — fine because the capacity is small.

use std::collections::HashMap;
use std::hash::Hash;

/// Least-recently-used cache holding at most `capacity` entries. When an insert would push the
/// cache past its capacity, the least-recently-used entry is evicted. Both `get` and `insert`
/// count as a use (they mark the entry most-recently-used).
#[derive(Debug, Clone)]
pub struct LruCache<K, V> {
    map: HashMap<K, V>,
    /// Keys ordered least- to most-recently used (back = most recent).
    order: Vec<K>,
    capacity: usize,
}

impl<K: Clone + Eq + Hash, V> LruCache<K, V> {
    /// Create a cache holding at most `capacity` entries. A `capacity` of 0 is clamped to 1 so
    /// the cache always retains at least the most recent entry.
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    /// Number of entries currently held.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the cache holds no entries.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Fetch a value, marking it most-recently-used.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        if self.map.contains_key(key) {
            self.touch(key);
            self.map.get(key)
        } else {
            None
        }
    }

    /// Insert or replace a value, marking it most-recently-used and evicting the
    /// least-recently-used entry if that pushes the cache past its capacity.
    pub fn insert(&mut self, key: K, value: V) {
        if self.map.insert(key.clone(), value).is_some() {
            self.touch(&key);
        } else {
            self.order.push(key);
            if self.map.len() > self.capacity {
                let evicted = self.order.remove(0);
                self.map.remove(&evicted);
            }
        }
    }

    /// Remove a value, if present, returning it.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
        self.map.remove(key)
    }

    /// Move an existing key to the most-recently-used (back) position.
    fn touch(&mut self, key: &K) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos);
            self.order.push(k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used_on_overflow() {
        let mut cache = LruCache::new(2);
        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3); // evicts "a" (least recently used)

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&"a"), None);
        assert_eq!(cache.get(&"b"), Some(&2));
        assert_eq!(cache.get(&"c"), Some(&3));
    }

    #[test]
    fn get_marks_entry_most_recently_used() {
        let mut cache = LruCache::new(2);
        cache.insert("a", 1);
        cache.insert("b", 2);
        assert_eq!(cache.get(&"a"), Some(&1)); // "a" is now most recent, "b" is LRU
        cache.insert("c", 3); // evicts "b", not "a"

        assert_eq!(cache.get(&"a"), Some(&1));
        assert_eq!(cache.get(&"b"), None);
        assert_eq!(cache.get(&"c"), Some(&3));
    }

    #[test]
    fn reinsert_updates_value_without_growing() {
        let mut cache = LruCache::new(2);
        cache.insert("a", 1);
        cache.insert("a", 9);

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&"a"), Some(&9));
    }

    #[test]
    fn remove_drops_entry_and_frees_a_slot() {
        let mut cache = LruCache::new(2);
        cache.insert("a", 1);
        cache.insert("b", 2);
        assert_eq!(cache.remove(&"a"), Some(1));
        assert_eq!(cache.len(), 1);

        cache.insert("c", 3); // fits without evicting "b"
        assert_eq!(cache.get(&"b"), Some(&2));
        assert_eq!(cache.get(&"c"), Some(&3));
    }

    #[test]
    fn zero_capacity_is_clamped_to_one() {
        let mut cache = LruCache::new(0);
        cache.insert("a", 1);
        cache.insert("b", 2);

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&"b"), Some(&2));
    }
}
