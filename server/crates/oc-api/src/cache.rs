//! The shared cache: results that every `observecraft-server` replica reuses, kept in Valkey
//! (`OC_CACHE_URL`, required). See ADR-0002, "Shared cache: Valkey, required".
//!
//! - Keys are `oc:{namespace}:{org}:{generation}:{hash of the caller's key}`: an org is part of
//!   every key, so tenants never read each other's entries, and no query text reaches Valkey.
//! - Values are MessagePack, zstd-compressed (level 1). Above `max_value_bytes` (compressed) a value
//!   is returned but not stored; a small marker tells other replicas to load it themselves.
//! - Coalescing: concurrent callers on one replica share one load; across replicas the first to
//!   miss takes a lock (`SET …:lock NX PX <load timeout>`) and loads, the others poll for the value
//!   with backoff and load themselves when the holder fails, dies (its lock expires) or exceeds the
//!   load timeout.
//! - `invalidate(namespace, org)` replaces the namespace's generation for the org: older entries are
//!   no longer reachable and expire on their own (no SCAN/KEYS). A lost generation key (eviction)
//!   is replaced by a new random one, so old entries never come back.
//! - When Valkey fails at runtime, requests are served from the source; Valkey is skipped for a
//!   second after an error and warnings are rate-limited.
//! - Counters (`oc_cache_*`) are served in Prometheus text format on the internal listener
//!   (`GET /metrics`, see `logproc.rs`).

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use axum::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;

#[derive(Clone, Debug)]
pub struct CacheOptions {
    /// Largest value stored, in bytes after compression (`OC_CACHE_MAX_VALUE_BYTES`).
    pub max_value_bytes: usize,
    /// How long a lock holder may load before another replica loads too
    /// (`OC_CACHE_LOAD_TIMEOUT_SECONDS`).
    pub load_timeout: Duration,
}

#[derive(Debug)]
pub struct BackendError(pub String);

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<redis::RedisError> for BackendError {
    fn from(e: redis::RedisError) -> Self {
        BackendError(e.to_string())
    }
}

/// The few commands the cache needs (Valkey in production, an in-memory fake in tests).
#[async_trait]
pub trait Backend: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, BackendError>;
    async fn set(&self, key: &str, value: &[u8], ttl: Duration) -> Result<(), BackendError>;
    /// `SET key value NX PX ttl`: true when the key was absent and is now set.
    async fn set_nx(&self, key: &str, value: &[u8], ttl: Duration) -> Result<bool, BackendError>;
    /// Deletes `key` only while it still holds `value` (a lock we own).
    async fn del_if(&self, key: &str, value: &[u8]) -> Result<(), BackendError>;
}

struct Valkey {
    conn: redis::aio::ConnectionManager,
    del_if: redis::Script,
}

#[async_trait]
impl Backend for Valkey {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, BackendError> {
        Ok(redis::cmd("GET").arg(key).query_async(&mut self.conn.clone()).await?)
    }
    async fn set(&self, key: &str, value: &[u8], ttl: Duration) -> Result<(), BackendError> {
        Ok(redis::cmd("SET").arg(key).arg(value).arg("PX").arg(millis(ttl)).query_async(&mut self.conn.clone()).await?)
    }
    async fn set_nx(&self, key: &str, value: &[u8], ttl: Duration) -> Result<bool, BackendError> {
        let set: Option<String> = redis::cmd("SET").arg(key).arg(value).arg("NX").arg("PX").arg(millis(ttl)).query_async(&mut self.conn.clone()).await?;
        Ok(set.is_some())
    }
    async fn del_if(&self, key: &str, value: &[u8]) -> Result<(), BackendError> {
        let _: i64 = self.del_if.key(key).arg(value).invoke_async(&mut self.conn.clone()).await?;
        Ok(())
    }
}

fn millis(d: Duration) -> u64 {
    (d.as_millis() as u64).max(1)
}

/// How long a namespace's generation key lives (refreshed whenever it is replaced).
const GENERATION_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// After a Valkey error, requests skip it for this long instead of waiting on it each time.
const BYPASS_AFTER_ERROR: Duration = Duration::from_secs(1);
const WARN_EVERY: Duration = Duration::from_secs(30);
/// Valkey replies slower than this count as errors (the request is then served from the source).
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(1);
const POLL_FIRST: Duration = Duration::from_millis(5);
const POLL_MAX: Duration = Duration::from_millis(100);

/// First byte of a stored value: a MessagePack + zstd value, or the "too large to store" marker.
const VALUE: u8 = 1;
const TOO_LARGE: u8 = 2;
const ZSTD_LEVEL: i32 = 1;

/// Upper bounds of the `oc_cache_value_bytes` histogram (bytes after compression).
const SIZE_BUCKETS: [u64; 6] = [1 << 10, 16 << 10, 256 << 10, 1 << 20, 4 << 20, 8 << 20];

pub struct SharedCache {
    backend: Arc<dyn Backend>,
    opts: CacheOptions,
    /// Loads in progress on this replica, by logical key: `OnceCell<Arc<T>>` behind `Any`.
    inflight: Mutex<HashMap<String, Arc<dyn Any + Send + Sync>>>,
    stats: Mutex<BTreeMap<&'static str, Arc<Counters>>>,
    /// Unix ms until which Valkey is skipped, and of the last warning.
    bypass_until: AtomicU64,
    last_warn: AtomicU64,
}

