//! Storage connections and schema migrations.
//!
//! - ClickHouse holds telemetry (logs today; traces/metrics come from the OTel Collector's exporter).
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
