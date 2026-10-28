//! ObserveCraft API server.
//!
//! `observecraft-server`          run the server (config from OC_* env vars)
//! `observecraft-server migrate`  apply migrations and built-in content, then exit (a deploy step;
//!                                set OC_RUN_MIGRATIONS=false on the servers then)
//! `observecraft-server openapi`  print the OpenAPI document (used to generate the web app's types)

mod auth;
mod config;
mod demo;
mod error;
mod log_access;
mod log_metrics;
mod logproc;
mod pipelines;
mod routes;
mod s3;
mod standard_attributes;
mod state;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::http::{header, HeaderValue, Method};
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;

use crate::config::Config;
use crate::state::{AppState, Inner};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("openapi") {
        println!("{}", routes::ApiDoc::openapi().to_pretty_json()?);
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")),
        )
        .init();

    let cfg = Config::from_env();
    let migrate_only = std::env::args().nth(1).as_deref() == Some("migrate");
    let ch_cfg = oc_store::ClickHouseConfig {
        url: cfg.clickhouse_url.clone(),
        user: cfg.clickhouse_user.clone(),
        password: cfg.clickhouse_password.clone(),
        database: "oc".into(),
        cluster: cfg.clickhouse_cluster.clone(),
    };
    let postgres = oc_store::postgres(&cfg.postgres_url, STARTUP_WAIT).await?;
    // Several servers may start at once (replicas, a rolling upgrade): one at a time migrates and
    // seeds, the others wait here and then find nothing left to do.
    let mut setup_lock = postgres.acquire().await?;
    sqlx::query("SELECT pg_advisory_lock(7300)")
        .execute(&mut *setup_lock)
        .await?;
    if cfg.run_migrations || migrate_only {
        // Retried while the databases settle (e.g. a new ClickHouse cluster before Keeper has a quorum).
        let start = std::time::Instant::now();
        while let Err(e) = migrate(&cfg, &ch_cfg, &postgres).await {
            if start.elapsed() > STARTUP_WAIT {
                return Err(e);
            }
            tracing::warn!(error = format!("{e:#}"), "migrations failed, retrying");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
    if migrate_only {
        tracing::info!("migrations applied");
        return Ok(());
    }
    let nats = async_nats::ConnectOptions::new()
        .name("observecraft-server")
        .connection_timeout(Duration::from_secs(5))
        // Live tail only: start without it and keep reconnecting.
        .retry_on_initial_connect()
        .connect(&cfg.nats_url)
        .await
        .context("connect nats")?;

    let secrets = auth::crypto::SecretBox::new(&cfg.secret_key);
    let state = AppState(Arc::new(Inner {
        clickhouse: oc_store::clickhouse(&ch_cfg),
        tables: ch_cfg,
        postgres,
        nats,
        secrets,
        log_restrictions: Default::default(),
        config: cfg.clone(),
    }));
    routes::auth::ensure_instance_admin(&state)
        .await
        .context("instance admin")?;
    routes::auth::bootstrap_admin(&state)
        .await
        .context("bootstrap admin")?;
    routes::ingest_keys::ensure_demo_key(&state)
        .await
        .context("demo ingest key")?;

    // Expired sessions, one-time links, SSO requests and replay records.
    let pg = state.postgres.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(600));
        loop {
            tick.tick().await;
            auth::sweep(&pg).await;
        }
    });

    if state.config.seed_demo {
        demo::install_monitors(&state, &state.config.dev_org)
            .await
            .context("install demo monitors")?;
    }
    tokio::spawn(routes::monitors::run_evaluator(state.clone()));
    routes::archives::ensure_demo_archive(&state)
        .await
        .context("demo log archive")?;
    sqlx::query("SELECT pg_advisory_unlock(7300)")
        .execute(&mut *setup_lock)
        .await?;
    drop(setup_lock);
    tokio::spawn(routes::archives::run_archiver(state.clone()));
    tokio::spawn(routes::notifications::run_notifier(state.clone()));
    tokio::spawn(routes::on_call::run_escalations(state.clone()));
    tokio::spawn(routes::synthetics::run_scheduler(state.clone()));
    tokio::spawn(routes::scorecards::run_scheduler(state.clone()));
    tokio::spawn(routes::resource_policies::run_snapshots(state.clone()));
    logproc::serve(state.clone()).await.context("log indexer")?;

    let cors = CorsLayer::new()
        .allow_origin(
            cfg.cors_origins
                .iter()
                .filter_map(|o| o.parse::<HeaderValue>().ok())
                .collect::<Vec<_>>(),
        )
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .allow_credentials(true);

    let mut app = routes::router()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require,
        ))
        .with_state(state);
    if let Some(dist) = &cfg.web_dist {
        // Serve the built SPA; unknown paths fall back to index.html for client-side routing.
        let index = format!("{dist}/index.html");
        app = app.fallback_service(ServeDir::new(dist).fallback(ServeFile::new(index)));
    }
    let app = app
        .layer(cors)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&cfg.http_addr)
        .await
        .with_context(|| format!("bind {}", cfg.http_addr))?;
    tracing::info!(addr = %cfg.http_addr, "observecraft-server listening");
    // Peer addresses feed the audit trail and session list.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    Ok(())
}

/// How long startup waits for Postgres and ClickHouse to become reachable.
const STARTUP_WAIT: Duration = Duration::from_secs(300);

/// Schema migrations (ClickHouse, Postgres), table retention and built-in content. Idempotent (safe
/// to retry after a partial run); the caller holds the setup lock.
async fn migrate(
    cfg: &Config,
    ch_cfg: &oc_store::ClickHouseConfig,
    postgres: &sqlx::PgPool,
) -> anyhow::Result<()> {
    oc_store::wait_clickhouse(ch_cfg, STARTUP_WAIT).await?;
    oc_store::migrate_clickhouse(ch_cfg, cfg.clickhouse_drop_legacy)
        .await
        .context("clickhouse migrations")?;
    let r = cfg.retention;
    oc_store::apply_retention(
        ch_cfg,
        oc_store::RetentionDays {
            traces: r.traces_days,
            traces_max: r.traces_max_days,
            metrics: r.metrics_days,
            metrics_5m: r.metrics_5m_days,
            metrics_1h: r.metrics_1h_days,
            ci: r.ci_days,
            raw_logs_hours: r.raw_logs_hours,
            metrics_cold_after_days: r.metrics_cold_after_days,
        },
    )
    .await
    .context("retention")?;
    oc_store::migrate_postgres(postgres).await?;
    routes::dashboards::install_seeds(postgres, &cfg.dev_org, &cfg.dev_user)
        .await
        .context("install built-in dashboards")?;
    // Other orgs get the same refreshed integration dashboards (custom seeds are only added once).
    let others: Vec<(String, String)> = sqlx::query_as(
        "SELECT o.id, (SELECT u.id::text FROM users u WHERE u.org_id = o.id ORDER BY u.created_at LIMIT 1) FROM orgs o \
         WHERE o.id <> $1 AND EXISTS (SELECT 1 FROM users u WHERE u.org_id = o.id)",
    )
    .bind(&cfg.dev_org)
    .fetch_all(postgres)
    .await?;
    for (org, user) in others {
        routes::dashboards::install_seeds(postgres, &org, &user)
            .await
            .with_context(|| format!("install built-in dashboards for {org}"))?;
    }
    if cfg.seed_demo {
        demo::install(postgres, &cfg.dev_org)
            .await
            .context("install demo data")?;
    }
    Ok(())
}

async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutting down");
}