#[derive(Default)]
struct Counters {
    hits: AtomicU64,
    misses: AtomicU64,
    loads: AtomicU64,
    coalesced_local: AtomicU64,
    coalesced_remote: AtomicU64,
    lock_timeouts: AtomicU64,
    oversized: AtomicU64,
    errors: AtomicU64,
    /// Values per size bucket (the last one: larger than every bound), and their total size.
    sizes: [AtomicU64; SIZE_BUCKETS.len() + 1],
    size_sum: AtomicU64,
}

impl Counters {
    fn add(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn record_size(&self, bytes: usize) {
        let i = SIZE_BUCKETS.iter().position(|b| bytes as u64 <= *b).unwrap_or(SIZE_BUCKETS.len());
        Self::add(&self.sizes[i]);
        self.size_sum.fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

/// What a lookup in Valkey decided.
enum Plan<T> {
    Hit(T),
    /// Load and store under `entry`; release the lock if we hold it.
    Load {
        entry: String,
        lock: Option<Vec<u8>>,
    },
    /// Load without storing (Valkey unavailable, or the value is too large to store).
    Direct,
}

/// A load lock this replica holds. Released after the load or, when the request is cancelled
/// mid-load (its future dropped), in the background, so other replicas don't wait out its TTL.
struct HeldLock {
    backend: Arc<dyn Backend>,
    key: String,
    token: Vec<u8>,
    released: bool,
}

impl HeldLock {
    async fn release(mut self) -> Result<(), BackendError> {
        self.released = true;
        self.backend.del_if(&self.key, &self.token).await
    }
}

impl Drop for HeldLock {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let (backend, key, token) = (self.backend.clone(), std::mem::take(&mut self.key), std::mem::take(&mut self.token));
            runtime.spawn(async move {
                let _ = backend.del_if(&key, &token).await;
            });
        }
    }
}

enum Waited<T> {
    Value(T),
    TooLarge,
    Acquired,
    TimedOut,
}

impl SharedCache {
    /// Connects to Valkey at `url` (`redis://` or `rediss://`), retrying for up to `wait`.
    pub async fn connect(url: &str, opts: CacheOptions, wait: Duration) -> anyhow::Result<Self> {
        let shown = redacted(url);
        if !(url.starts_with("redis://") || url.starts_with("rediss://")) {
            anyhow::bail!("OC_CACHE_URL must be a redis:// or rediss:// URL of the shared cache (Valkey), got {shown}");
        }
        let client = redis::Client::open(url).with_context(|| format!("OC_CACHE_URL ({shown}) is not a valid redis:// or rediss:// URL"))?;
        let config = redis::aio::ConnectionManagerConfig::new()
            .set_connection_timeout(Some(Duration::from_secs(2)))
            .set_response_timeout(Some(RESPONSE_TIMEOUT))
            .set_number_of_retries(2)
            .set_max_delay(Duration::from_secs(2));
        let started = Instant::now();
        let conn = loop {
            let attempt = async {
                let mut conn = redis::aio::ConnectionManager::new_with_config(client.clone(), config.clone()).await?;
                redis::cmd("PING").query_async::<String>(&mut conn).await?;
                Ok::<_, redis::RedisError>(conn)
            };
            match attempt.await {
                Ok(conn) => break conn,
                Err(e) if started.elapsed() >= wait => {
                    anyhow::bail!("cannot reach the shared cache (Valkey) at {shown} (OC_CACHE_URL) after {:?}: {e}", started.elapsed());
                }
                Err(e) => {
                    tracing::warn!(cache = %shown, error = %e, "shared cache (Valkey) not reachable yet, retrying");
                    tokio::time::sleep(Duration::from_secs(1).min(wait.saturating_sub(started.elapsed()))).await;
                }
            }
        };
        tracing::info!(cache = %shown, "connected to the shared cache");
        let del_if = redis::Script::new("if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end");
        Ok(Self::with_backend(Arc::new(Valkey { conn, del_if }), opts))
    }

    pub fn with_backend(backend: Arc<dyn Backend>, opts: CacheOptions) -> Self {
        SharedCache { backend, opts, inflight: Mutex::default(), stats: Mutex::default(), bypass_until: AtomicU64::new(0), last_warn: AtomicU64::new(0) }
    }

    /// The cached value of (`namespace`, `org`, `key`), or the result of `load`, stored for `ttl`.
    /// Valkey problems never fail the call: the value is then loaded from the source.
    #[allow(dead_code)] // First call sites: dashboard widgets and tag values (after ADR-0002 Stages 2–4).
    pub async fn get_or_load<T, E, F, Fut>(&self, namespace: &'static str, org: &str, key: &str, ttl: Duration, load: F) -> Result<Arc<T>, E>
    where
        T: Serialize + DeserializeOwned + Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let stats = self.stats(namespace);
        let local = format!("{namespace}\u{1f}{org}\u{1f}{key}");
        let shared: Arc<dyn Any + Send + Sync> = {
            let mut inflight = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
            inflight.entry(local.clone()).or_insert_with(|| Arc::new(OnceCell::<Arc<T>>::new())).clone()
        };
        let Ok(cell) = Arc::downcast::<OnceCell<Arc<T>>>(shared.clone()) else {
            // Another type under the same key (a programming error): don't share.
            return self.fetch(namespace, org, key, ttl, &stats, load).await.map(Arc::new);
        };
        let ran = AtomicBool::new(false);
        let result = cell
            .get_or_try_init(|| async {
                ran.store(true, Ordering::Relaxed);
                self.fetch(namespace, org, key, ttl, &stats, load).await.map(Arc::new)
            })
            .await
            .cloned();
        if ran.load(Ordering::Relaxed) {
            let mut inflight = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
            if inflight.get(&local).is_some_and(|c| Arc::ptr_eq(c, &shared)) {
                inflight.remove(&local);
            }
        } else if result.is_ok() {
            Counters::add(&stats.coalesced_local);
        }
        result
    }

