//! Helpers for tests that run against a live ClickHouse (`OC_TEST_CLICKHOUSE_URL`) and Postgres
//! (`OC_TEST_POSTGRES_URL`).

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
    let (Ok(ch_url), Ok(pg_url)) = (std::env::var("OC_TEST_CLICKHOUSE_URL"), std::env::var("OC_TEST_POSTGRES_URL")) else {
        eprintln!("OC_TEST_CLICKHOUSE_URL or OC_TEST_POSTGRES_URL unset: skipping");
        return None;
    };
    let tables = || oc_store::ClickHouseConfig {
        url: ch_url.clone(),
        user: std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into()),
        password: std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default(),
        database: "oc".into(),
        cluster: None,
    };
    migrate_clickhouse_once(&tables()).await;
    let postgres = oc_store::postgres(&pg_url, std::time::Duration::from_secs(5)).await.expect("postgres");
    PG_MIGRATED.get_or_init(|| async { oc_store::migrate_postgres(&postgres).await.expect("migrate postgres") }).await;
    // Live tail only: never connected in these tests.
    let nats = async_nats::ConnectOptions::new().retry_on_initial_connect().connect("nats://127.0.0.1:1").await.expect("nats client");
    let backend = Arc::new(FakeBackend::default());
    let states = (0..count)
        .map(|_| {
            AppState(Arc::new(Inner {
                config: crate::config::Config::from_env(),
                clickhouse: oc_store::clickhouse(&tables()),
                tables: tables(),
                postgres: postgres.clone(),
                nats: nats.clone(),
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
