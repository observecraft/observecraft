//! Per-organization query limits (docs/adr/0002, "Per-org protection"), so one organization can't
//! starve the others:
//!
//! - every ClickHouse query of an org carries `max_execution_time`, `max_memory_usage` and
//!   optionally `max_rows_to_read` (`OC_ORG_QUERY_*`, see [`OrgQueryLimits`]), and its org as
//!   `quota_key` for the ClickHouse quota of the Helm chart;
//! - each server replica runs at most `OC_ORG_QUERY_CONCURRENCY` (16) ClickHouse queries of one
//!   org at once. More wait up to `OC_ORG_QUERY_QUEUE_TIMEOUT_MS` (10 s) for a slot, then fail
//!   with [`OrgBusy`], which the API answers with 429 Too Many Requests.
//!
//! The concurrency limit lives in the HTTP connections: each org's client has its own connection
//! pool whose connector hands out a connection only with one of the org's permits, held as long as
//! the connection is open (one query at a time per HTTP/1.1 connection; an idle pooled connection
//! keeps its permit for the pool's 2 s idle timeout and serves the org's next query). So every
//! call site of `AppState::ch` is limited without knowing it, cursors and inserts included.
//!
//! Background jobs (the metric rollup job, monitor evaluation, schedulers) run inside
//! [`background`] and are exempt from the concurrency limit, so an org's busy users can't delay its
//! alerts or rollups; their own scheduling bounds them (one org at a time per job and replica), and
//! the time and memory limits still apply.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use hyper_util::client::legacy::connect::{Connected, Connection, HttpConnector};
use hyper_util::rt::TokioExecutor;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Limits on the ClickHouse queries of one organization (per server replica for `concurrency`).
#[derive(Clone, Debug, PartialEq)]
pub struct OrgQueryLimits {
    /// `OC_ORG_QUERY_TIMEOUT_SECONDS` (`OC_QUERY_TIMEOUT_SECONDS`), 60: `max_execution_time`.
    pub timeout_secs: u32,
    /// `OC_ORG_QUERY_MAX_MEMORY_BYTES` (`OC_QUERY_MAX_MEMORY_BYTES`), 4 GiB: `max_memory_usage`.
    pub max_memory_bytes: u64,
    /// `OC_ORG_QUERY_MAX_ROWS_TO_READ`, 0 = no limit: `max_rows_to_read`, failing the query
    /// (`read_overflow_mode = 'throw'`; truncated results would be silently wrong).
    pub max_rows_to_read: u64,
    /// `OC_ORG_QUERY_CONCURRENCY`, 16, 0 = no limit: queries of an org running at once.
    pub concurrency: usize,
    /// `OC_ORG_QUERY_QUEUE_TIMEOUT_MS`, 10 000: how long a query waits for one of them.
    pub queue_timeout: Duration,
}

impl OrgQueryLimits {
    pub fn from_env() -> Self {
        Self::from_vars(|key| std::env::var(key).ok())
    }

    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        fn number<T: std::str::FromStr>(var: &impl Fn(&str) -> Option<String>, keys: &[&str], default: T) -> T {
            keys.iter().find_map(|k| var(k)).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
        }
        OrgQueryLimits {
            timeout_secs: number(&var, &["OC_ORG_QUERY_TIMEOUT_SECONDS", "OC_QUERY_TIMEOUT_SECONDS"], 60u32).max(1),
            max_memory_bytes: number(&var, &["OC_ORG_QUERY_MAX_MEMORY_BYTES", "OC_QUERY_MAX_MEMORY_BYTES"], 4u64 << 30),
            max_rows_to_read: number(&var, &["OC_ORG_QUERY_MAX_ROWS_TO_READ"], 0u64),
            concurrency: number(&var, &["OC_ORG_QUERY_CONCURRENCY"], 16usize),
            queue_timeout: Duration::from_millis(number(&var, &["OC_ORG_QUERY_QUEUE_TIMEOUT_MS"], 10_000u64)),
        }
    }

    /// The ClickHouse settings every query of an org carries.
    pub fn settings(&self) -> Vec<(&'static str, String)> {
        let mut settings = vec![("max_execution_time", self.timeout_secs.to_string()), ("max_memory_usage", self.max_memory_bytes.to_string())];
        if self.max_rows_to_read > 0 {
            settings.push(("max_rows_to_read", self.max_rows_to_read.to_string()));
            settings.push(("read_overflow_mode", "throw".to_string()));
        }
        settings
    }
}