    /// Drops every entry of `namespace` for `org`, on every replica.
    #[allow(dead_code)] // First call sites: dashboard edits (after ADR-0002 Stage 4).
    pub async fn invalidate(&self, namespace: &'static str, org: &str) {
        let stats = self.stats(namespace);
        if let Err(e) = self.backend.set(&generation_key(namespace, org), new_token().as_bytes(), GENERATION_TTL).await {
            self.degraded(namespace, &stats, &e);
        }
    }

    /// Prometheus text of the `oc_cache_*` counters.
    pub fn render_metrics(&self) -> String {
        let stats: Vec<(&'static str, Arc<Counters>)> = self.stats.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(k, v)| (*k, v.clone())).collect();
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let mut out = String::new();
        let mut counter = |name: &str, help: &str, rows: &mut dyn Iterator<Item = (String, u64)>| {
            let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} counter");
            for (labels, v) in rows {
                let _ = writeln!(out, "{name}{{{labels}}} {v}");
            }
        };
        counter(
            "oc_cache_lookups_total",
            "Shared cache lookups by result.",
            &mut stats.iter().flat_map(|(ns, c)| [(format!("namespace=\"{ns}\",result=\"hit\""), get(&c.hits)), (format!("namespace=\"{ns}\",result=\"miss\""), get(&c.misses))]),
        );
        counter("oc_cache_loads_total", "Values loaded from the source.", &mut stats.iter().map(|(ns, c)| (format!("namespace=\"{ns}\""), get(&c.loads))));
        counter(
            "oc_cache_coalesced_total",
            "Callers served by another caller's load, on this replica or another one.",
            &mut stats
                .iter()
                .flat_map(|(ns, c)| [(format!("namespace=\"{ns}\",scope=\"replica\""), get(&c.coalesced_local)), (format!("namespace=\"{ns}\",scope=\"cluster\""), get(&c.coalesced_remote))]),
        );
        counter("oc_cache_lock_timeouts_total", "Waits for another replica's load that ran out.", &mut stats.iter().map(|(ns, c)| (format!("namespace=\"{ns}\""), get(&c.lock_timeouts))));
        counter("oc_cache_oversized_total", "Values too large to store.", &mut stats.iter().map(|(ns, c)| (format!("namespace=\"{ns}\""), get(&c.oversized))));
        counter("oc_cache_errors_total", "Valkey errors (requests were served from the source).", &mut stats.iter().map(|(ns, c)| (format!("namespace=\"{ns}\""), get(&c.errors))));
        let _ = writeln!(out, "# HELP oc_cache_value_bytes Size of loaded values after compression.\n# TYPE oc_cache_value_bytes histogram");
        for (ns, c) in &stats {
            let mut cumulative = 0;
            for (i, bound) in SIZE_BUCKETS.iter().map(|b| b.to_string()).chain(["+Inf".to_string()]).enumerate() {
                cumulative += get(&c.sizes[i]);
                let _ = writeln!(out, "oc_cache_value_bytes_bucket{{namespace=\"{ns}\",le=\"{bound}\"}} {cumulative}");
            }
            let _ = writeln!(out, "oc_cache_value_bytes_sum{{namespace=\"{ns}\"}} {}", get(&c.size_sum));
            let _ = writeln!(out, "oc_cache_value_bytes_count{{namespace=\"{ns}\"}} {cumulative}");
        }
        out
    }

    async fn fetch<T, E, F, Fut>(&self, namespace: &'static str, org: &str, key: &str, ttl: Duration, stats: &Counters, load: F) -> Result<T, E>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let plan = if self.bypassed() {
            Plan::Direct
        } else {
            self.lookup(namespace, org, key, stats).await.unwrap_or_else(|e| {
                self.degraded(namespace, stats, &e);
                Plan::Direct
            })
        };
        let (entry, lock) = match plan {
            Plan::Hit(v) => return Ok(v),
            Plan::Direct => (None, None),
            Plan::Load { entry, lock } => {
                let lock = lock.map(|token| HeldLock { backend: self.backend.clone(), key: format!("{entry}:lock"), token, released: false });
                (Some(entry), lock)
            }
        };
        Counters::add(&stats.loads);
        let loaded = load().await;
        if let Some(entry) = &entry {
            let stored = match &loaded {
                Ok(value) => self.store(namespace, entry, value, ttl, stats).await,
                Err(_) => Ok(()),
            };
            let released = match lock {
                Some(lock) => lock.release().await,
                None => Ok(()),
            };
            if let Err(e) = stored.and(released) {
                self.degraded(namespace, stats, &e);
            }
        }
        loaded
    }

