use std::env;

/// Process configuration, read from `OC_*` environment variables (see `.env.example`).
#[derive(Clone, Debug)]
pub struct Config {
    pub http_addr: String,
    pub clickhouse_url: String,
    pub clickhouse_user: String,
    pub clickhouse_password: String,
    /// ClickHouse cluster to create replicated, sharded tables on (unset: a single server).
    pub clickhouse_cluster: Option<String>,
    /// Replace a pre-0001_schema ClickHouse schema (deletes its telemetry).
    pub clickhouse_drop_legacy: bool,
    /// Apply migrations at startup (off when a deploy step runs `observecraft-server migrate`).
    pub run_migrations: bool,
    /// Limits on each query an org runs, so one org can't starve the others.
    pub query_timeout_secs: u32,
    pub query_max_memory_bytes: u64,
    pub postgres_url: String,
    pub nats_url: String,
    /// Built web app to serve (e.g. `../web/dist`); unset in development (Vite serves it).
    pub web_dist: Option<String>,
    pub cors_origins: Vec<String>,
    /// The organization created at install time (instance admin's home org, bootstrap admin, demo data).
    pub dev_org: String,
    /// The first admin (seeded by migration 0002). Gets a one-time set-password link while nobody can sign in.
    pub dev_user: String,
    /// Account that administers the install (creates and manages organizations). Lowercased.
    pub instance_admin_email: Option<String>,
    /// Encrypts secrets at rest (TOTP seeds, OIDC client secrets). Required in production.
    pub secret_key: String,
    /// Origin users reach the web app on (SSO redirect URLs, emailed links). No trailing slash.
    pub public_url: String,
    /// `Secure` cookie flag: enable whenever the app is served over HTTPS.
    pub cookie_secure: bool,
    /// Idle session lifetime; sessions also expire 7 days after sign-in.
    pub session_hours: i64,
    /// Trust `X-Forwarded-For` for client addresses (only behind your own reverse proxy).
    pub trust_proxy: bool,
    /// Install demo users, teams and incidents into an empty org (development only).
    pub seed_demo: bool,
    /// With `seed_demo`: an ingest key of the install's org for the local data generators.
    pub demo_ingest_key: Option<String>,
    /// With `seed_demo`: a local S3-compatible store (RustFS) the install's org archives logs to.
    pub demo_archive: Option<DemoArchive>,
    /// Local working files (archive download cache, search scratch space).
    pub data_dir: String,
    /// Days telemetry stays in ClickHouse (applied as table TTLs at startup).
    pub retention: Retention,
    /// Internal listener of the log indexer (Vector posts every log there).
    pub logproc_addr: String,
    /// Optional bearer token Vector must send to the log indexer.
    pub internal_token: Option<String>,
    /// SMTP server for monitor notification emails (`smtps://user:pass@host:465`, `smtp://host:25?tls=required`).
    pub smtp_url: Option<String>,
    /// Sender address of notification emails.
    pub smtp_from: String,
}

#[derive(Clone, Copy, Debug)]
pub struct Retention {
    /// Retention of a new org's first log index.
    pub logs_days: u32,
    /// Span retention of orgs that haven't chosen one.
    pub traces_days: u32,
    /// Ceilings the instance admin can grant orgs (log indexes / spans).
    pub logs_max_days: u32,
    pub traces_max_days: u32,
    /// Raw metric samples, and the 5-minute and 1-hour rollups, of orgs that haven't chosen.
    pub metrics_days: u32,
    pub metrics_5m_days: u32,
    pub metrics_1h_days: u32,
    pub ci_days: u32,
    /// Hours every ingested log stays in the raw table (archive source).
    pub raw_logs_hours: u32,
}

fn days(key: &str, default: u32) -> u32 {
    var(key, &default.to_string()).parse::<u32>().unwrap_or(default).clamp(1, 3650)
}

#[derive(Clone, Debug)]
pub struct DemoArchive {
    pub endpoint: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
}

