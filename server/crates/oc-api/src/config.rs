use std::env;

/// Process configuration, read from `OC_*` environment variables (see `.env.example`).
#[derive(Clone, Debug)]
pub struct Config {
    pub http_addr: String,
    pub clickhouse_url: String,
    pub clickhouse_user: String,
    pub clickhouse_password: String,
    pub postgres_url: String,
    pub nats_url: String,
    /// Built web app to serve (e.g. `../web/dist`); unset in development (Vite serves it).
    pub web_dist: Option<String>,
    pub cors_origins: Vec<String>,
    /// The organization created at install time (built-in dashboards, bootstrap admin, demo data).
    pub dev_org: String,
    /// The first admin (seeded by migration 0002). Gets a one-time set-password link while nobody can sign in.
    pub dev_user: String,
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
            secret_key: env::var("OC_SECRET_KEY").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
                tracing::warn!("OC_SECRET_KEY is not set: using an insecure development key. Set it in production.");
                "observecraft-insecure-development-key".into()
            }),
            public_url: var("OC_PUBLIC_URL", "http://localhost:5173").trim_end_matches('/').to_string(),
            cookie_secure: var("OC_COOKIE_SECURE", "false") == "true",
            session_hours: var("OC_SESSION_HOURS", "12").parse().unwrap_or(12).clamp(1, 168),
            trust_proxy: var("OC_TRUST_PROXY", "false") == "true",
            seed_demo: var("OC_SEED_DEMO", "true") == "true",
        }
    }
}
