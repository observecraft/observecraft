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
}

/// Ordered, embedded ClickHouse migrations. Each must be idempotent (also run by the container on first boot).
const CH_MIGRATIONS: &[(&str, &str)] = &[
    ("0001_logs", include_str!("../../../migrations/clickhouse/0001_logs.sql")),
    ("0002_otel_database", include_str!("../../../migrations/clickhouse/0002_otel_database.sql")),
    ("0003_otel_tenancy", include_str!("../../../migrations/clickhouse/0003_otel_tenancy.sql")),
    ("0004_logs_otel", include_str!("../../../migrations/clickhouse/0004_logs_otel.sql")),
    ("0005_logs_rehydrated", include_str!("../../../migrations/clickhouse/0005_logs_rehydrated.sql")),
    ("0006_log_indexes", include_str!("../../../migrations/clickhouse/0006_log_indexes.sql")),
    ("0007_kubernetes", include_str!("../../../migrations/clickhouse/0007_kubernetes.sql")),
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

/// Applies pending ClickHouse migrations and records them in `oc.schema_migrations`.
pub async fn migrate_clickhouse(cfg: &ClickHouseConfig) -> anyhow::Result<()> {
    // Bootstrap without a default database: the first migration creates it.
    let admin = Client::default().with_url(&cfg.url).with_user(&cfg.user).with_password(&cfg.password);
    admin.query("CREATE DATABASE IF NOT EXISTS oc").execute().await.context("create database oc")?;
    admin
        .query("CREATE TABLE IF NOT EXISTS oc.schema_migrations (name String, applied_at DateTime DEFAULT now()) ENGINE = MergeTree ORDER BY name")
        .execute()
        .await
        .context("create schema_migrations")?;
    let applied: Vec<String> = admin
        .query("SELECT name FROM oc.schema_migrations")
        .fetch_all::<Applied>()
        .await?
        .into_iter()
        .map(|a| a.name)
        .collect();

    for (name, sql) in CH_MIGRATIONS {
        if applied.iter().any(|a| a == name) {
            continue;
        }
        // Strip comments before splitting so a ';' inside a comment can't cut a statement.
        for statement in strip_comments(sql).split(';').map(str::trim).filter(|s| !s.is_empty()) {
            // `?` is the client's bind placeholder; DDL contains none, so escape defensively.
            admin.query(&statement.replace('?', "??")).execute().await.with_context(|| format!("clickhouse migration {name}"))?;
        }
        admin.query("INSERT INTO oc.schema_migrations (name) VALUES (?)").bind(*name).execute().await?;
        tracing::info!(migration = name, "applied clickhouse migration");
    }
    Ok(())
}

fn strip_comments(s: &str) -> String {
    s.lines().filter(|l| !l.trim_start().starts_with("--")).collect::<Vec<_>>().join("\n").trim().to_string()
}

pub async fn postgres(url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
        .context("connect postgres")?;
    sqlx::migrate!("../../migrations/postgres").run(&pool).await.context("postgres migrations")?;
    Ok(pool)
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
    let admin = Client::default().with_url(&cfg.url).with_user(&cfg.user).with_password(&cfg.password).with_option("materialize_ttl_after_modify", "0");
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
        admin.query(&format!("ALTER TABLE {table} MODIFY TTL {ttl}")).execute().await.with_context(|| format!("set retention of {table}"))?;
    }
    tracing::info!(traces = r.traces, traces_max = r.traces_max, metrics = r.metrics, ci = r.ci, raw_logs_hours = r.raw_logs_hours, "retention applied");
    Ok(())
}
