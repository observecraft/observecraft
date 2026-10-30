//! Storage connections and schema migrations.
//!
//! - ClickHouse holds telemetry: logs (`oc.logs`), and traces/metrics in `otel.*` (written by the OTel
//!   Collector's exporter, schemas owned here). Every row carries its organization.
//! - Postgres holds control-plane metadata (dashboards, monitors, users, …).

use anyhow::Context;
use clickhouse::Client;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct ClickHouseConfig {
    pub url: String,
    pub user: String,
    pub password: String,
    pub database: String,
    /// ClickHouse cluster (from `remote_servers`) to create replicated, sharded tables on; `None`
    /// for a single server.
    pub cluster: Option<String>,
}

impl ClickHouseConfig {
    /// The table that physically holds `db.table`'s rows on each server: `db.table_local` in a
    /// cluster (where `db.table` is the Distributed table over it), `db.table` otherwise.
    /// DDL and mutations (ALTER, DROP PARTITION) target this; reads and inserts use `db.table`.
    pub fn local(&self, table: &str) -> String {
        match self.cluster {
            Some(_) => format!("{table}_local"),
            None => table.to_string(),
        }
    }

    /// ` ON CLUSTER '<cluster>'`, or nothing on a single server.
    pub fn on_cluster(&self) -> String {
        self.cluster
            .as_ref()
            .map(|c| format!(" ON CLUSTER '{c}'"))
            .unwrap_or_default()
    }

    fn admin(&self) -> Client {
        Client::default()
            .with_url(&self.url)
            .with_user(&self.user)
            .with_password(&self.password)
    }
}

/// Ordered, embedded ClickHouse migrations. Each must be idempotent; see 0001_schema.sql for the
/// placeholders [`render`] fills in.
const CH_MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_schema",
        include_str!("../../../migrations/clickhouse/0001_schema.sql"),
    ),
    (
        "0002_events",
        include_str!("../../../migrations/clickhouse/0002_events.sql"),
    ),
    (
        "0003_network",
        include_str!("../../../migrations/clickhouse/0003_network.sql"),
    ),
    (
        "0004_profiles",
        include_str!("../../../migrations/clickhouse/0004_profiles.sql"),
    ),
    (
        "0005_profile_totals",
        include_str!("../../../migrations/clickhouse/0005_profile_totals.sql"),
    ),
    (
        "0006_metrics_storage",
        include_str!("../../../migrations/clickhouse/0006_metrics_storage.sql"),
    ),
    (
        "0007_log_error_keys",
        include_str!("../../../migrations/clickhouse/0007_log_error_keys.sql"),
    ),
    (
        "0008_k8s_generations",
        include_str!("../../../migrations/clickhouse/0008_k8s_generations.sql"),
    ),
    (
        "0009_metrics_write_path",
        include_str!("../../../migrations/clickhouse/0009_metrics_write_path.sql"),
    ),
    (
        "0010_metric_tags",
        include_str!("../../../migrations/clickhouse/0010_metric_tags.sql"),
    ),
    (
        "0011_k8s_inventory",
        include_str!("../../../migrations/clickhouse/0011_k8s_inventory.sql"),
    ),
    (
        "0012_log_aggregates",
        include_str!("../../../migrations/clickhouse/0012_log_aggregates.sql"),
    ),
    (
        "0013_resource_changes",
        include_str!("../../../migrations/clickhouse/0013_resource_changes.sql"),
    ),
    (
        "0014_cloud_costs",
        include_str!("../../../migrations/clickhouse/0014_cloud_costs.sql"),
    ),
];

/// Migrations of the schema before 0001_schema (no org-first sort keys, no cluster support).
const LEGACY_MIGRATIONS: &[&str] = &["0001_logs", "0003_otel_tenancy", "0007_kubernetes"];

/// Tables and views of the legacy schema, dropped when it is replaced.
const LEGACY_OBJECTS: &[&str] = &[
    "otel.otel_traces_trace_id_ts_mv",
    "otel.otel_ci_traces_trace_id_ts_mv",
    "otel.k8s_resources_mv",
    "otel.k8s_events_mv",
    "oc.logs",
    "oc.logs_raw",
    "oc.logs_rehydrated",
    "otel.otel_traces",
    "otel.otel_ci_traces",
    "otel.otel_traces_trace_id_ts",
    "otel.otel_ci_traces_trace_id_ts",
    "otel.otel_metrics_gauge",
    "otel.otel_metrics_sum",
    "otel.otel_metrics_histogram",
    "otel.otel_metrics_exponential_histogram",
    "otel.otel_metrics_summary",
    "otel.otel_k8s_objects",
    "otel.k8s_resources",
    "otel.k8s_events",
];