    async fn lookup<T: DeserializeOwned>(&self, namespace: &'static str, org: &str, key: &str, stats: &Counters) -> Result<Plan<T>, BackendError> {
        let generation = self.generation(namespace, org).await?;
        let entry = entry_key(namespace, org, &generation, key);
        if let Some(bytes) = self.backend.get(&entry).await? {
            match decode(&bytes) {
                Ok(Some(v)) => {
                    Counters::add(&stats.hits);
                    return Ok(Plan::Hit(v));
                }
                Ok(None) => {
                    Counters::add(&stats.misses);
                    return Ok(Plan::Direct);
                }
                Err(e) => tracing::debug!(namespace, error = %e, "unreadable shared cache entry, reloading"),
            }
        }
        Counters::add(&stats.misses);
        let lock = format!("{entry}:lock");
        let token = new_token().into_bytes();
        if self.backend.set_nx(&lock, &token, self.opts.load_timeout).await? {
            return Ok(Plan::Load { entry, lock: Some(token) });
        }
        Ok(match self.wait_for_holder(&entry, &lock, &token).await? {
            Waited::Value(v) => {
                Counters::add(&stats.coalesced_remote);
                Plan::Hit(v)
            }
            Waited::TooLarge => Plan::Direct,
            Waited::Acquired => Plan::Load { entry, lock: Some(token) },
            Waited::TimedOut => {
                Counters::add(&stats.lock_timeouts);
                Plan::Load { entry, lock: None }
            }
        })
    }

    /// Polls for the value another replica is loading, taking the lock over if it disappears.
    async fn wait_for_holder<T: DeserializeOwned>(&self, entry: &str, lock: &str, token: &[u8]) -> Result<Waited<T>, BackendError> {
        let started = Instant::now();
        let mut delay = POLL_FIRST;
        loop {
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(POLL_MAX);
            if let Some(bytes) = self.backend.get(entry).await? {
                return Ok(match decode(&bytes) {
                    Ok(Some(v)) => Waited::Value(v),
                    Ok(None) | Err(_) => Waited::TooLarge,
                });
            }
            if self.backend.set_nx(lock, token, self.opts.load_timeout).await? {
                return Ok(Waited::Acquired);
            }
            if started.elapsed() >= self.opts.load_timeout {
                return Ok(Waited::TimedOut);
            }
        }
    }