/// A query of an org that found all of the org's query slots taken for the whole queue timeout.
#[derive(Debug)]
pub struct OrgBusy;

impl std::fmt::Display for OrgBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(ORG_BUSY)
    }
}

impl std::error::Error for OrgBusy {}

pub const ORG_BUSY: &str = "This organization is running too many queries at once; retry shortly.";

/// Whether `e` is a query that gave up waiting for one of its org's slots ([`OrgBusy`]).
pub fn is_org_busy(e: &clickhouse::error::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = source {
        if err.is::<OrgBusy>() {
            return true;
        }
        source = err.source();
    }
    false
}

tokio::task_local! {
    static BACKGROUND: ();
}

/// Runs `job` as background work: its ClickHouse queries don't take the orgs' query slots (see the
/// module docs). Tasks it spawns are not covered.
pub async fn background<F: Future>(job: F) -> F::Output {
    BACKGROUND.scope((), job).await
}

fn in_background() -> bool {
    BACKGROUND.try_with(|_| ()).is_ok()
}

/// ClickHouse clients for organizations' telemetry, with their limits.
pub struct OrgClients {
    cfg: oc_store::ClickHouseConfig,
    limits: OrgQueryLimits,
    /// Without a concurrency limit: background work, and every org when the limit is off.
    shared: clickhouse::Client,
    /// Each org's client, on its own connection pool limited to its slots.
    per_org: Mutex<HashMap<String, clickhouse::Client>>,
}

impl OrgClients {
    pub fn new(cfg: &oc_store::ClickHouseConfig, limits: OrgQueryLimits) -> Self {
        OrgClients { cfg: cfg.clone(), limits, shared: oc_store::clickhouse(cfg), per_org: Mutex::default() }
    }

    /// The client for one org's queries: `{oc_org:String}` is the org, `quota_key` too, and the
    /// org's limits apply.
    pub fn client(&self, org: &str) -> clickhouse::Client {
        let base = if self.limits.concurrency == 0 || in_background() { self.shared.clone() } else { self.per_org.lock().unwrap().entry(org.to_string()).or_insert_with(|| self.limited()).clone() };
        let client = base.with_option("param_oc_org", org).with_option("quota_key", org);
        self.limits.settings().into_iter().fold(client, |c, (name, value)| c.with_option(name, value))
    }

    /// A client on a connection pool of at most `concurrency` connections (as the clickhouse
    /// crate's default pool otherwise: TCP keepalive 60 s, idle connections closed after 2 s).
    fn limited(&self) -> clickhouse::Client {
        let mut http = HttpConnector::new();
        http.set_keepalive(Some(Duration::from_secs(60)));
        let connector = SlotConnector { http, slots: Arc::new(Semaphore::new(self.limits.concurrency)), wait: self.limits.queue_timeout };
        let pool = hyper_util::client::legacy::Client::builder(TokioExecutor::new()).pool_idle_timeout(Duration::from_secs(2)).build(connector);
        oc_store::configure(clickhouse::Client::with_http_client(pool), &self.cfg)
    }
}

/// Opens a connection once it has one of the org's slots, which the connection keeps.
#[derive(Clone)]
struct SlotConnector {
    http: HttpConnector,
    slots: Arc<Semaphore>,
    wait: Duration,
}

impl tower_service::Service<hyper::Uri> for SlotConnector {
    type Response = SlotConnection<<HttpConnector as tower_service::Service<hyper::Uri>>::Response>;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.http.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, uri: hyper::Uri) -> Self::Future {
        let (mut http, slots, wait) = (self.http.clone(), self.slots.clone(), self.wait);
        Box::pin(async move {
            let slot = match tokio::time::timeout(wait, slots.acquire_owned()).await {
                Ok(slot) => slot.expect("the slots are never closed"),
                Err(_) => return Err(Box::new(OrgBusy) as Self::Error),
            };
            let io = http.call(uri).await?;
            Ok(SlotConnection { io, _slot: slot })
        })
    }
}