/// The server's client for reads and inserts. In a cluster, a join or `IN` subquery over a
/// Distributed table inside a query of another one runs as `GLOBAL` (the subquery once, its result
/// sent to every shard), which is right whatever the two tables are sharded by. Readers whose
/// tables share a shard key (series, tags, samples and rollups by SeriesId) set
/// `distributed_product_mode = 'local'` to join on each shard instead.
pub fn clickhouse(cfg: &ClickHouseConfig) -> Client {
    configure(Client::default(), cfg)
}

/// Sets up `client` as [`clickhouse`] does, e.g. one made with its own HTTP client
/// (`Client::with_http_client`).
pub fn configure(client: Client, cfg: &ClickHouseConfig) -> Client {
    let client = client.with_url(&cfg.url).with_user(&cfg.user).with_password(&cfg.password).with_database(&cfg.database);
    match cfg.cluster {
        Some(_) => client.with_option("distributed_product_mode", "global"),
        None => client,
    }
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct Applied {
    name: String,
}

/// Fills in a migration's placeholders for a single server (`cluster` None) or a cluster.
pub fn render(sql: &str, cluster: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    for statement in strip_comments(sql)
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(spec) = statement
            .strip_prefix("${DISTRIBUTED ")
            .and_then(|s| s.strip_suffix('}'))
        {
            // ${DISTRIBUTED db.table sharding_key}
            let (table, key) = spec.split_once(' ').unwrap_or((spec, "rand()"));
            if let Some(c) = cluster {
                let (db, name) = table.split_once('.').unwrap_or(("default", table));
                out.push(format!("CREATE TABLE IF NOT EXISTS {table} ON CLUSTER '{c}' AS {table}_local ENGINE = Distributed('{c}', '{db}', '{name}_local', {key})"));
            }
            continue;
        }
        let rendered = match cluster {
            Some(c) => statement
                .replace("${ON_CLUSTER}", &format!("ON CLUSTER '{c}'"))
                .replace("${LOCAL}", "_local")
                .replace("${REPLICATED}", "Replicated"),
            None => statement
                .replace("${ON_CLUSTER}", "")
                .replace("${LOCAL}", "")
                .replace("${REPLICATED}", ""),
        };
        out.push(rendered);
    }
    out
}

/// Waits up to `wait` for ClickHouse to answer.
pub async fn wait_clickhouse(cfg: &ClickHouseConfig, wait: Duration) -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    loop {
        match cfg.admin().query("SELECT 1").execute().await {
            Ok(()) => return Ok(()),
            Err(e) if start.elapsed() < wait => {
                tracing::warn!(error = %e, "clickhouse not reachable yet, retrying");
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            Err(e) => return Err(e).context("connect clickhouse"),
        }
    }
}

/// Applies pending ClickHouse migrations and records them in `oc.schema_migrations`.
///
/// An install still on the legacy schema is replaced only when `drop_legacy` is set (its telemetry
/// is lost); otherwise this fails, so data is never dropped by surprise.
pub async fn migrate_clickhouse(cfg: &ClickHouseConfig, drop_legacy: bool) -> anyhow::Result<()> {
    let admin = cfg.admin();
    let on = cfg.on_cluster();
    admin
        .query(&format!("CREATE DATABASE IF NOT EXISTS oc{on}"))
        .execute()
        .await
        .context("create database oc")?;
    // In a cluster every server must see the same history: one replicated table shared by all
    // shards (a fixed replica path, not per shard).
    let engine = match cfg.cluster {
        Some(_) => "ReplicatedMergeTree('/clickhouse/observecraft/schema_migrations', '{replica}')",
        None => "MergeTree",
    };
    admin
        .query(&format!("CREATE TABLE IF NOT EXISTS oc.schema_migrations{on} (name String, applied_at DateTime DEFAULT now()) ENGINE = {engine} ORDER BY name"))
        .execute()
        .await
        .context("create schema_migrations")?;
    let mut applied: Vec<String> = admin
        .query("SELECT name FROM oc.schema_migrations")
        .fetch_all::<Applied>()
        .await?
        .into_iter()
        .map(|a| a.name)
        .collect();

    if applied
        .iter()
        .any(|a| LEGACY_MIGRATIONS.contains(&a.as_str()))
    {
        anyhow::ensure!(
            drop_legacy,
            "ClickHouse has the pre-0001_schema telemetry schema, which isn't sorted by organization. \
             Set OC_CLICKHOUSE_DROP_LEGACY_SCHEMA=true to drop it and create the new schema (stored telemetry is deleted)."
        );
        for object in LEGACY_OBJECTS {
            admin
                .query(&format!("DROP TABLE IF EXISTS {object}{on} SYNC"))
                .execute()
                .await
                .with_context(|| format!("drop legacy {object}"))?;
        }
        admin
            .query(&format!("TRUNCATE TABLE oc.schema_migrations{on}"))
            .execute()
            .await?;
        applied.clear();
        tracing::warn!("dropped the legacy ClickHouse schema");
    }

    for (name, sql) in CH_MIGRATIONS {
        if applied.iter().any(|a| a == name) {
            continue;
        }
        for statement in render(sql, cfg.cluster.as_deref()) {
            // `?` is the client's bind placeholder; DDL contains none, so escape defensively.
            admin
                .query(&statement.replace('?', "??"))
                .execute()
                .await
                .with_context(|| format!("clickhouse migration {name}: {statement}"))?;
        }
        admin
            .query("INSERT INTO oc.schema_migrations (name) VALUES (?)")
            .bind(*name)
            .execute()
            .await?;
        tracing::info!(
            migration = name,
            cluster = cfg.cluster.as_deref().unwrap_or("-"),
            "applied clickhouse migration"
        );
    }
    Ok(())
}