    async fn store<T: Serialize>(&self, namespace: &'static str, entry: &str, value: &T, ttl: Duration, stats: &Counters) -> Result<(), BackendError> {
        let bytes = match encode(value) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(namespace, error = %e, "value can't be encoded for the shared cache");
                return Ok(());
            }
        };
        stats.record_size(bytes.len());
        if bytes.len() > self.opts.max_value_bytes {
            Counters::add(&stats.oversized);
            tracing::debug!(namespace, bytes = bytes.len(), max = self.opts.max_value_bytes, "value too large for the shared cache, not stored");
            return self.backend.set(entry, &[TOO_LARGE], ttl).await;
        }
        self.backend.set(entry, &bytes, ttl).await
    }

    /// The org's current generation of `namespace`, created when missing.
    async fn generation(&self, namespace: &'static str, org: &str) -> Result<String, BackendError> {
        let key = generation_key(namespace, org);
        if let Some(g) = self.backend.get(&key).await? {
            return Ok(String::from_utf8_lossy(&g).into_owned());
        }
        let fresh = new_token();
        if self.backend.set_nx(&key, fresh.as_bytes(), GENERATION_TTL).await? {
            return Ok(fresh);
        }
        Ok(self.backend.get(&key).await?.map(|g| String::from_utf8_lossy(&g).into_owned()).unwrap_or(fresh))
    }

    fn stats(&self, namespace: &'static str) -> Arc<Counters> {
        debug_assert!(!namespace.contains(':') && !namespace.contains('"'), "cache namespace {namespace:?}");
        self.stats.lock().unwrap_or_else(|e| e.into_inner()).entry(namespace).or_default().clone()
    }

    fn bypassed(&self) -> bool {
        now_ms() < self.bypass_until.load(Ordering::Relaxed)
    }

    fn degraded(&self, namespace: &'static str, stats: &Counters, e: &BackendError) {
        Counters::add(&stats.errors);
        let now = now_ms();
        self.bypass_until.store(now + BYPASS_AFTER_ERROR.as_millis() as u64, Ordering::Relaxed);
        let last = self.last_warn.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= WARN_EVERY.as_millis() as u64 && self.last_warn.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            tracing::warn!(namespace, error = %e, "shared cache (Valkey) unavailable: serving from the source (logged at most every 30 s)");
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn new_token() -> String {
    format!("{:016x}", rand::random::<u64>())
}

/// `org` as one key segment: characters outside `[A-Za-z0-9._-]` are percent-encoded, so an org
/// id can't contain `:` and reach into another org's keys.
fn org_segment(org: &str) -> String {
    let mut out = String::with_capacity(org.len());
    for b in org.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn generation_key(namespace: &str, org: &str) -> String {
    format!("oc:{namespace}:{}:gen", org_segment(org))
}

fn entry_key(namespace: &str, org: &str, generation: &str, key: &str) -> String {
    let hash = Sha256::digest(key.as_bytes());
    format!("oc:{namespace}:{}:{generation}:{}", org_segment(org), data_encoding::HEXLOWER.encode(&hash[..16]))
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let packed = rmp_serde::to_vec_named(value).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(packed.len() / 4 + 16);
    out.push(VALUE);
    zstd::stream::copy_encode(packed.as_slice(), &mut out, ZSTD_LEVEL).map_err(|e| e.to_string())?;
    Ok(out)
}

/// A stored value, or `None` for the "too large" marker.
fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<Option<T>, String> {
    match bytes.split_first() {
        Some((&TOO_LARGE, [])) => Ok(None),
        Some((&VALUE, body)) => {
            let packed = zstd::stream::decode_all(body).map_err(|e| e.to_string())?;
            rmp_serde::from_slice(&packed).map(Some).map_err(|e| e.to_string())
        }
        _ => Err("unknown value format".into()),
    }
}

/// `url` without its credentials, for messages.
fn redacted(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => format!("{}://{}:{}{}", u.scheme(), u.host_str().unwrap_or(""), u.port().unwrap_or(6379), u.path()),
        Err(_) => "(unparseable URL)".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Instant;

    /// In-memory stand-in for Valkey: values with expiry, `SET NX`, compare-and-delete. Shared by
    /// several caches, it plays one Valkey behind several replicas.
    #[derive(Default)]
    struct FakeBackend {
        data: Mutex<HashMap<String, (Vec<u8>, Instant)>>,
        down: AtomicBool,
        ops: AtomicUsize,
    }

    impl FakeBackend {
        fn check(&self) -> Result<(), BackendError> {
            self.ops.fetch_add(1, Ordering::SeqCst);
            if self.down.load(Ordering::SeqCst) {
                return Err(BackendError("connection refused".into()));
            }
            Ok(())
        }
        fn keys(&self) -> Vec<String> {
            let now = Instant::now();
            let mut k: Vec<String> = self.data.lock().unwrap().iter().filter(|(_, (_, exp))| *exp > now).map(|(k, _)| k.clone()).collect();
            k.sort();
            k
        }
        fn live(&self, key: &str) -> Option<Vec<u8>> {
            let d = self.data.lock().unwrap();
            d.get(key).filter(|(_, exp)| *exp > Instant::now()).map(|(v, _)| v.clone())
        }
    }

    #[async_trait]
    impl Backend for FakeBackend {
        async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, BackendError> {
            self.check()?;
            Ok(self.live(key))
        }
        async fn set(&self, key: &str, value: &[u8], ttl: Duration) -> Result<(), BackendError> {
            self.check()?;
            self.data.lock().unwrap().insert(key.into(), (value.to_vec(), Instant::now() + ttl));
            Ok(())
        }
        async fn set_nx(&self, key: &str, value: &[u8], ttl: Duration) -> Result<bool, BackendError> {
            self.check()?;
            if self.live(key).is_some() {
                return Ok(false);
            }
            self.data.lock().unwrap().insert(key.into(), (value.to_vec(), Instant::now() + ttl));
            Ok(true)
        }
        async fn del_if(&self, key: &str, value: &[u8]) -> Result<(), BackendError> {
            self.check()?;
            let mut d = self.data.lock().unwrap();
            if d.get(key).is_some_and(|(v, _)| v == value) {
                d.remove(key);
            }
            Ok(())
        }
    }

    fn options() -> CacheOptions {
        CacheOptions { max_value_bytes: 1 << 20, load_timeout: Duration::from_millis(500) }
    }

    fn pair() -> (Arc<FakeBackend>, SharedCache, SharedCache) {
        let backend = Arc::new(FakeBackend::default());
        let a = SharedCache::with_backend(backend.clone(), options());
        let b = SharedCache::with_backend(backend.clone(), options());
        (backend, a, b)
    }

    const TTL: Duration = Duration::from_secs(30);

    #[derive(Debug)]
    struct Snapshot {
        hits: u64,
        misses: u64,
        loads: u64,
        coalesced_local: u64,
        coalesced_remote: u64,
        oversized: u64,
        errors: u64,
    }

    impl SharedCache {
        fn stats_for(&self, namespace: &'static str) -> Snapshot {
            let c = self.stats(namespace);
            let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
            Snapshot {
                hits: get(&c.hits),
                misses: get(&c.misses),
                loads: get(&c.loads),
                coalesced_local: get(&c.coalesced_local),
                coalesced_remote: get(&c.coalesced_remote),
                oversized: get(&c.oversized),
                errors: get(&c.errors),
            }
        }
    }

    fn counting<'a>(loads: &'a AtomicUsize, value: Vec<u64>, delay_ms: u64) -> impl FnOnce() -> futures::future::BoxFuture<'a, Result<Vec<u64>, String>> {
        move || {
            Box::pin(async move {
                loads.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                Ok(value)
            })
        }
    }

    #[tokio::test]
    async fn a_stored_value_is_served_to_other_replicas() {
        let (_, a, b) = pair();
        let loads = AtomicUsize::new(0);
        assert_eq!(*a.get_or_load("widgets", "acme", "q1", TTL, counting(&loads, vec![1, 2], 0)).await.unwrap(), vec![1, 2]);
        assert_eq!(*b.get_or_load("widgets", "acme", "q1", TTL, counting(&loads, vec![9], 0)).await.unwrap(), vec![1, 2]);
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        let s = b.stats_for("widgets");
        assert_eq!((s.hits, s.misses), (1, 0));
        assert_eq!(a.stats_for("widgets").loads, 1);
    }

    #[tokio::test]
    async fn keys_carry_namespace_and_org_and_orgs_never_share_entries() {
        let (backend, a, _) = pair();
        let loads = AtomicUsize::new(0);
        a.get_or_load("widgets", "acme", "q1", TTL, counting(&loads, vec![1], 0)).await.unwrap();
        assert_eq!(*a.get_or_load("widgets", "globex", "q1", TTL, counting(&loads, vec![2], 0)).await.unwrap(), vec![2]);
        assert_eq!(*a.get_or_load("tags", "acme", "q1", TTL, counting(&loads, vec![3], 0)).await.unwrap(), vec![3]);
        assert_eq!(loads.load(Ordering::SeqCst), 3);
        let keys = backend.keys();
        let entries: Vec<&String> = keys.iter().filter(|k| !k.ends_with(":gen")).collect();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(|k| k.starts_with("oc:widgets:acme:")));
        assert!(entries.iter().any(|k| k.starts_with("oc:widgets:globex:")));
        assert!(entries.iter().any(|k| k.starts_with("oc:tags:acme:")));
        // The cache key is hashed: no query text in Valkey.
        assert!(entries.iter().all(|k| !k.contains("q1")));
    }

    #[test]
    fn org_ids_cannot_forge_another_orgs_prefix() {
        assert_ne!(org_segment("acme:1"), "acme:1");
        assert_ne!(org_segment("acme%3A1"), org_segment("acme:1"));
        assert_eq!(org_segment("load-org-1"), "load-org-1");
    }

    #[tokio::test]
    async fn concurrent_callers_on_one_replica_share_one_load() {
        let (_, a, _) = pair();
        let loads = AtomicUsize::new(0);
        let (x, y, z) = tokio::join!(
            a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![7], 30)),
            a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![8], 30)),
            a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![9], 30)),
        );
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert_eq!((x.unwrap().to_vec(), y.unwrap().to_vec(), z.unwrap().to_vec()), (vec![7], vec![7], vec![7]));
        assert_eq!(a.stats_for("widgets").coalesced_local, 2);
    }

    #[tokio::test]
    async fn replicas_missing_the_same_key_load_it_once() {
        let (_, a, b) = pair();
        let loads = AtomicUsize::new(0);
        let (x, y) = tokio::join!(a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![1], 80)), async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            b.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![2], 80)).await
        });
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert_eq!((x.unwrap().to_vec(), y.unwrap().to_vec()), (vec![1], vec![1]));
        assert_eq!(b.stats_for("widgets").coalesced_remote, 1);
    }

    #[tokio::test]
    async fn a_waiter_loads_itself_when_the_lock_holder_fails() {
        let (_, a, b) = pair();
        let loads = AtomicUsize::new(0);
        let failing = || async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Err::<Vec<u64>, String>("clickhouse down".into())
        };
        let (x, y) = tokio::join!(a.get_or_load("widgets", "acme", "q", TTL, failing), async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            b.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![3], 0)).await
        });
        assert_eq!(x.unwrap_err(), "clickhouse down");
        assert_eq!(*y.unwrap(), vec![3]);
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_waiter_loads_itself_after_the_load_timeout_when_the_holder_died() {
        let (backend, a, b) = pair();
        // A replica took the lock and died: the lock stays until it expires.
        let gen = a.generation("widgets", "acme").await.unwrap();
        let lock = format!("{}:lock", entry_key("widgets", "acme", &gen, "q"));
        backend.set(&lock, b"dead-replica", Duration::from_millis(200)).await.unwrap();
        let loads = AtomicUsize::new(0);
        let started = Instant::now();
        assert_eq!(*b.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![4], 0)).await.unwrap(), vec![4]);
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(190) && waited < Duration::from_millis(450), "{waited:?}");
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_cancelled_lock_holder_releases_its_lock() {
        let (_, a, b) = pair();
        let loads = AtomicUsize::new(0);
        // The client of replica a goes away while its load runs (the request future is dropped).
        let cancelled = tokio::time::timeout(Duration::from_millis(30), a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![1], 1_000))).await;
        assert!(cancelled.is_err());
        tokio::task::yield_now().await;
        let started = Instant::now();
        assert_eq!(*b.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![2], 0)).await.unwrap(), vec![2]);
        // Not the 500 ms load timeout: the lock was released when the holder was dropped.
        assert!(started.elapsed() < Duration::from_millis(100), "{:?}", started.elapsed());
    }

    #[tokio::test]
    async fn entries_expire_after_their_ttl() {
        let (_, a, _) = pair();
        let loads = AtomicUsize::new(0);
        a.get_or_load("widgets", "acme", "q", Duration::from_millis(40), counting(&loads, vec![1], 0)).await.unwrap();
        a.get_or_load("widgets", "acme", "q", Duration::from_millis(40), counting(&loads, vec![1], 0)).await.unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_millis(60)).await;
        a.get_or_load("widgets", "acme", "q", Duration::from_millis(40), counting(&loads, vec![1], 0)).await.unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn invalidate_drops_one_orgs_entries_of_one_namespace() {
        let (_, a, b) = pair();
        let loads = AtomicUsize::new(0);
        for (ns, org) in [("widgets", "acme"), ("widgets", "globex"), ("tags", "acme")] {
            a.get_or_load(ns, org, "q", TTL, counting(&loads, vec![1], 0)).await.unwrap();
        }
        b.invalidate("widgets", "acme").await;
        assert_eq!(*a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![2], 0)).await.unwrap(), vec![2]);
        assert_eq!(loads.load(Ordering::SeqCst), 4);
        a.get_or_load("widgets", "globex", "q", TTL, counting(&loads, vec![3], 0)).await.unwrap();
        a.get_or_load("tags", "acme", "q", TTL, counting(&loads, vec![3], 0)).await.unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn a_lost_generation_never_brings_back_old_entries() {
        let (backend, a, _) = pair();
        let loads = AtomicUsize::new(0);
        a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![1], 0)).await.unwrap();
        // Valkey evicted the generation key (allkeys-lru): the old entry must not be found again.
        backend.data.lock().unwrap().remove("oc:widgets:acme:gen");
        assert_eq!(*a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![2], 0)).await.unwrap(), vec![2]);
    }

    #[tokio::test]
    async fn oversized_values_are_returned_but_not_stored() {
        let backend = Arc::new(FakeBackend::default());
        let opts = CacheOptions { max_value_bytes: 64, ..options() };
        let (a, b) = (SharedCache::with_backend(backend.clone(), opts.clone()), SharedCache::with_backend(backend.clone(), opts));
        let loads = AtomicUsize::new(0);
        // Random-looking numbers that zstd can't shrink below 64 bytes.
        let big: Vec<u64> = (0..64u64).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15)).collect();
        assert_eq!(a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, big.clone(), 0)).await.unwrap().len(), 64);
        assert_eq!(a.stats_for("widgets").oversized, 1);
        // Other replicas see the marker and load at once instead of waiting for a lock.
        b.get_or_load("widgets", "acme", "q", TTL, counting(&loads, big, 0)).await.unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 2);
        assert!(backend.keys().iter().all(|k| backend.live(k).unwrap().len() < 64));
    }

    #[tokio::test]
    async fn requests_are_served_from_the_source_while_valkey_is_down() {
        let (backend, a, _) = pair();
        backend.down.store(true, Ordering::SeqCst);
        let loads = AtomicUsize::new(0);
        assert_eq!(*a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![5], 0)).await.unwrap(), vec![5]);
        assert!(a.stats_for("widgets").errors >= 1);
        // Further requests skip Valkey for a moment instead of timing out on it each time.
        let ops = backend.ops.load(Ordering::SeqCst);
        a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![5], 0)).await.unwrap();
        assert_eq!(backend.ops.load(Ordering::SeqCst), ops);
        assert_eq!(loads.load(Ordering::SeqCst), 2);
        a.invalidate("widgets", "acme").await;
    }

    #[tokio::test]
    async fn stats_render_as_prometheus_text() {
        let (_, a, _) = pair();
        let loads = AtomicUsize::new(0);
        a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![1], 0)).await.unwrap();
        a.get_or_load("widgets", "acme", "q", TTL, counting(&loads, vec![1], 0)).await.unwrap();
        let text = a.render_metrics();
        assert!(text.contains("# TYPE oc_cache_lookups_total counter"), "{text}");
        assert!(text.contains("oc_cache_lookups_total{namespace=\"widgets\",result=\"hit\"} 1"), "{text}");
        assert!(text.contains("oc_cache_lookups_total{namespace=\"widgets\",result=\"miss\"} 1"), "{text}");
        assert!(text.contains("oc_cache_loads_total{namespace=\"widgets\"} 1"), "{text}");
        assert!(text.contains("oc_cache_value_bytes_count{namespace=\"widgets\"} 1"), "{text}");
        assert!(text.contains("oc_cache_value_bytes_bucket{namespace=\"widgets\",le=\"+Inf\"} 1"), "{text}");
    }

    #[test]
    fn values_round_trip_compactly() {
        let v: Vec<String> = (0..1000).map(|i| format!("order-service-{}", i % 10)).collect();
        let bytes = encode(&v).unwrap();
        assert!(bytes.len() < 1024, "{}", bytes.len()); // ~17 KB as MessagePack
        assert_eq!(decode::<Vec<String>>(&bytes).unwrap(), Some(v));
        assert_eq!(decode::<Vec<String>>(&[TOO_LARGE]).unwrap(), None);
        assert!(decode::<Vec<String>>(&[42, 1, 2]).is_err());
    }

    /// Live tests against a real Valkey: `OC_TEST_VALKEY_URL=redis://localhost:16379/15 cargo test -p oc-api valkey_`.
    fn valkey_url() -> Option<String> {
        std::env::var("OC_TEST_VALKEY_URL").ok().filter(|u| !u.is_empty())
    }

    async fn live_pair(opts: CacheOptions) -> Option<(SharedCache, SharedCache)> {
        let url = valkey_url()?;
        Some((SharedCache::connect(&url, opts.clone(), Duration::from_secs(5)).await.unwrap(), SharedCache::connect(&url, opts, Duration::from_secs(5)).await.unwrap()))
    }

    fn unique(prefix: &str) -> String {
        format!("{prefix}-{}", rand::random::<u32>())
    }

    #[tokio::test]
    async fn valkey_two_replicas_coalesce_one_load() {
        let Some((a, b)) = live_pair(options()).await else { return };
        let org = unique("acme");
        let loads = AtomicUsize::new(0);
        // Replica b asks only once replica a's load has started, i.e. a holds the lock: a fixed
        // delay was racy under a busy test run.
        let (started_tx, started_rx) = tokio::sync::watch::channel(false);
        let load_a = || {
            let first = counting(&loads, vec![1], 100);
            async move {
                let _ = started_tx.send(true);
                first().await
            }
        };
        let wait_started = || {
            let mut rx = started_rx.clone();
            async move {
                let _ = rx.wait_for(|started| *started).await;
            }
        };
        let (x, y, z) = tokio::join!(
            a.get_or_load("widgets", &org, "q", TTL, load_a),
            async {
                wait_started().await;
                b.get_or_load("widgets", &org, "q", TTL, counting(&loads, vec![2], 100)).await
            },
            async {
                wait_started().await;
                b.get_or_load("widgets", &org, "q", TTL, counting(&loads, vec![3], 100)).await
            },
        );
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert_eq!((x.unwrap().to_vec(), y.unwrap().to_vec(), z.unwrap().to_vec()), (vec![1], vec![1], vec![1]));
        let s = b.stats_for("widgets");
        assert_eq!((s.coalesced_remote, s.coalesced_local), (1, 1));
    }

    #[tokio::test]
    async fn valkey_ttl_expiry_org_isolation_and_invalidation() {
        let Some((a, b)) = live_pair(options()).await else { return };
        let (acme, globex) = (unique("acme"), unique("globex"));
        let loads = AtomicUsize::new(0);
        let short = Duration::from_millis(300);
        a.get_or_load("widgets", &acme, "q", short, counting(&loads, vec![1], 0)).await.unwrap();
        assert_eq!(*b.get_or_load("widgets", &globex, "q", TTL, counting(&loads, vec![2], 0)).await.unwrap(), vec![2]);
        assert_eq!(*b.get_or_load("widgets", &acme, "q", short, counting(&loads, vec![9], 0)).await.unwrap(), vec![1]);
        assert_eq!(loads.load(Ordering::SeqCst), 2);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(*b.get_or_load("widgets", &acme, "q", TTL, counting(&loads, vec![3], 0)).await.unwrap(), vec![3]);
        a.invalidate("widgets", &acme).await;
        assert_eq!(*b.get_or_load("widgets", &acme, "q", TTL, counting(&loads, vec![4], 0)).await.unwrap(), vec![4]);
        assert_eq!(*a.get_or_load("widgets", &globex, "q", TTL, counting(&loads, vec![5], 0)).await.unwrap(), vec![2]);
        assert_eq!(loads.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn valkey_oversized_values_and_a_dead_lock_holder() {
        let Some((a, b)) = live_pair(CacheOptions { max_value_bytes: 64, load_timeout: Duration::from_millis(300) }).await else { return };
        let org = unique("acme");
        let loads = AtomicUsize::new(0);
        let big: Vec<u64> = (0..64u64).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15)).collect();
        a.get_or_load("widgets", &org, "big", TTL, counting(&loads, big.clone(), 0)).await.unwrap();
        b.get_or_load("widgets", &org, "big", TTL, counting(&loads, big, 0)).await.unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 2);
        assert_eq!(a.stats_for("widgets").oversized, 1);
        // A replica that took the lock and crashed: the other loads once the lock expires.
        let gen = a.generation("widgets", &org).await.unwrap();
        let lock = format!("{}:lock", entry_key("widgets", &org, &gen, "q"));
        a.backend.set_nx(&lock, b"dead-replica", Duration::from_millis(300)).await.unwrap();
        let started = Instant::now();
        assert_eq!(*b.get_or_load("widgets", &org, "q", TTL, counting(&loads, vec![6], 0)).await.unwrap(), vec![6]);
        assert!(started.elapsed() >= Duration::from_millis(250), "{:?}", started.elapsed());
        assert_eq!(loads.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn valkey_unreachable_url_fails_startup_with_a_clear_message() {
        let err = SharedCache::connect("redis://127.0.0.1:1/0", options(), Duration::from_millis(300)).await.err().expect("connect must fail");
        let msg = format!("{err:#}");
        assert!(msg.contains("OC_CACHE_URL") && msg.contains("127.0.0.1:1"), "{msg}");
        let err = SharedCache::connect("http://valkey:6379", options(), Duration::from_millis(300)).await.err().expect("connect must fail");
        assert!(format!("{err:#}").contains("redis://"), "{err:#}");
        let err = SharedCache::connect("redis://:s3cret@127.0.0.1:1/0", options(), Duration::from_millis(300)).await.err().expect("connect must fail");
        assert!(!format!("{err:#}").contains("s3cret"), "{err:#}");
    }

    /// Round trip of a hit (generation + value, decode): `OC_TEST_VALKEY_URL=… cargo test --release -p oc-api valkey_hit_latency -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn valkey_hit_latency() {
        let Some((a, _)) = live_pair(options()).await else { return };
        let org = unique("acme");
        for (label, n) in [("1 KiB", 1usize << 7), ("100 KiB", 100 << 7)] {
            let value: Vec<u64> = (0..n as u64).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15)).collect();
            a.get_or_load("bench", &org, label, TTL, || async { Ok::<_, ()>(value.clone()) }).await.unwrap();
            let started = Instant::now();
            for _ in 0..1000 {
                a.get_or_load("bench", &org, label, TTL, || async { Ok::<Vec<u64>, ()>(vec![]) }).await.unwrap();
            }
            println!("{label}: {:?} per hit", started.elapsed() / 1000);
        }
    }

    /// Serialization cost and size: `cargo test --release -p oc-api cache_codec_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn cache_codec_cost() {
        #[derive(Serialize, serde::Deserialize)]
        struct Series {
            metric: String,
            tags: std::collections::BTreeMap<String, String>,
            points: Vec<(i64, Option<f64>)>,
        }
        for target in [1usize << 20, 10 << 20] {
            let one = |i: u64| Series {
                metric: "kubernetes.cpu.usage.total".into(),
                tags: (0..12).map(|k| (format!("kube_label_{k}"), format!("value-{k}-{}", i % 97))).chain([("pod_name".to_string(), format!("order-service-7d9f8c6b5-{i:05x}"))]).collect(),
                points: (0..300).map(|p| (1_761_700_000_000 + p * 15_000, Some((i.wrapping_mul(0x9E37_79B9) ^ p as u64) as f64 / 7.0))).collect(),
            };
            let per = serde_json::to_vec(&one(0)).unwrap().len();
            let value: Vec<Series> = (0..(target / per) as u64).map(one).collect();
            let json = serde_json::to_vec(&value).unwrap().len();
            let t = Instant::now();
            let bytes = encode(&value).unwrap();
            let enc = t.elapsed();
            let t = Instant::now();
            decode::<Vec<Series>>(&bytes).unwrap().unwrap();
            println!("{:.1} MiB as JSON: stored {} bytes ({:.1} %), encode {:?}, decode {:?}", json as f64 / 1048576.0, bytes.len(), bytes.len() as f64 * 100.0 / json as f64, enc, t.elapsed());
        }
    }
}
