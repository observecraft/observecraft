//! ObserveCraft telemetry intake.
//!
//! The only public way in for telemetry. Every request must carry an ingest API key
//! (`Authorization: Bearer ocik_…` or `OC-API-Key: ocik_…`); the key is resolved to its organization
//! and the request is forwarded unchanged with `x-oc-org: <org>` (any client-sent value is dropped).
//! The OTel Collector turns that header into the `oc.org_id` resource attribute every table is
//! filtered by; for JSON logs, Vector does the same.
//!
//! - `POST /v1/traces | /v1/logs | /v1/metrics`  OTLP/HTTP (protobuf or JSON, optionally gzip) → Collector
//! - `POST /api/v2/logs`                         JSON log events (object or array) → Vector
//!
//! Each org is rate limited (requests and bytes per second); over the limit answers 429.
//!
//! Env: OC_INTAKE_ADDR (0.0.0.0:4318), OC_POSTGRES_URL, OC_INTAKE_COLLECTOR_URL (http://localhost:14318),
//! OC_INTAKE_LOGS_URL (http://localhost:8687), OC_INTAKE_MAX_BODY_MB (16), OC_INTAKE_ORG_RPS (500),
//! OC_INTAKE_ORG_MBPS (50).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing_subscriber::EnvFilter;

/// Header carrying the resolved organization to the Collector / Vector.
const ORG_HEADER: &str = "x-oc-org";
/// The org's span retention (days), stamped on spans so their table TTL honours it.
const RETENTION_HEADER: &str = "x-oc-retention-days";
/// How long a key → org resolution is trusted (revocations take effect within this).
const KEY_TTL: Duration = Duration::from_secs(60);
/// Unknown keys are remembered briefly so a misconfigured sender can't hammer Postgres.
const MISS_TTL: Duration = Duration::from_secs(10);

struct Config {
    addr: String,
    postgres_url: String,
    collector_url: String,
    logs_url: String,
    max_body: usize,
    org_rps: f64,
    org_bytes_per_sec: f64,
}

fn env(key: &str, default: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

impl Config {
    fn from_env() -> Self {
        Config {
            addr: env("OC_INTAKE_ADDR", "0.0.0.0:4318"),
            postgres_url: env("OC_POSTGRES_URL", "postgres://observecraft:observecraft@localhost:5432/observecraft"),
            collector_url: env("OC_INTAKE_COLLECTOR_URL", "http://localhost:14318").trim_end_matches('/').to_string(),
            logs_url: env("OC_INTAKE_LOGS_URL", "http://localhost:8687").trim_end_matches('/').to_string(),
            max_body: env("OC_INTAKE_MAX_BODY_MB", "16").parse::<usize>().unwrap_or(16) * 1024 * 1024,
            org_rps: env("OC_INTAKE_ORG_RPS", "500").parse().unwrap_or(500.0),
            org_bytes_per_sec: env("OC_INTAKE_ORG_MBPS", "50").parse::<f64>().unwrap_or(50.0) * 1024.0 * 1024.0,
        }
    }
}

/// What a key resolves to: its org and the org's span retention.
type KeyOrg = (String, i32);

struct CachedKey {
    org: Option<KeyOrg>,
    at: Instant,
    /// When `last_used_at` was last written for this key.
    touched: Option<Instant>,
}

/// Token bucket: `rate` per second, bursts up to one second's worth.
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn take(&mut self, n: f64, rate: f64) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.at).as_secs_f64() * rate).min(rate);
        self.at = now;
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }
}

struct Limits {
    requests: Bucket,
    bytes: Bucket,
}

struct Inner {
    cfg: Config,
    pg: PgPool,
    http: reqwest::Client,
    keys: Mutex<HashMap<Vec<u8>, CachedKey>>,
    limits: Mutex<HashMap<String, Limits>>,
}

type Shared = Arc<Inner>;

fn reject(status: StatusCode, msg: &str) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], format!("{{\"error\":\"{msg}\"}}")).into_response()
}

fn api_key(headers: &HeaderMap) -> Option<&str> {
    let bearer = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    let plain = headers.get("oc-api-key").and_then(|v| v.to_str().ok());
    bearer.or(plain).map(str::trim).filter(|k| k.starts_with("ocik_"))
}

/// The org an ingest key belongs to (active key, active org) and its span retention, cached for [`KEY_TTL`].
async fn resolve(st: &Shared, key: &str) -> anyhow::Result<Option<KeyOrg>> {
    let hash = Sha256::digest(key.as_bytes()).to_vec();
    let (cached, touch) = {
        let mut keys = st.keys.lock().unwrap();
        match keys.get_mut(&hash) {
            Some(c) if c.at.elapsed() < if c.org.is_some() { KEY_TTL } else { MISS_TTL } => {
                let touch = c.org.is_some() && c.touched.is_none_or(|t| t.elapsed() > Duration::from_secs(60));
                if touch {
                    c.touched = Some(Instant::now());
                }
                (Some(c.org.clone()), touch)
            }
            _ => (None, false),
        }
    };
    if let Some(org) = cached {
        if touch {
            sqlx::query("UPDATE ingest_keys SET last_used_at = now() WHERE key_hash = $1").bind(&hash).execute(&st.pg).await?;
        }
        return Ok(org);
    }
    let org: Option<KeyOrg> = sqlx::query_as(
        "SELECT k.org_id, COALESCE(s.span_retention_days, 0) FROM ingest_keys k JOIN orgs o ON o.id = k.org_id \
         LEFT JOIN org_data_settings s ON s.org_id = k.org_id WHERE k.key_hash = $1 AND k.revoked_at IS NULL AND o.status = 'active'",
    )
    .bind(&hash)
    .fetch_optional(&st.pg)
    .await?;
    if org.is_some() {
        sqlx::query("UPDATE ingest_keys SET last_used_at = now() WHERE key_hash = $1").bind(&hash).execute(&st.pg).await?;
    }
    st.keys.lock().unwrap().insert(hash, CachedKey { org: org.clone(), at: Instant::now(), touched: org.as_ref().map(|_| Instant::now()) });
    Ok(org)
}