/// A connection and the org slot it holds while open.
struct SlotConnection<T> {
    io: T,
    _slot: OwnedSemaphorePermit,
}

impl<T: Connection> Connection for SlotConnection<T> {
    fn connected(&self) -> Connected {
        self.io.connected()
    }
}

impl<T: hyper::rt::Read + Unpin> hyper::rt::Read for SlotConnection<T> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: hyper::rt::ReadBufCursor<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}

impl<T: hyper::rt::Write + Unpin> hyper::rt::Write for SlotConnection<T> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_write_vectored(self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[std::io::IoSlice<'_>]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write_vectored(cx, bufs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn defaults_bound_time_memory_and_concurrency() {
        let limits = OrgQueryLimits::from_vars(vars(&[]));
        assert_eq!(limits, OrgQueryLimits { timeout_secs: 60, max_memory_bytes: 4 << 30, max_rows_to_read: 0, concurrency: 16, queue_timeout: Duration::from_secs(10) });
        assert_eq!(limits.settings(), vec![("max_execution_time", "60".to_string()), ("max_memory_usage", (4u64 << 30).to_string())]);
    }

    #[test]
    fn the_environment_sets_each_limit_and_the_old_names_still_work() {
        let limits = OrgQueryLimits::from_vars(vars(&[
            ("OC_ORG_QUERY_TIMEOUT_SECONDS", "20"),
            ("OC_QUERY_MAX_MEMORY_BYTES", "1000000"),
            ("OC_ORG_QUERY_MAX_ROWS_TO_READ", "5000000000"),
            ("OC_ORG_QUERY_CONCURRENCY", "4"),
            ("OC_ORG_QUERY_QUEUE_TIMEOUT_MS", "250"),
        ]));
        assert_eq!(limits, OrgQueryLimits { timeout_secs: 20, max_memory_bytes: 1_000_000, max_rows_to_read: 5_000_000_000, concurrency: 4, queue_timeout: Duration::from_millis(250) });
        assert_eq!(
            limits.settings(),
            vec![("max_execution_time", "20".to_string()), ("max_memory_usage", "1000000".to_string()), ("max_rows_to_read", "5000000000".to_string()), ("read_overflow_mode", "throw".to_string()),]
        );
        // The new names win over the old ones; nonsense falls back to the default.
        let limits = OrgQueryLimits::from_vars(vars(&[("OC_QUERY_TIMEOUT_SECONDS", "5"), ("OC_ORG_QUERY_TIMEOUT_SECONDS", "7"), ("OC_ORG_QUERY_CONCURRENCY", "many")]));
        assert_eq!((limits.timeout_secs, limits.concurrency), (7, 16));
        // 0 turns the concurrency limit off; the time limit is at least a second.
        let limits = OrgQueryLimits::from_vars(vars(&[("OC_ORG_QUERY_CONCURRENCY", "0"), ("OC_ORG_QUERY_TIMEOUT_SECONDS", "0")]));
        assert_eq!((limits.timeout_secs, limits.concurrency), (1, 0));
    }

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    /// Queries running at the fake ClickHouse: now, and the most at once.
    #[derive(Default)]
    struct Running {
        now: AtomicUsize,
        max: AtomicUsize,
    }

    /// A stand-in for ClickHouse's HTTP interface: answers every query with an empty result, after
    /// 400 ms for queries containing "slow".
    async fn fake_clickhouse() -> (String, Arc<Running>) {
        let running = Arc::new(Running::default());
        let state = running.clone();
        let app = axum::Router::new().fallback(move |body: String| {
            let running = state.clone();
            async move {
                let now = running.now.fetch_add(1, Ordering::SeqCst) + 1;
                running.max.fetch_max(now, Ordering::SeqCst);
                if body.contains("slow") {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
                running.now.fetch_sub(1, Ordering::SeqCst);
                ""
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), running)
    }

    fn clients(url: &str, concurrency: usize, queue_timeout: Duration) -> OrgClients {
        let cfg = oc_store::ClickHouseConfig { url: url.into(), user: "default".into(), password: String::new(), database: "oc".into(), cluster: None };
        OrgClients::new(&cfg, OrgQueryLimits { concurrency, queue_timeout, ..OrgQueryLimits::from_vars(|_| None) })
    }

    async fn run(ch: clickhouse::Client, sql: &'static str) -> Result<(), clickhouse::error::Error> {
        ch.query(sql).execute().await
    }

    #[tokio::test]
    async fn an_org_runs_at_most_its_concurrency_and_the_others_wait_for_a_slot() {
        let (url, running) = fake_clickhouse().await;
        let clients = clients(&url, 2, Duration::from_secs(5));
        let started = Instant::now();
        let results = futures::future::join_all((0..6).map(|_| run(clients.client("acme"), "SELECT 'slow'"))).await;
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        assert_eq!(running.max.load(Ordering::SeqCst), 2);
        assert!(started.elapsed() >= Duration::from_millis(1200), "three rounds of two");
    }

    #[tokio::test]
    async fn a_busy_org_does_not_hold_up_another() {
        let (url, running) = fake_clickhouse().await;
        let clients = clients(&url, 1, Duration::from_secs(5));
        let busy = tokio::spawn(run(clients.client("acme"), "SELECT 'slow'"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let started = Instant::now();
        run(clients.client("globex"), "SELECT 1").await.unwrap();
        assert!(started.elapsed() < Duration::from_millis(300), "another org's query waited {:?}", started.elapsed());
        // The busy org's next query waits for its slot.
        run(clients.client("acme"), "SELECT 1").await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(300));
        busy.await.unwrap().unwrap();
        assert_eq!(running.max.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_query_that_waits_too_long_fails_as_too_many_requests() {
        let (url, _) = fake_clickhouse().await;
        let clients = clients(&url, 1, Duration::from_millis(50));
        let busy = tokio::spawn(run(clients.client("acme"), "SELECT 'slow'"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let err = run(clients.client("acme"), "SELECT 1").await.unwrap_err();
        assert!(is_org_busy(&err), "{err:?}");
        let response = axum::response::IntoResponse::into_response(crate::error::ApiError::from(err));
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers().get("retry-after").map(|v| v.to_str().unwrap()), Some("1"));
        busy.await.unwrap().unwrap();
        // Other failures are not mistaken for it.
        let refused = clients.client("globex").with_url("http://127.0.0.1:9").query("SELECT 1").execute().await.unwrap_err();
        assert!(!is_org_busy(&refused));
    }

    #[tokio::test]
    async fn background_jobs_are_not_held_up_by_the_orgs_requests() {
        let (url, running) = fake_clickhouse().await;
        let clients = Arc::new(clients(&url, 1, Duration::from_millis(50)));
        let busy = tokio::spawn(run(clients.client("acme"), "SELECT 'slow'"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let job = clients.clone();
        background(async move { run(job.client("acme"), "SELECT 'slow'").await }).await.expect("the job's query runs at once");
        busy.await.unwrap().unwrap();
        assert_eq!(running.max.load(Ordering::SeqCst), 2);
    }

    /// The limits reach ClickHouse with every query of an org. Runs against OC_TEST_CLICKHOUSE_URL.
    #[tokio::test]
    async fn every_query_of_an_org_carries_its_limits() {
        let Some(live) = crate::test_support::live_clickhouse().await else { return };
        let limits = OrgQueryLimits { timeout_secs: 17, max_memory_bytes: 3 << 30, max_rows_to_read: 123_456_789, concurrency: 4, queue_timeout: Duration::from_secs(1) };
        let clients = OrgClients::new(&live.cfg, limits);
        #[derive(clickhouse::Row, serde::Deserialize, Debug, PartialEq)]
        struct Settings {
            timeout: String,
            memory: String,
            rows: String,
            overflow: String,
            org: String,
        }
        let sql = "SELECT toString(getSetting('max_execution_time')) AS timeout, toString(getSetting('max_memory_usage')) AS memory, toString(getSetting('max_rows_to_read')) AS rows, \
                   toString(getSetting('read_overflow_mode')) AS overflow, {oc_org:String} AS org";
        for client in [clients.client("acme"), background(async { clients.client("acme") }).await] {
            let got: Settings = client.query(sql).fetch_one().await.expect("settings");
            assert_eq!(got, Settings { timeout: "17".into(), memory: (3u64 << 30).to_string(), rows: "123456789".into(), overflow: "throw".into(), org: "acme".into() });
        }
    }
}
