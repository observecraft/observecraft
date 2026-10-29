//! A short-lived per-key cache with request coalescing: callers that ask for a key while it is
//! being loaded wait for that one load instead of starting their own. A failed load is not kept.
//!
//! Optionally bounded by entries and by total weight (e.g. bytes or objects, from a weigher):
//! beyond a bound the least recently used finished entries are dropped, and a value heavier than
//! the whole bound is returned but not kept.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::OnceCell;

pub struct TtlCache<T> {
    ttl: Duration,
    max_entries: usize,
    max_weight: usize,
    weigh: fn(&T) -> usize,
    slots: Mutex<Slots<T>>,
}

struct Slots<T> {
    map: HashMap<String, Slot<T>>,
    /// Use counter: the slot with the smallest `used` is the least recently used.
    tick: u64,
}

/// One load of a key: started at `at`, shared by every caller until it expires.
struct Slot<T> {
    at: Instant,
    used: u64,
    /// Set once the load finished (in-flight loads are never evicted).
    weight: Option<usize>,
    cell: Arc<OnceCell<Arc<T>>>,
}

impl<T> TtlCache<T> {
    pub fn new(ttl: Duration) -> Self {
        TtlCache { ttl, max_entries: usize::MAX, max_weight: usize::MAX, weigh: |_| 0, slots: Mutex::new(Slots { map: HashMap::new(), tick: 0 }) }
    }

    /// Keep at most `max` entries.
    pub fn with_max_entries(mut self, max: usize) -> Self {
        self.max_entries = max.max(1);
        self
    }

    /// Keep entries whose `weigh`ts add up to at most `max`.
    pub fn with_max_weight(mut self, max: usize, weigh: fn(&T) -> usize) -> Self {
        self.max_weight = max;
        self.weigh = weigh;
        self
    }

    /// The value of `key`, loaded by `load` unless a load started less than the TTL ago.
    pub async fn get_or_load<E, F, Fut>(&self, key: &str, load: F) -> Result<Arc<T>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let cell = {
            let mut s = self.lock();
            let tick = s.touch();
            let slot = s.map.entry(key.to_string()).or_insert_with(|| Slot { at: Instant::now(), used: tick, weight: None, cell: Arc::new(OnceCell::new()) });
            slot.used = tick;
            Arc::clone(&slot.cell)
        };
        let value = cell.get_or_try_init(|| async { load().await.map(Arc::new) }).await?.clone();
        let mut s = self.lock();
        if let Some(slot) = s.map.get_mut(key).filter(|slot| Arc::ptr_eq(&slot.cell, &cell) && slot.weight.is_none()) {
            slot.weight = Some((self.weigh)(&value));
            self.enforce_bounds(&mut s);
        }
        Ok(value)
    }

    /// The finished, unexpired value of `key`, if any (doesn't wait for a load in progress).
    pub fn get(&self, key: &str) -> Option<Arc<T>> {
        let mut s = self.lock();
        let tick = s.touch();
        let slot = s.map.get_mut(key)?;
        let value = slot.cell.get()?.clone();
        slot.used = tick;
        Some(value)
    }

    /// Stores `value` as the freshly loaded value of `key`.
    pub fn insert(&self, key: String, value: T) {
        let weight = (self.weigh)(&value);
        let mut s = self.lock();
        let tick = s.touch();
        s.map.insert(key, Slot { at: Instant::now(), used: tick, weight: Some(weight), cell: Arc::new(OnceCell::new_with(Some(Arc::new(value)))) });
        self.enforce_bounds(&mut s);
    }

    /// Drops every entry whose key starts with `prefix`.
    pub fn remove_prefix(&self, prefix: &str) {
        self.lock().map.retain(|k, _| !k.starts_with(prefix));
    }

    /// The slots with expired ones removed.
    fn lock(&self) -> MutexGuard<'_, Slots<T>> {
        let mut s = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let ttl = self.ttl;
        s.map.retain(|_, slot| slot.at.elapsed() < ttl);
        s
    }

    /// Drops least recently used finished entries until both bounds hold.
    fn enforce_bounds(&self, s: &mut Slots<T>) {
        if self.max_entries == usize::MAX && self.max_weight == usize::MAX {
            return;
        }
        // A value heavier than the whole bound would empty the cache: drop just that one.
        let max_weight = self.max_weight;
        s.map.retain(|_, slot| slot.weight.is_none_or(|w| w <= max_weight));
        let mut weight: usize = s.map.values().filter_map(|slot| slot.weight).sum();
        while s.map.len() > self.max_entries || weight > self.max_weight {
            let Some((key, w)) = s.map.iter().filter_map(|(k, slot)| slot.weight.map(|w| (k, slot.used, w))).min_by_key(|(_, used, _)| *used).map(|(k, _, w)| (k.clone(), w)) else {
                break;
            };
            s.map.remove(&key);
            weight -= w;
        }
    }
}