fn within_limits(st: &Shared, org: &str, bytes: usize) -> bool {
    let mut limits = st.limits.lock().unwrap();
    let l = limits.entry(org.to_string()).or_insert_with(|| Limits {
        requests: Bucket { tokens: st.cfg.org_rps, at: Instant::now() },
        bytes: Bucket { tokens: st.cfg.org_bytes_per_sec, at: Instant::now() },
    });
    l.requests.take(1.0, st.cfg.org_rps) && l.bytes.take(bytes as f64, st.cfg.org_bytes_per_sec)
}

/// Authenticates, rate limits and forwards one request body to `target`.
async fn forward(st: Shared, headers: HeaderMap, body: Bytes, target: String) -> Response {
    let Some(key) = api_key(&headers) else {
        return reject(StatusCode::UNAUTHORIZED, "missing API key: send Authorization: Bearer ocik_...");
    };
    let (org, retention) = match resolve(&st, key).await {
        Ok(Some(found)) => found,
        Ok(None) => return reject(StatusCode::UNAUTHORIZED, "invalid or revoked API key"),
        Err(e) => {
            tracing::error!(error = %e, "key lookup failed");
            return reject(StatusCode::SERVICE_UNAVAILABLE, "temporarily unavailable");
        }
    };
    if !within_limits(&st, &org, body.len()) {
        let mut r = reject(StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded for this organization");
        r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        return r;
    }
    let mut req = st.http.post(&target).header(ORG_HEADER, &org).header(RETENTION_HEADER, retention.to_string()).body(body);
    for h in [header::CONTENT_TYPE, header::CONTENT_ENCODING] {
        if let Some(v) = headers.get(&h) {
            req = req.header(h, v);
        }
    }
    match req.send().await {
        Ok(res) => {
            let status = StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let ct = res.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| HeaderValue::from_bytes(v.as_bytes()).ok());
            let bytes = res.bytes().await.unwrap_or_default();
            let mut out = (status, bytes).into_response();
            if let Some(ct) = ct {
                out.headers_mut().insert(header::CONTENT_TYPE, ct);
            }
            out
        }
        Err(e) => {
            tracing::warn!(error = %e, target, "upstream unavailable");
            reject(StatusCode::SERVICE_UNAVAILABLE, "pipeline unavailable, retry later")
        }
    }
}

async fn traces(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let target = format!("{}/v1/traces", st.cfg.collector_url);
    forward(st, headers, body, target).await
}

async fn metrics(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let target = format!("{}/v1/metrics", st.cfg.collector_url);
    forward(st, headers, body, target).await
}

async fn otlp_logs(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let target = format!("{}/v1/logs", st.cfg.collector_url);
    forward(st, headers, body, target).await
}

async fn json_logs(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let target = st.cfg.logs_url.clone();
    forward(st, headers, body, target).await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))).init();
    let cfg = Config::from_env();
    let pg = PgPoolOptions::new().max_connections(8).acquire_timeout(Duration::from_secs(5)).connect(&cfg.postgres_url).await.context("connect postgres")?;
    let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let addr = cfg.addr.clone();
    let max_body = cfg.max_body;
    let st: Shared = Arc::new(Inner { cfg, pg, http, keys: Mutex::default(), limits: Mutex::default() });
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/traces", post(traces))
        .route("/v1/metrics", post(metrics))
        .route("/v1/logs", post(otlp_logs))
        .route("/api/v2/logs", post(json_logs))
        .layer(DefaultBodyLimit::max(max_body))
        .with_state(st);
    let listener = tokio::net::TcpListener::bind(&addr).await.with_context(|| format!("bind {addr}"))?;
    tracing::info!(%addr, "oc-intake listening");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_headers() {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer ocik_abc"));
        assert_eq!(api_key(&h), Some("ocik_abc"));
        let mut h = HeaderMap::new();
        h.insert("oc-api-key", HeaderValue::from_static("ocik_xyz"));
        assert_eq!(api_key(&h), Some("ocik_xyz"));
        // Application keys (ocak_) are for the read API, not for sending data.
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer ocak_abc"));
        assert_eq!(api_key(&h), None);
    }

    #[test]
    fn bucket_limits_and_refills() {
        let mut b = Bucket { tokens: 2.0, at: Instant::now() };
        assert!(b.take(1.0, 2.0));
        assert!(b.take(1.0, 2.0));
        assert!(!b.take(1.0, 2.0));
        b.at -= Duration::from_secs(1);
        assert!(b.take(2.0, 2.0));
    }
}
