//! Helpers for tests that run against a live ClickHouse (`OC_TEST_CLICKHOUSE_URL`).

use tokio::sync::OnceCell;

static MIGRATED: OnceCell<()> = OnceCell::const_new();

/// Applies the ClickHouse migrations once per test process. Live tests run in parallel, and on a
/// fresh server their concurrent migration runs raced each other creating the same tables.
pub async fn migrate_clickhouse_once(cfg: &oc_store::ClickHouseConfig) {
    MIGRATED.get_or_init(|| async { oc_store::migrate_clickhouse(cfg, false).await.expect("migrate") }).await;
}