fn strip_comments(s: &str) -> String {
    s.lines().filter(|l| !l.trim_start().starts_with("--")).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Connects to Postgres, waiting up to `wait` for it to come up (in Kubernetes the database may
/// start after the server).
pub async fn postgres(url: &str, wait: Duration) -> anyhow::Result<PgPool> {
    let start = std::time::Instant::now();
    loop {
        match PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await
        {
            Ok(pool) => return Ok(pool),
            Err(e) if start.elapsed() < wait => {
                tracing::warn!(error = %e, "postgres not reachable yet, retrying");
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            Err(e) => return Err(e).context("connect postgres"),
        }
    }
}

/// Applies pending Postgres migrations.
pub async fn migrate_postgres(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("../../migrations/postgres")
        .run(pool)
        .await
        .context("postgres migrations")
}

/// How long each kind of telemetry is kept.
#[derive(Clone, Copy, Debug)]
pub struct RetentionDays {
    /// Spans of orgs that didn't choose a retention.
    pub traces: u32,
    /// Longest span retention any org may choose (the trace-id lookup table keeps this long).
    pub traces_max: u32,
    /// Raw metric samples, and the 5-minute and 1-hour rollups, of orgs that didn't choose.
    pub metrics: u32,
    pub metrics_5m: u32,
    pub metrics_1h: u32,
    pub ci: u32,
    /// Hours every ingested log stays in `oc.logs_raw` (archives read it).
    pub raw_logs_hours: u32,
    /// Days after which metric samples and rollups move to the `cold` volume of the
    /// [`COLD_STORAGE_POLICY`] (object storage); 0 = no cold storage.
    pub metrics_cold_after_days: u32,
}

/// Storage policy with a `default` (local) and a `cold` (object storage) volume, declared in the
/// ClickHouse server config when cold storage is on (the Helm chart's `clickhouse.bundled.coldStorage`).
pub const COLD_STORAGE_POLICY: &str = "observecraft_tiered";

/// The metric storage tables that move to cold storage (not the series table: small, read by every query).
const COLD_TABLES: &[&str] = &[
    "otel.metrics_samples",
    "otel.metrics_rollup_5m",
    "otel.metrics_rollup_1h",
    "otel.metrics_dist_samples",
    "otel.metrics_dist_rollup_5m",
    "otel.metrics_dist_rollup_1h",
];

/// Each table's TTL expression for `r`.
fn retention_ttls(r: RetentionDays) -> Vec<(&'static str, String)> {
    let mut ttls = delete_ttls(r);
    if r.metrics_cold_after_days > 0 {
        for (_, ttl) in ttls.iter_mut().filter(|(t, _)| COLD_TABLES.contains(t)) {
            // The same time column as the delete TTL (TimeUnix or Bucket).
            let time = ttl.split(" + ").next().unwrap_or_default().to_string();
            *ttl = format!("{time} + toIntervalDay({}) TO VOLUME 'cold', {ttl}", r.metrics_cold_after_days);
        }
    }
    ttls
}

/// Each table's delete TTL for `r`.
fn delete_ttls(r: RetentionDays) -> Vec<(&'static str, String)> {
    // Rehydrated logs (oc.logs_rehydrated) expire with their rehydration, not by age.
    vec![
        ("oc.logs", "toDateTime(timestamp) + toIntervalDay(retention_days)".into()),
        ("oc.logs_raw", format!("toDateTime(timestamp) + INTERVAL {} HOUR", r.raw_logs_hours)),
        ("otel.otel_traces", format!("toDate(Timestamp) + toIntervalDay(if(RetentionDays = 0, {}, RetentionDays))", r.traces)),
        ("otel.otel_traces_trace_id_ts", format!("toDate(Start) + INTERVAL {} DAY", r.traces_max)),
        ("otel.otel_ci_traces", format!("toDate(Timestamp) + INTERVAL {} DAY", r.ci)),
        ("otel.otel_ci_traces_trace_id_ts", format!("toDate(Start) + INTERVAL {} DAY", r.ci)),
        ("otel.metrics_samples", format!("toDateTime(TimeUnix) + toIntervalDay(if(RetentionDays = 0, {}, RetentionDays))", r.metrics)),
        ("otel.metrics_rollup_5m", format!("Bucket + toIntervalDay(if(RetentionDays = 0, {}, RetentionDays))", r.metrics_5m)),
        ("otel.metrics_rollup_1h", format!("Bucket + toIntervalDay(if(RetentionDays = 0, {}, RetentionDays))", r.metrics_1h)),
        ("otel.metrics_dist_samples", format!("toDateTime(TimeUnix) + toIntervalDay(if(RetentionDays = 0, {}, RetentionDays))", r.metrics)),
        ("otel.metrics_dist_rollup_5m", format!("Bucket + toIntervalDay(if(RetentionDays = 0, {}, RetentionDays))", r.metrics_5m)),
        ("otel.metrics_dist_rollup_1h", format!("Bucket + toIntervalDay(if(RetentionDays = 0, {}, RetentionDays))", r.metrics_1h)),
        ("otel.metrics_series", format!("LastSeen + toIntervalDay(if(RetentionDays = 0, {}, RetentionDays))", r.metrics.max(r.metrics_5m).max(r.metrics_1h))),
        // The tag index lives as long as its series.
        ("otel.metrics_tags", format!("LastSeen + toIntervalDay(if(RetentionDays = 0, {}, RetentionDays))", r.metrics.max(r.metrics_5m).max(r.metrics_1h))),
        ("otel.metrics_tag_values", format!("LastSeen + toIntervalDay(if(RetentionDays = 0, {}, RetentionDays))", r.metrics.max(r.metrics_5m).max(r.metrics_1h))),
    ]
}

/// Sets the tables' TTLs. Indexed logs, spans and the metrics storage carry their own retention per
/// row (the org's choice); the rest, and rows without a choice, use install-wide values. A changed
/// TTL is recalculated for existing parts without rewriting them; expired parts are dropped at the
/// next merges.
pub async fn apply_retention(cfg: &ClickHouseConfig, r: RetentionDays) -> anyhow::Result<()> {
    let admin = cfg.admin().with_option("materialize_ttl_after_modify", "0");
    let on = cfg.on_cluster();
    for (table, ttl) in retention_ttls(r) {
        let local = cfg.local(table);
        let (db, name) = local.split_once('.').unwrap_or(("default", &local));
        let definition = || {
            admin
                .query("SELECT engine_full FROM system.tables WHERE database = ? AND name = ?")
                .bind(db)
                .bind(name)
                .fetch_one::<String>()
        };
        let before = definition().await.with_context(|| format!("definition of {table}"))?;
        if r.metrics_cold_after_days > 0 && COLD_TABLES.contains(&table) && !before.contains(COLD_STORAGE_POLICY) {
            // Its `default` volume is the default policy's, so existing parts stay where they are.
            admin
                .query(&format!("ALTER TABLE {local}{on} MODIFY SETTING storage_policy = '{COLD_STORAGE_POLICY}'"))
                .execute()
                .await
                .with_context(|| format!("cold storage policy on {table} (is {COLD_STORAGE_POLICY} in the ClickHouse config?)"))?;
        }
        admin
            .query(&format!("ALTER TABLE {local}{on} MODIFY TTL {ttl}"))
            .execute()
            .await
            .with_context(|| format!("set retention of {table}"))?;
        // A new TTL only applies to parts written from now on: recalculate the existing parts' TTL
        // (no rewrite; expired parts are dropped by the next merges). Only when it changed, and in
        // the background so startup doesn't wait on big tables.
        if definition().await? != before {
            admin
                .query(&format!("ALTER TABLE {local}{on} MODIFY SETTING materialize_ttl_recalculate_only = 1"))
                .execute()
                .await
                .with_context(|| format!("recalculate-only TTL on {table}"))?;
            admin
                .query(&format!("ALTER TABLE {local}{on} MATERIALIZE TTL"))
                .execute()
                .await
                .with_context(|| format!("materialize retention of {table}"))?;
            tracing::info!(table, ttl, "retention changed; recalculating existing parts");
        }
    }
    tracing::info!(traces = r.traces, traces_max = r.traces_max, metrics = r.metrics, metrics_5m = r.metrics_5m, metrics_1h = r.metrics_1h, ci = r.ci, raw_logs_hours = r.raw_logs_hours, "retention applied");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = include_str!("../../../migrations/clickhouse/0001_schema.sql");

    /// Every migration's statements.
    fn all(cluster: Option<&str>) -> Vec<String> {
        CH_MIGRATIONS
            .iter()
            .flat_map(|(_, sql)| render(sql, cluster))
            .collect()
    }

    #[test]
    fn single_server_has_no_cluster_parts() {
        let stmts = all(None);
        assert!(stmts.iter().all(|s| !s.contains("${")
            && !s.contains("_local")
            && !s.contains("ON CLUSTER")
            && !s.contains("Replicated")
            && !s.contains("Distributed")));
        assert!(stmts.iter().any(
            |s| s.starts_with("CREATE TABLE IF NOT EXISTS otel.otel_traces")
                && s.contains("ORDER BY (OrgId, ServiceName")
        ));
    }

    #[test]
    fn cluster_has_replicated_locals_and_distributed_tables() {
        let stmts = all(Some("oc"));
        assert!(stmts.iter().all(|s| !s.contains("${")));
        let creates: Vec<&String> = stmts
            .iter()
            .filter(|s| s.starts_with("CREATE TABLE"))
            .collect();
        let name = |s: &str| s.split_whitespace().nth(5).unwrap_or_default().to_string();
        let distributed: Vec<String> = creates
            .iter()
            .filter(|s| s.contains("ENGINE = Distributed('oc'"))
            .map(|s| name(s))
            .collect();
        // Replicated tables are read and written through their Distributed table.
        for local in creates
            .iter()
            .filter(|s| s.contains("_local ON CLUSTER 'oc'") && s.contains("ENGINE = Replicated"))
        {
            let table = name(local).trim_end_matches("_local").to_string();
            assert!(distributed.contains(&table), "no Distributed table for {table}");
        }
        // Every Distributed table is over a shard-local table (Null ones too: the metrics ingest).
        for table in &distributed {
            assert!(
                creates.iter().any(|s| name(s) == format!("{table}_local")),
                "no local table under {table}"
            );
        }
        assert!(distributed.len() >= 15);
        assert!(stmts
            .iter()
            .any(|s| s
                .contains("Distributed('oc', 'otel', 'otel_traces_local', cityHash64(TraceId))")));
        // Materialized views read and write the shard-local tables.
        for mv in stmts.iter().filter(|s| s.contains("MATERIALIZED VIEW")) {
            assert!(
                mv.contains("ON CLUSTER 'oc'") && mv.contains("_local"),
                "{mv}"
            );
            assert!(
                !mv.contains("FROM otel.otel_traces\n")
                    && !mv.contains("FROM otel.otel_k8s_objects\n"),
                "{mv}"
            );
        }
    }

    /// The tables and views that exist after every migration ran, as (name, statement), replaying
    /// CREATE and DROP statements in order.
    fn final_objects(cluster: Option<&str>) -> Vec<(String, String)> {
        let mut objects: Vec<(String, String)> = Vec::new();
        for s in all(cluster) {
            let words: Vec<&str> = s.split_whitespace().collect();
            let name = match words.as_slice() {
                ["CREATE", "TABLE", "IF", "NOT", "EXISTS", name, ..] | ["CREATE", "MATERIALIZED", "VIEW", "IF", "NOT", "EXISTS", name, ..] => name.to_string(),
                ["DROP", "TABLE" | "VIEW", "IF", "EXISTS", name, ..] => {
                    objects.retain(|(n, _)| n != name);
                    continue;
                }
                _ => continue,
            };
            if !objects.iter().any(|(n, _)| *n == name) {
                objects.push((name, s.clone()));
            }
        }
        objects
    }

    #[test]
    fn rollups_are_written_by_the_rollup_job_not_by_views() {
        for cluster in [None, Some("oc")] {
            let objects = final_objects(cluster);
            let local = if cluster.is_some() { "_local" } else { "" };
            for table in ["metrics_rollup_5m", "metrics_rollup_1h", "metrics_dist_rollup_5m", "metrics_dist_rollup_1h"] {
                let name = format!("otel.{table}{local}");
                let (_, create) = objects.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("no {name}"));
                // Plain values; re-running a bucket replaces its rows.
                assert!(create.contains("ReplacingMergeTree"), "{create}");
                assert!(!create.contains("AggregateFunction"), "{create}");
                assert!(create.contains("ORDER BY (OrgId, MetricName, SeriesId, Bucket)"), "{create}");
                assert!(
                    !objects.iter().any(|(_, s)| s.contains("MATERIALIZED VIEW") && s.contains(&format!(" TO {name} "))),
                    "a view still writes {name}"
                );
            }
            // The 5-minute rollups keep the 1-hour tier their hours are rolled up with.
            let (_, five) = objects.iter().find(|(n, _)| *n == format!("otel.metrics_rollup_5m{local}")).unwrap();
            assert!(five.contains("`Retention1hDays` UInt16"), "{five}");
            // Samples carry every tier, for the rollup job.
            assert!(all(cluster).iter().any(|s| s.contains(&format!("ALTER TABLE otel.metrics_samples{local}")) && s.contains("Retention5mDays")));
        }
    }

    #[test]
    fn series_views_skip_points_of_series_the_intake_marked_known() {
        for cluster in [None, Some("oc")] {
            let objects = final_objects(cluster);
            for view in ["otel.metrics_series_mv", "otel.metrics_dist_series_mv"] {
                let (_, mv) = objects.iter().find(|(n, _)| n == view).unwrap_or_else(|| panic!("no {view}"));
                assert!(mv.contains("WHERE NOT SeriesKnown"), "{mv}");
            }
            for view in ["otel.metrics_points_gauge_mv", "otel.metrics_points_sum_mv", "otel.metrics_dist_points_histogram_mv"] {
                let (_, mv) = objects.iter().find(|(n, _)| n == view).unwrap_or_else(|| panic!("no {view}"));
                assert!(mv.contains("src.ResourceAttributes['oc.series_known'] = '1' AS SeriesKnown"), "{mv}");
            }
        }
    }

    #[test]
    fn the_tag_index_is_fed_from_the_series_and_sharded_with_them() {
        for cluster in [None, Some("oc")] {
            let objects = final_objects(cluster);
            let local = if cluster.is_some() { "_local" } else { "" };
            let get = |name: &str| objects.iter().find(|(n, _)| n == name).map(|(_, s)| s.clone()).unwrap_or_else(|| panic!("no {name}"));
            let tags = get(&format!("otel.metrics_tags{local}"));
            assert!(tags.contains("ReplacingMergeTree(LastSeen)") && tags.contains("ORDER BY (OrgId, MetricName, Key, Value, SeriesId)"), "{tags}");
            let values = get(&format!("otel.metrics_tag_values{local}"));
            assert!(values.contains("ReplacingMergeTree(LastSeen)") && values.contains("ORDER BY (OrgId, Key, Value, Scope)"), "{values}");
            // Rows come from the series rows (both label maps and the service), and values from the rows.
            let mv = get("otel.metrics_tags_mv");
            assert!(mv.contains(&format!("TO otel.metrics_tags{local} ")) && mv.contains(&format!("FROM otel.metrics_series{local}")), "{mv}");
            assert!(mv.contains("mapKeys(ResourceAttributes)") && mv.contains("mapKeys(Attributes)") && mv.contains("ServiceName"), "{mv}");
            let mv = get("otel.metrics_tag_values_mv");
            assert!(mv.contains(&format!("FROM otel.metrics_tags{local}")), "{mv}");
            // Existing series get their index rows once.
            assert!(all(cluster).iter().any(|s| s.starts_with("INSERT INTO otel.metrics_tags ") && s.contains("FROM otel.metrics_series")));
        }
        // A series' index rows live on its shard (series → samples joins stay shard-local); a key's
        // values on one shard.
        let stmts = all(Some("oc"));
        assert!(stmts.iter().any(|s| s.contains("Distributed('oc', 'otel', 'metrics_tags_local', SeriesId)")));
        assert!(stmts.iter().any(|s| s.contains("Distributed('oc', 'otel', 'metrics_tag_values_local', cityHash64(OrgId, Key))")));
        let r = RetentionDays { traces: 15, traces_max: 90, metrics: 30, metrics_5m: 90, metrics_1h: 395, ci: 30, raw_logs_hours: 72, metrics_cold_after_days: 0 };
        let ttls = retention_ttls(r);
        for t in ["otel.metrics_tags", "otel.metrics_tag_values"] {
            assert_eq!(ttls.iter().find(|(n, _)| *n == t).map(|(_, ttl)| ttl.as_str()), Some("LastSeen + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))"), "{t}");
        }
    }

    #[test]
    fn the_exporters_metric_tables_are_gone() {
        for cluster in [None, Some("oc")] {
            let objects = final_objects(cluster);
            assert!(objects.iter().all(|(n, _)| !n.starts_with("otel.otel_metrics_")), "{:?}", objects.iter().map(|o| &o.0).filter(|n| n.contains("otel_metrics")).collect::<Vec<_>>());
            // The ingest tables (their copies) stay.
            assert!(objects.iter().any(|(n, _)| n.starts_with("otel.metrics_ingest_gauge")));
        }
        assert!(delete_ttls(RetentionDays { traces: 15, traces_max: 90, metrics: 30, metrics_5m: 90, metrics_1h: 395, ci: 30, raw_logs_hours: 72, metrics_cold_after_days: 0 })
            .iter()
            .all(|(t, _)| !t.starts_with("otel.otel_metrics_")));
    }

    #[test]
    fn every_table_sorts_by_org_first() {
        for s in all(None)
            .iter()
            .filter(|s| s.contains("ORDER BY") && !s.contains("schema_migrations"))
        {
            let order = s.split("ORDER BY").nth(1).unwrap().trim_start();
            assert!(
                order.starts_with("(org_id") || order.starts_with("(OrgId"),
                "not sorted by org first: {}",
                &s[..80.min(s.len())]
            );
        }
    }

    /// The log aggregates (error groups, standard facets): fed by views over the shard-local logs,
    /// partitioned and expired by the logs' own retention, readable through Distributed tables.
    #[test]
    fn log_aggregates_follow_the_logs_on_one_server_and_in_a_cluster() {
        let migration = CH_MIGRATIONS.iter().find(|(name, _)| *name == "0012_log_aggregates").map(|(_, sql)| *sql).expect("0012_log_aggregates is applied");
        for table in ["oc.error_groups_1m", "oc.log_facets_1m"] {
            let single = render(migration, None);
            let create = single.iter().find(|s| s.starts_with(&format!("CREATE TABLE IF NOT EXISTS {table} "))).unwrap_or_else(|| panic!("no {table}"));
            assert!(create.contains("PARTITION BY (toDate(minute), retention_days)"), "{create}");
            assert!(create.contains("TTL minute + toIntervalDay(retention_days)"), "{create}");
            assert!(create.contains("ORDER BY (org_id, minute"), "{create}");
            let mv = single.iter().find(|s| s.starts_with(&format!("CREATE MATERIALIZED VIEW IF NOT EXISTS {table}_mv"))).unwrap_or_else(|| panic!("no view for {table}"));
            assert!(mv.contains(&format!("TO {table} AS")) && mv.contains("FROM oc.logs\n"), "{mv}");

            let cluster = render(migration, Some("oc"));
            assert!(cluster.iter().any(|s| s.starts_with(&format!("CREATE TABLE IF NOT EXISTS {table}_local ON CLUSTER 'oc'")) && s.contains("ENGINE = Replicated")));
            assert!(cluster.iter().any(|s| s.starts_with(&format!("CREATE TABLE IF NOT EXISTS {table} ON CLUSTER 'oc' AS {table}_local ENGINE = Distributed("))));
            let mv = cluster.iter().find(|s| s.starts_with(&format!("CREATE MATERIALIZED VIEW IF NOT EXISTS {table}_mv ON CLUSTER 'oc'"))).unwrap();
            assert!(mv.contains(&format!("TO {table}_local AS")) && mv.contains("FROM oc.logs_local\n"), "{mv}");
        }
    }

    #[test]
    fn kubernetes_inventory_keeps_one_typed_row_per_object() {
        let single = all(None);
        let table = single.iter().find(|s| s.starts_with("CREATE TABLE IF NOT EXISTS otel.k8s_inventory ")).expect("k8s_inventory table");
        assert!(table.contains("ENGINE = ReplacingMergeTree(LastSeen)") && table.contains("ORDER BY (OrgId, Kind, Uid)"), "{table}");
        assert!(table.contains("`Labels` Map(LowCardinality(String), String)"), "{table}");
        let mv = single.iter().find(|s| s.contains("MATERIALIZED VIEW IF NOT EXISTS otel.k8s_inventory_mv")).expect("k8s_inventory_mv");
        assert!(mv.contains("TO otel.k8s_inventory AS") && mv.contains("FROM otel.otel_k8s_objects\n"), "{mv}");

        let cluster = all(Some("oc"));
        assert!(cluster.iter().any(|s| s.starts_with("CREATE TABLE IF NOT EXISTS otel.k8s_inventory_local ON CLUSTER 'oc'") && s.contains("ENGINE = ReplicatedReplacingMergeTree(LastSeen)")));
        // Sharded like the objects it is built from, so an object's snapshots dedupe on one shard.
        assert!(cluster.iter().any(|s| s.contains("ENGINE = Distributed('oc', 'otel', 'k8s_inventory_local', cityHash64(OrgId, Uid, ''))")));
        assert!(cluster.iter().any(|s| s.contains("otel.k8s_inventory_mv ON CLUSTER 'oc' TO otel.k8s_inventory_local AS") && s.contains("FROM otel.otel_k8s_objects_local\n")));
    }

    #[test]
    fn metrics_storage_ttls_default_to_the_install_retention() {
        let r = RetentionDays { traces: 15, traces_max: 90, metrics: 30, metrics_5m: 90, metrics_1h: 395, ci: 30, raw_logs_hours: 72, metrics_cold_after_days: 0 };
        let ttls = retention_ttls(r);
        let ttl = |t: &str| ttls.iter().find(|(table, _)| *table == t).map(|(_, ttl)| ttl.clone()).unwrap_or_else(|| panic!("no TTL for {t}"));
        // Rows without an org choice (RetentionDays = 0) use the install's retention.
        assert_eq!(ttl("otel.metrics_samples"), "toDateTime(TimeUnix) + toIntervalDay(if(RetentionDays = 0, 30, RetentionDays))");
        assert_eq!(ttl("otel.metrics_rollup_5m"), "Bucket + toIntervalDay(if(RetentionDays = 0, 90, RetentionDays))");
        assert_eq!(ttl("otel.metrics_rollup_1h"), "Bucket + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))");
        // Distributions follow the same retention.
        assert_eq!(ttl("otel.metrics_dist_samples"), "toDateTime(TimeUnix) + toIntervalDay(if(RetentionDays = 0, 30, RetentionDays))");
        assert_eq!(ttl("otel.metrics_dist_rollup_5m"), "Bucket + toIntervalDay(if(RetentionDays = 0, 90, RetentionDays))");
        assert_eq!(ttl("otel.metrics_dist_rollup_1h"), "Bucket + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))");
        // A series lives as long as the longest of its data.
        assert_eq!(ttl("otel.metrics_series"), "LastSeen + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))");
    }

    #[test]
    fn metric_storage_moves_to_cold_storage_when_configured() {
        let r = RetentionDays { traces: 15, traces_max: 90, metrics: 30, metrics_5m: 90, metrics_1h: 395, ci: 30, raw_logs_hours: 72, metrics_cold_after_days: 7 };
        let ttls = retention_ttls(r);
        let ttl = |t: &str| ttls.iter().find(|(table, _)| *table == t).map(|(_, ttl)| ttl.clone()).unwrap();
        // Parts older than a week move to the cold volume, then are dropped at their retention.
        assert_eq!(
            ttl("otel.metrics_rollup_1h"),
            "Bucket + toIntervalDay(7) TO VOLUME 'cold', Bucket + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))"
        );
        assert!(ttl("otel.metrics_samples").starts_with("toDateTime(TimeUnix) + toIntervalDay(7) TO VOLUME 'cold', "));
        assert!(ttl("otel.metrics_dist_rollup_5m").contains("TO VOLUME 'cold'"));
        // Not the series table (small, read by every query) nor other telemetry.
        assert!(!ttl("otel.metrics_series").contains("VOLUME"));
        assert!(!ttl("otel.otel_traces").contains("VOLUME"));
        let off = retention_ttls(RetentionDays { metrics_cold_after_days: 0, ..r });
        assert!(off.iter().all(|(_, t)| !t.contains("VOLUME")));
    }

    #[test]
    fn local_names() {
        let mut cfg = ClickHouseConfig {
            url: String::new(),
            user: String::new(),
            password: String::new(),
            database: "oc".into(),
            cluster: None,
        };
        assert_eq!(
            (cfg.local("oc.logs"), cfg.on_cluster()),
            ("oc.logs".to_string(), String::new())
        );
        cfg.cluster = Some("prod".into());
        assert_eq!(
            (cfg.local("oc.logs"), cfg.on_cluster()),
            (
                "oc.logs_local".to_string(),
                " ON CLUSTER 'prod'".to_string()
            )
        );
    }
}
