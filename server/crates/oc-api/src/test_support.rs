//! Helpers for tests that run against a live ClickHouse (`OC_TEST_CLICKHOUSE_URL`) and Postgres
//! (`OC_TEST_POSTGRES_URL`).
//!
//! With `OC_TEST_CLICKHOUSE_CLUSTER` set (e.g. `oc`, from `make cluster-up`), the URL is a node of
//! that cluster: migrations create the sharded, replicated tables and the same tests check that
//! the cluster gives the single server's results.

use std::sync::Arc;

use tokio::sync::OnceCell;

use crate::cache::testing::FakeBackend;
use crate::state::{AppState, Inner};

static MIGRATED: OnceCell<()> = OnceCell::const_new();
static PG_MIGRATED: OnceCell<()> = OnceCell::const_new();

/// Applies the ClickHouse migrations once per test process. Live tests run in parallel, and on a
/// fresh server their concurrent migration runs raced each other creating the same tables.
pub async fn migrate_clickhouse_once(cfg: &oc_store::ClickHouseConfig) {
    MIGRATED.get_or_init(|| async { oc_store::migrate_clickhouse(cfg, false).await.expect("migrate") }).await;
}

/// `count` server replicas over the test databases (migrations applied) sharing one in-memory
/// Valkey; None (the test is skipped) unless both `OC_TEST_CLICKHOUSE_URL` and
/// `OC_TEST_POSTGRES_URL` are set.
pub async fn replicas(count: usize) -> Option<(Arc<FakeBackend>, Vec<AppState>)> {
    replicas_with(count, |_| {}).await
}

/// [`replicas`] with the server configuration adjusted by `configure`.
pub async fn replicas_with(count: usize, configure: impl Fn(&mut crate::config::Config)) -> Option<(Arc<FakeBackend>, Vec<AppState>)> {
    let (Ok(ch_url), Ok(pg_url)) = (std::env::var("OC_TEST_CLICKHOUSE_URL"), std::env::var("OC_TEST_POSTGRES_URL")) else {
        eprintln!("OC_TEST_CLICKHOUSE_URL or OC_TEST_POSTGRES_URL unset: skipping");
        return None;
    };
    let tables = || oc_store::ClickHouseConfig {
        url: ch_url.clone(),
        user: std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into()),
        password: std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default(),
        database: "oc".into(),
        cluster: test_cluster(),
    };
    migrate_clickhouse_once(&tables()).await;
    let postgres = oc_store::postgres(&pg_url, std::time::Duration::from_secs(5)).await.expect("postgres");
    PG_MIGRATED.get_or_init(|| async { oc_store::migrate_postgres(&postgres).await.expect("migrate postgres") }).await;
    let backend = Arc::new(FakeBackend::default());
    let states = (0..count)
        .map(|_| {
            AppState(Arc::new(Inner {
                ingest_http: reqwest::Client::new(),
                config: {
                    let mut config = crate::config::Config::from_env();
                    configure(&mut config);
                    config
                },
                clickhouse: oc_store::clickhouse(&tables()),
                org_clients: crate::org_limits::OrgClients::new(&tables(), crate::org_limits::OrgQueryLimits::from_env()),
                tables: tables(),
                postgres: postgres.clone(),
                live: None,
                secrets: crate::auth::crypto::SecretBox::new("test-secret"),
                log_restrictions: Default::default(),
                kube_inventory: crate::ttl_cache::TtlCache::new(crate::routes::kubernetes::INVENTORY_TTL),
                cache: crate::cache::SharedCache::with_backend(backend.clone(), crate::cache::testing::options()),
            }))
        })
        .collect();
    Some((backend, states))
}

/// A fresh org id for one test run (rows in the shared test databases never collide).
pub fn org(prefix: &str) -> String {
    format!("{prefix}-{}-{}", std::process::id(), crate::routes::metrics::now_ms())
}

/// A new org in Postgres.
pub async fn new_org(st: &AppState, prefix: &str) -> String {
    let org = org(prefix);
    sqlx::query("INSERT INTO orgs (id, name) VALUES ($1, $1)").bind(&org).execute(&st.postgres).await.expect("org");
    org
}

/// A new org in Postgres and an admin acting in it (for handlers behind `Admin` / `Writer`).
pub async fn admin_of_new_org(st: &AppState, prefix: &str) -> crate::auth::AuthContext {
    let org = new_org(st, prefix).await;
    crate::auth::AuthContext {
        user_id: "00000000-0000-4000-8000-000000000001".into(),
        org_id: org,
        account_id: "00000000-0000-4000-8000-000000000001".into(),
        instance_admin: false,
        name: "Test Admin".into(),
        access: crate::auth::Access::Admin,
        credential: crate::auth::Credential::Session,
        session_hash: None,
        ip: None,
    }
}

/// The ClickHouse of the live tests, migrated.
pub struct LiveClickHouse {
    pub cfg: oc_store::ClickHouseConfig,
    /// The server's client (`oc_store::clickhouse`) as the `default` user (or OC_TEST_CLICKHOUSE_USER).
    /// Metric point queries add `distributed_product_mode = 'local'` as `points_client` does: use
    /// [`LiveClickHouse::points`]. On a cluster, inserts wait until every
    /// shard has its rows, and reads and inserts use the first replica of each shard, so a test
    /// reads what it just wrote (the second replica fetches it in the background).
    pub client: clickhouse::Client,
}

impl LiveClickHouse {
    pub fn cluster(&self) -> bool {
        self.cfg.cluster.is_some()
    }

    /// `client` as `routes::metrics::points_client` sets it up for metric point queries.
    pub fn points(&self, client: &clickhouse::Client) -> clickhouse::Client {
        crate::routes::metrics::points_settings(client.clone(), self.cluster())
    }
}

/// `None` (the test is skipped) unless OC_TEST_CLICKHOUSE_URL is set.
pub async fn live_clickhouse() -> Option<LiveClickHouse> {
    let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_URL") else {
        eprintln!("OC_TEST_CLICKHOUSE_URL unset: skipping");
        return None;
    };
    let user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
    let password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
    let cfg = oc_store::ClickHouseConfig { url: url.clone(), user: user.clone(), password: password.clone(), database: "oc".into(), cluster: test_cluster() };
    migrate_clickhouse_once(&cfg).await;
    // The server's client (cluster settings included), as the routes get it through AppState.
    let client = oc_store::clickhouse(&cfg);
    let client = if cfg.cluster.is_some() { client.with_option("insert_distributed_sync", "1").with_option("load_balancing", "in_order") } else { client };
    Some(LiveClickHouse { cfg, client })
}

/// The cluster OC_TEST_CLICKHOUSE_URL is a node of (`OC_TEST_CLICKHOUSE_CLUSTER`), if any.
fn test_cluster() -> Option<String> {
    std::env::var("OC_TEST_CLICKHOUSE_CLUSTER").ok().filter(|c| !c.is_empty())
}
