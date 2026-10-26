//! Storage connections and schema migrations.
//!
//! - ClickHouse holds telemetry: logs (`oc.logs`), and traces/metrics in `otel.*` (written by the OTel
//!   Collector's exporter, schemas owned here). Every row carries its organization.
//! - Postgres holds control-plane metadata (dashboards, monitors, users, …).

use anyhow::Context;
use clickhouse::Client;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

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

pub fn clickhouse(cfg: &ClickHouseConfig) -> Client {
    Client::default()
        .with_url(&cfg.url)
        .with_user(&cfg.user)
        .with_password(&cfg.password)
        .with_database(&cfg.database)
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
    pub metrics: u32,
    pub ci: u32,
    /// Hours every ingested log stays in `oc.logs_raw` (archives read it).
    pub raw_logs_hours: u32,
}

/// Sets the tables' TTLs. Indexed logs and spans carry their own retention per row (set by the org's
/// log indexes and span retention); the rest use install-wide values. A metadata change only:
/// expired parts are dropped at the next merges instead of rewriting the tables now.
pub async fn apply_retention(cfg: &ClickHouseConfig, r: RetentionDays) -> anyhow::Result<()> {
    let admin = cfg.admin().with_option("materialize_ttl_after_modify", "0");
    // Rehydrated logs (oc.logs_rehydrated) expire with their rehydration, not by age.
    let tables: [(&str, String); 11] = [
        ("oc.logs", "toDateTime(timestamp) + toIntervalDay(retention_days)".into()),
        ("oc.logs_raw", format!("toDateTime(timestamp) + INTERVAL {} HOUR", r.raw_logs_hours)),
        ("otel.otel_traces", format!("toDate(Timestamp) + toIntervalDay(if(RetentionDays = 0, {}, RetentionDays))", r.traces)),
        ("otel.otel_traces_trace_id_ts", format!("toDate(Start) + INTERVAL {} DAY", r.traces_max)),
        ("otel.otel_ci_traces", format!("toDate(Timestamp) + INTERVAL {} DAY", r.ci)),
        ("otel.otel_ci_traces_trace_id_ts", format!("toDate(Start) + INTERVAL {} DAY", r.ci)),
        ("otel.otel_metrics_gauge", format!("toDateTime(TimeUnix) + INTERVAL {} DAY", r.metrics)),
        ("otel.otel_metrics_sum", format!("toDateTime(TimeUnix) + INTERVAL {} DAY", r.metrics)),
        ("otel.otel_metrics_histogram", format!("toDateTime(TimeUnix) + INTERVAL {} DAY", r.metrics)),
        ("otel.otel_metrics_exponential_histogram", format!("toDateTime(TimeUnix) + INTERVAL {} DAY", r.metrics)),
        ("otel.otel_metrics_summary", format!("toDateTime(TimeUnix) + INTERVAL {} DAY", r.metrics)),
    ];
    for (table, ttl) in tables {
        let (local, on) = (cfg.local(table), cfg.on_cluster());
        admin
            .query(&format!("ALTER TABLE {local}{on} MODIFY TTL {ttl}"))
            .execute()
            .await
            .with_context(|| format!("set retention of {table}"))?;
    }
    tracing::info!(traces = r.traces, traces_max = r.traces_max, metrics = r.metrics, ci = r.ci, raw_logs_hours = r.raw_logs_hours, "retention applied");
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
        let locals = creates
            .iter()
            .filter(|s| s.contains("_local ON CLUSTER 'oc'") && s.contains("ENGINE = Replicated"))
            .count();
        let distributed = creates
            .iter()
            .filter(|s| s.contains("ENGINE = Distributed('oc'"))
            .count();
        assert_eq!(
            locals, distributed,
            "every local table has its Distributed table"
        );
        assert!(distributed >= 15);
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