impl<T> Slots<T> {
    fn touch(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    impl<T> TtlCache<T> {
        fn len(&self) -> usize {
            self.lock().map.len()
        }

        fn weight(&self) -> usize {
            self.lock().map.values().filter_map(|s| s.weight).sum()
        }
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_load() {
        let cache = TtlCache::new(Duration::from_secs(15));
        let loads = AtomicUsize::new(0);
        let load = || async {
            loads.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok::<_, ()>(vec![1, 2, 3])
        };
        let (a, b, c) = tokio::join!(cache.get_or_load("acme", load), cache.get_or_load("acme", load), cache.get_or_load("acme", load));
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
        assert_eq!(*c.unwrap(), vec![1, 2, 3]);
        // A later caller within the TTL gets the same value without loading.
        cache.get_or_load("acme", load).await.unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn keys_are_separate_and_expire() {
        let cache = TtlCache::new(Duration::from_millis(30));
        let loads = AtomicUsize::new(0);
        let load = |v: &'static str| {
            let loads = &loads;
            move || async move {
                loads.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ()>(v)
            }
        };
        assert_eq!(*cache.get_or_load("acme", load("a")).await.unwrap(), "a");
        assert_eq!(*cache.get_or_load("globex", load("g")).await.unwrap(), "g");
        assert_eq!(loads.load(Ordering::SeqCst), 2);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(*cache.get_or_load("acme", load("a2")).await.unwrap(), "a2");
        assert_eq!(loads.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn least_recently_used_entries_go_first_beyond_the_entry_bound() {
        let cache = TtlCache::new(Duration::from_secs(15)).with_max_entries(2);
        let ok = |v: u32| move || async move { Ok::<_, ()>(v) };
        cache.get_or_load("a", ok(1)).await.unwrap();
        cache.get_or_load("b", ok(2)).await.unwrap();
        // "a" is used again, so "b" is the least recently used when "c" arrives.
        assert_eq!(cache.get("a").map(|v| *v), Some(1));
        cache.get_or_load("c", ok(3)).await.unwrap();
        assert_eq!(cache.get("a").map(|v| *v), Some(1));
        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.get("c").map(|v| *v), Some(3));
        assert_eq!(cache.len(), 2);
    }

    #[tokio::test]
    async fn the_weight_bound_evicts_by_total_weight() {
        let cache = TtlCache::new(Duration::from_secs(15)).with_max_weight(10, |v: &Vec<u8>| v.len());
        let ok = |n: usize| move || async move { Ok::<_, ()>(vec![0u8; n]) };
        cache.get_or_load("a", ok(4)).await.unwrap();
        cache.get_or_load("b", ok(4)).await.unwrap();
        assert_eq!(cache.weight(), 8);
        cache.get_or_load("c", ok(4)).await.unwrap();
        // 12 > 10: the oldest ("a") goes.
        assert_eq!(cache.get("a"), None);
        assert_eq!(cache.weight(), 8);
        // A value heavier than the whole bound is returned but not kept.
        assert_eq!(cache.get_or_load("huge", ok(11)).await.unwrap().len(), 11);
        assert_eq!(cache.get("huge"), None);
        assert!(cache.get("b").is_some() && cache.get("c").is_some());
    }

    #[test]
    fn inserted_values_are_read_back_until_they_expire_or_are_removed_by_prefix() {
        let cache = TtlCache::new(Duration::from_millis(30));
        cache.insert("org-a\u{1f}u1".into(), 1u32);
        cache.insert("org-ab\u{1f}u1".into(), 2u32);
        assert_eq!(cache.get("org-a\u{1f}u1").map(|v| *v), Some(1));
        cache.remove_prefix("org-a\u{1f}");
        assert_eq!(cache.get("org-a\u{1f}u1"), None);
        assert_eq!(cache.get("org-ab\u{1f}u1").map(|v| *v), Some(2));
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(cache.get("org-ab\u{1f}u1"), None);
    }

    #[tokio::test]
    async fn get_does_not_return_a_load_in_progress() {
        let cache = Arc::new(TtlCache::new(Duration::from_secs(15)));
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let loader = {
            let cache = cache.clone();
            tokio::spawn(async move {
                cache
                    .get_or_load("a", || async move {
                        let _ = rx.await;
                        Ok::<_, ()>(5u32)
                    })
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert_eq!(cache.get("a"), None);
        tx.send(()).unwrap();
        assert_eq!(*loader.await.unwrap().unwrap(), 5);
        assert_eq!(cache.get("a").map(|v| *v), Some(5));
    }

    #[tokio::test]
    async fn a_failed_load_is_retried() {
        let cache: TtlCache<u32> = TtlCache::new(Duration::from_secs(15));
        assert_eq!(cache.get_or_load("acme", || async { Err::<u32, _>("clickhouse down") }).await.unwrap_err(), "clickhouse down");
        assert_eq!(*cache.get_or_load("acme", || async { Ok::<_, &str>(7) }).await.unwrap(), 7);
    }
}