fn var(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            http_addr: var("OC_HTTP_ADDR", "0.0.0.0:8080"),
            clickhouse_url: var("OC_CLICKHOUSE_URL", "http://localhost:8123"),
            clickhouse_user: var("OC_CLICKHOUSE_USER", "observecraft"),
            clickhouse_password: var("OC_CLICKHOUSE_PASSWORD", "observecraft"),
            clickhouse_cluster: env::var("OC_CLICKHOUSE_CLUSTER").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            clickhouse_drop_legacy: var("OC_CLICKHOUSE_DROP_LEGACY_SCHEMA", "false") == "true",
            run_migrations: var("OC_RUN_MIGRATIONS", "true") != "false",
            query_timeout_secs: var("OC_QUERY_TIMEOUT_SECONDS", "60").parse().unwrap_or(60).max(1),
            query_max_memory_bytes: var("OC_QUERY_MAX_MEMORY_BYTES", "4294967296").parse().unwrap_or(4 << 30),
            postgres_url: var("OC_POSTGRES_URL", "postgres://observecraft:observecraft@localhost:5432/observecraft"),
            nats_url: var("OC_NATS_URL", "nats://localhost:4222"),
            web_dist: env::var("OC_WEB_DIST").ok().filter(|s| !s.is_empty()),
            cors_origins: var("OC_CORS_ORIGINS", "http://localhost:5173")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            dev_org: var("OC_DEV_ORG", "default"),
            dev_user: var("OC_DEV_USER", "00000000-0000-0000-0000-000000000001"),
            instance_admin_email: env::var("OC_INSTANCE_ADMIN_EMAIL").ok().map(|s| s.trim().to_lowercase()).filter(|s| s.contains('@')),
            secret_key: env::var("OC_SECRET_KEY").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
                tracing::warn!("OC_SECRET_KEY is not set: using an insecure development key. Set it in production.");
                "observecraft-insecure-development-key".into()
            }),
            public_url: var("OC_PUBLIC_URL", "http://localhost:5173").trim_end_matches('/').to_string(),
            cookie_secure: var("OC_COOKIE_SECURE", "false") == "true",
            session_hours: var("OC_SESSION_HOURS", "12").parse().unwrap_or(12).clamp(1, 168),
            trust_proxy: var("OC_TRUST_PROXY", "false") == "true",
            seed_demo: var("OC_SEED_DEMO", "true") == "true",
            demo_archive: env::var("OC_DEMO_ARCHIVE_ENDPOINT").ok().filter(|e| !e.is_empty() && var("OC_SEED_DEMO", "true") == "true").map(|endpoint| DemoArchive {
                endpoint,
                bucket: var("OC_DEMO_ARCHIVE_BUCKET", "oc-archives"),
                access_key: var("OC_DEMO_ARCHIVE_ACCESS_KEY", "observecraft"),
                secret_key: var("OC_DEMO_ARCHIVE_SECRET_KEY", "observecraft-archive-dev"),
            }),
            data_dir: var("OC_DATA_DIR", ".oc-data"),
            logproc_addr: var("OC_LOGPROC_ADDR", "0.0.0.0:8090"),
            internal_token: env::var("OC_INTERNAL_TOKEN").ok().filter(|s| !s.is_empty()),
            smtp_url: env::var("OC_SMTP_URL").ok().filter(|s| !s.is_empty()),
            smtp_from: var("OC_SMTP_FROM", "ObserveCraft <alerts@observecraft.local>"),
            retention: Retention {
                logs_days: days("OC_RETENTION_LOGS_DAYS", 15),
                traces_days: days("OC_RETENTION_TRACES_DAYS", 15),
                metrics_days: days("OC_RETENTION_METRICS_DAYS", 30),
                metrics_5m_days: days("OC_RETENTION_METRICS_5M_DAYS", 90),
                metrics_1h_days: days("OC_RETENTION_METRICS_1H_DAYS", 395),
                ci_days: days("OC_RETENTION_CI_DAYS", 30),
                logs_max_days: days("OC_RETENTION_LOGS_MAX_DAYS", 365),
                traces_max_days: days("OC_RETENTION_TRACES_MAX_DAYS", 90),
                raw_logs_hours: var("OC_RAW_LOGS_HOURS", "72").parse::<u32>().unwrap_or(72).clamp(2, 24 * 30),
            },
            demo_ingest_key: env::var("OC_DEMO_INGEST_KEY").ok().filter(|k| k.starts_with("ocik_") && k.len() >= 20 && var("OC_SEED_DEMO", "true") == "true"),
        }
    }
}
