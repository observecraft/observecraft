//! A short-lived per-key cache with request coalescing: callers that ask for a key while it is
//! being loaded wait for that one load instead of starting their own. A failed load is not kept.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::OnceCell;

pub struct TtlCache<T> {
    ttl: Duration,
    slots: Mutex<HashMap<String, Slot<T>>>,
}

/// One load of a key: started at `at`, shared by every caller until it expires.
struct Slot<T> {
    at: Instant,
    cell: Arc<OnceCell<Arc<T>>>,
}

impl<T> TtlCache<T> {
    pub fn new(ttl: Duration) -> Self {
        TtlCache { ttl, slots: Mutex::new(HashMap::new()) }
    }

    /// The value of `key`, loaded by `load` unless a load started less than the TTL ago.
    pub async fn get_or_load<E, F, Fut>(&self, key: &str, load: F) -> Result<Arc<T>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let cell = {
            let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
            let ttl = self.ttl;
            slots.retain(|_, s| s.at.elapsed() < ttl);
            Arc::clone(&slots.entry(key.to_string()).or_insert_with(|| Slot { at: Instant::now(), cell: Arc::new(OnceCell::new()) }).cell)
        };
        cell.get_or_try_init(|| async { load().await.map(Arc::new) }).await.cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
    async fn a_failed_load_is_retried() {
        let cache: TtlCache<u32> = TtlCache::new(Duration::from_secs(15));
        assert_eq!(cache.get_or_load("acme", || async { Err::<u32, _>("clickhouse down") }).await.unwrap_err(), "clickhouse down");
        assert_eq!(*cache.get_or_load("acme", || async { Ok::<_, &str>(7) }).await.unwrap(), 7);
    }
}
