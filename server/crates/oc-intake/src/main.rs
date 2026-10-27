//! ObserveCraft telemetry intake.
//!
//! The only public way in for telemetry. Every request must carry an ingest API key
//! (`Authorization: Bearer ocik_…`, `OC-API-Key: ocik_…` or Datadog's `DD-API-KEY: ocik_…`); the key
//! is resolved to its organization and the request is forwarded with `x-oc-org: <org>` (any
//! client-sent value is dropped). The OTel Collector turns that header into the `oc.org_id` resource
//! attribute every table is filtered by; for JSON logs, Vector does the same.
//!
//! - `POST /v1/traces | /v1/logs | /v1/metrics`  OTLP/HTTP (protobuf or JSON, optionally gzip) → Collector
//! - OTLP/gRPC on OC_INTAKE_GRPC_ADDR             the same three services (passed through undecoded) → Collector
//! - `POST /api/v2/logs`                         JSON log events (object or array; Datadog's format) → Vector
//! - `POST /api/v2/series`, `/api/v1/series`     Datadog metrics (JSON or the Agent's protobuf) → OTLP → Collector
//! - `POST /api/beta/sketches`                   Datadog distribution metrics (DDSketch) → OTLP exponential histograms
//! - `POST /api/v0.2/traces`, `/api/v0.2/stats`  the Datadog Agent's APM traces (dd-trace libraries) → OTLP spans
//! - `POST /api/v2/orchmanif`                    Datadog Cluster Agent manifests → Kubernetes Explorer objects
//! - `GET /api/v1/validate`                      Datadog's API key check
//! - `POST /api/v1/check_run`                    Datadog service checks → the `service_check.status` gauge
//! - `POST /api/v1/collector`, `/api/v1/container`  Datadog live processes and containers → process.* / container.* metrics
//! - `POST /api/v1/connections`                  Datadog network connections (system-probe) → otel.otel_network
//! - `POST /api/v2/profile`                      Datadog profiler uploads → the server (otel.profiles)
//! - `POST /intake/`, `/api/v2/host_metadata`   Datadog Agent host metadata → Postgres `datadog_hosts`; its events → events
//! - `POST /api/v1/events`, `/api/v2/events`     Datadog events (API, Agent) → otel.otel_events
//! - `POST /api/v2/service_checks`               the Agent's service checks (as `/api/v1/check_run`)
//! - `POST /api/v1/metadata`, `/api/v2/orch`: the Agent's inventories and orchestrator summaries,
//!   acknowledged but not stored
//!
//! Each org is rate limited (requests and bytes per second); over the limit answers 429.
//!
//! Env: OC_INTAKE_ADDR (0.0.0.0:4318), OC_INTAKE_GRPC_ADDR (0.0.0.0:4317), OC_POSTGRES_URL,
//! OC_INTAKE_COLLECTOR_URL (http://localhost:14318), OC_INTAKE_LOGS_URL (http://localhost:8687),
//! OC_INTAKE_PROFILES_URL (http://localhost:8090/ingest/profiles), OC_INTERNAL_TOKEN,
//! OC_INTAKE_MAX_BODY_MB (16), OC_INTAKE_ORG_RPS (500), OC_INTAKE_ORG_MBPS (50).

mod datadog;
mod datadog_apm;
mod datadog_dns;
mod datadog_events;
mod datadog_network;
mod datadog_orch;
mod datadog_process;
mod datadog_tags;
mod grpc;
mod otlp;

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
    grpc_addr: String,
    postgres_url: String,
    collector_url: String,
    logs_url: String,
    /// The server's internal listener for profile uploads.
    profiles_url: String,
    /// Bearer token of the server's internal listener (OC_INTERNAL_TOKEN), if it requires one.
    internal_token: Option<String>,
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
            grpc_addr: env("OC_INTAKE_GRPC_ADDR", "0.0.0.0:4317"),
            postgres_url: env(
                "OC_POSTGRES_URL",
                "postgres://observecraft:observecraft@localhost:5432/observecraft",
            ),
            collector_url: env("OC_INTAKE_COLLECTOR_URL", "http://localhost:14318")
                .trim_end_matches('/')
                .to_string(),
            logs_url: env("OC_INTAKE_LOGS_URL", "http://localhost:8687")
                .trim_end_matches('/')
                .to_string(),
            profiles_url: env(
                "OC_INTAKE_PROFILES_URL",
                "http://localhost:8090/ingest/profiles",
            ),
            internal_token: std::env::var("OC_INTERNAL_TOKEN")
                .ok()
                .filter(|t| !t.is_empty()),
            max_body: env("OC_INTAKE_MAX_BODY_MB", "16")
                .parse::<usize>()
                .unwrap_or(16)
                * 1024
                * 1024,
            org_rps: env("OC_INTAKE_ORG_RPS", "500").parse().unwrap_or(500.0),
            org_bytes_per_sec: env("OC_INTAKE_ORG_MBPS", "50").parse::<f64>().unwrap_or(50.0) * 1024.0 * 1024.0,
        }
    }
}

/// The org an ingest key sends as, and its retention (days; 0 = the install's default).
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct KeyOrg {
    pub org_id: String,
    pub span_retention_days: i32,
    pub metrics_retention_days: i32,
    pub metrics_5m_days: i32,
    pub metrics_1h_days: i32,
}

/// Headers every forwarded body carries; the collector turns them into resource attributes
/// (`oc.org_id`, `oc.retention_days`, `oc.metrics_retention_*`), replacing what a sender set.
fn org_headers(o: &KeyOrg) -> [(&'static str, String); 5] {
    [
        (ORG_HEADER, o.org_id.clone()),
        (RETENTION_HEADER, o.span_retention_days.to_string()),
        ("x-oc-metrics-retention-days", o.metrics_retention_days.to_string()),
        ("x-oc-metrics-retention-5m-days", o.metrics_5m_days.to_string()),
        ("x-oc-metrics-retention-1h-days", o.metrics_1h_days.to_string()),
    ]
}

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
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let plain = || {
        ["oc-api-key", "dd-api-key"]
            .into_iter()
            .find_map(|h| headers.get(h).and_then(|v| v.to_str().ok()))
    };
    bearer
        .or_else(plain)
        .map(str::trim)
        .filter(|k| k.starts_with("ocik_"))
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
        "SELECT k.org_id, COALESCE(s.span_retention_days, 0) AS span_retention_days, COALESCE(s.metrics_retention_days, 0) AS metrics_retention_days, \
         COALESCE(s.metrics_5m_days, 0) AS metrics_5m_days, COALESCE(s.metrics_1h_days, 0) AS metrics_1h_days FROM ingest_keys k JOIN orgs o ON o.id = k.org_id \
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

/// Why a request was turned away.
pub struct Rejection {
    pub status: StatusCode,
    pub message: &'static str,
}

impl Rejection {
    const fn new(status: StatusCode, message: &'static str) -> Self {
        Rejection { status, message }
    }
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        let mut r = reject(self.status, self.message);
        if self.status == StatusCode::TOO_MANY_REQUESTS {
            r.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        }
        r
    }
}

/// Authenticates the sender and charges `bytes` to its org's rate limit.
async fn admit(st: &Shared, headers: &HeaderMap, bytes: usize) -> Result<KeyOrg, Rejection> {
    let Some(key) = api_key(headers) else {
        return Err(Rejection::new(
            StatusCode::UNAUTHORIZED,
            "missing API key: send Authorization: Bearer ocik_...",
        ));
    };
    let org = match resolve(st, key).await {
        Ok(Some(found)) => found,
        Ok(None) => {
            return Err(Rejection::new(
                StatusCode::UNAUTHORIZED,
                "invalid or revoked API key",
            ))
        }
        Err(e) => {
            tracing::error!(error = %e, "key lookup failed");
            return Err(Rejection::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily unavailable",
            ));
        }
    };
    if !within_limits(st, &org.org_id, bytes) {
        return Err(Rejection::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded for this organization",
        ));
    }
    Ok(org)
}

/// What the pipeline answered.
pub struct Upstream {
    pub status: StatusCode,
    pub content_type: Option<HeaderValue>,
    pub body: Bytes,
}

/// Sends one admitted body to `target`, stamped with its org.
async fn send(
    st: &Shared,
    org: &KeyOrg,
    target: &str,
    content_type: Option<&HeaderValue>,
    content_encoding: Option<&HeaderValue>,
    body: Bytes,
) -> Result<Upstream, Rejection> {
    let mut req = st.http.post(target).body(body);
    for (name, value) in org_headers(org) {
        req = req.header(name, value);
    }
    if let Some(v) = content_type {
        req = req.header(header::CONTENT_TYPE, v);
    }
    if let Some(v) = content_encoding {
        req = req.header(header::CONTENT_ENCODING, v);
    }
    match req.send().await {
        Ok(res) => {
            let status =
                StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let content_type = res
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| HeaderValue::from_bytes(v.as_bytes()).ok());
            let body = res.bytes().await.unwrap_or_default();
            Ok(Upstream {
                status,
                content_type,
                body,
            })
        }
        Err(e) => {
            tracing::warn!(error = %e, target, "upstream unavailable");
            Err(Rejection::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "pipeline unavailable, retry later",
            ))
        }
    }
}

/// Authenticates, rate limits and forwards one request body to `target` unchanged.
async fn forward(st: Shared, headers: HeaderMap, body: Bytes, target: String) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    match send(
        &st,
        &org,
        &target,
        headers.get(header::CONTENT_TYPE),
        headers.get(header::CONTENT_ENCODING),
        body,
    )
    .await
    {
        Ok(up) => {
            let mut out = (up.status, up.body).into_response();
            if let Some(ct) = up.content_type {
                out.headers_mut().insert(header::CONTENT_TYPE, ct);
            }
            out
        }
        Err(r) => r.into_response(),
    }
}

/// `POST /api/v2/profile`: a profiler upload (multipart) from the Datadog Agent, passed unchanged to
/// the server's internal listener, which stores it.
async fn profile(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org.org_id,
        Err(r) => return r.into_response(),
    };
    let mut req = st
        .http
        .post(&st.cfg.profiles_url)
        .header(ORG_HEADER, &org)
        .body(body);
    for h in [
        header::CONTENT_TYPE.as_str(),
        header::CONTENT_ENCODING.as_str(),
        "x-datadog-additional-tags",
    ] {
        if let Some(v) = headers.get(h) {
            req = req.header(h, v);
        }
    }
    if let Some(t) = &st.cfg.internal_token {
        req = req.bearer_auth(t);
    }
    match req.send().await {
        Ok(res) if res.status().is_success() => StatusCode::ACCEPTED.into_response(),
        Ok(res) => {
            tracing::warn!(status = %res.status(), "server refused a profile");
            reject(
                if res.status().is_client_error() {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                },
                "profile was not accepted",
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "profile store unavailable");
            reject(
                StatusCode::SERVICE_UNAVAILABLE,
                "pipeline unavailable, retry later",
            )
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

/// Datadog clients may pass the key in the query string (`?api_key=…` or `?dd-api-key=…`, e.g. the
/// Agent's key validation); it becomes the `DD-API-KEY` header the handlers read.
async fn query_api_key(mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if api_key(req.headers()).is_none() {
        let key = req.uri().query().and_then(|q| {
            q.split('&')
                .filter_map(|kv| kv.split_once('='))
                .find(|(k, _)| matches!(*k, "api_key" | "dd-api-key" | "DD-API-KEY"))
                .map(|(_, v)| v.to_string())
        });
        if let Some(v) = key.and_then(|k| HeaderValue::from_str(&k).ok()) {
            req.headers_mut().insert("dd-api-key", v);
        }
    }
    next.run(req).await
}

/// Resident memory in MiB (Linux), for the request log.
fn rss_mib() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map_or(0, |pages| pages * 4096 / 1_048_576)
}

/// Debug log of every request: path, size, encoding, status, latency and resident memory after it.
async fn log_requests(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let path = req.uri().path().to_string();
    let size = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string();
    let encoding = req
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string();
    let start = Instant::now();
    let res = next.run(req).await;
    tracing::debug!(%path, %size, %encoding, status = res.status().as_u16(), ms = start.elapsed().as_millis() as u64, rss_mib = rss_mib(), "request");
    res
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))).init();
    let cfg = Config::from_env();
    // Lazy: the intake starts (and answers 503) while Postgres isn't reachable yet.
    let pg = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(&cfg.postgres_url)
        .context("postgres url")?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let addr = cfg.addr.clone();
    let grpc_addr = cfg.grpc_addr.clone();
    let max_body = cfg.max_body;
    let st: Shared = Arc::new(Inner { cfg, pg, http, keys: Mutex::default(), limits: Mutex::default() });
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/traces", post(traces))
        .route("/v1/metrics", post(metrics))
        .route("/v1/logs", post(otlp_logs))
        .route("/api/v2/logs", post(json_logs))
        .route("/api/v2/series", post(datadog::series_v2))
        .route("/api/v1/series", post(datadog::series_v1))
        .route("/api/v1/validate", get(datadog::validate))
        .route("/api/beta/sketches", post(datadog::sketches))
        .route("/api/v0.2/traces", post(datadog_apm::traces))
        .route("/api/v0.2/stats", post(datadog_apm::stats))
        .route("/api/v2/orchmanif", post(datadog_orch::manifests))
        .route("/api/v2/orch", post(datadog::acknowledge))
        .route("/api/v1/collector", post(datadog_process::collector))
        .route("/api/v1/container", post(datadog_process::collector))
        .route("/api/v1/connections", post(datadog_network::connections))
        .route("/api/v2/profile", post(profile))
        .route("/api/v1/check_run", post(datadog::check_run))
        .route("/api/v1/metadata", post(datadog::acknowledge))
        .route("/intake/", post(datadog::intake))
        .route("/api/v2/host_metadata", post(datadog::intake))
        .route("/api/v1/events", post(datadog_events::post_v1))
        .route("/api/v2/events", post(datadog_events::post_agent))
        .route("/api/v2/service_checks", post(datadog::check_run))
        .layer(DefaultBodyLimit::max(max_body))
        .layer(axum::middleware::from_fn(query_api_key))
        .layer(axum::middleware::from_fn(log_requests))
        .with_state(st.clone());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    let grpc_listener = tokio::net::TcpListener::bind(&grpc_addr)
        .await
        .with_context(|| format!("bind {grpc_addr}"))?;
    tracing::info!(%addr, %grpc_addr, "oc-intake listening (OTLP/HTTP, OTLP/gRPC)");
    tokio::try_join!(
        axum::serve(listener, app),
        axum::serve(grpc_listener, grpc::router(st))
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_headers_carry_its_retention() {
        let org = KeyOrg { org_id: "acme".into(), span_retention_days: 15, metrics_retention_days: 30, metrics_5m_days: 90, metrics_1h_days: 395 };
        assert_eq!(
            org_headers(&org),
            [
                ("x-oc-org", "acme".to_string()),
                ("x-oc-retention-days", "15".to_string()),
                ("x-oc-metrics-retention-days", "30".to_string()),
                ("x-oc-metrics-retention-5m-days", "90".to_string()),
                ("x-oc-metrics-retention-1h-days", "395".to_string()),
            ]
        );
    }

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

    #[tokio::test]
    async fn api_key_from_the_query_string() {
        use tower::ServiceExt;
        let app = Router::new()
            .route(
                "/k",
                get(|h: HeaderMap| async move { api_key(&h).unwrap_or("none").to_string() }),
            )
            .layer(axum::middleware::from_fn(query_api_key));
        let call = |uri: &'static str| {
            let app = app.clone();
            async move {
                let res = app
                    .oneshot(
                        axum::extract::Request::builder()
                            .uri(uri)
                            .body(axum::body::Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                String::from_utf8(
                    axum::body::to_bytes(res.into_body(), 1024)
                        .await
                        .unwrap()
                        .to_vec(),
                )
                .unwrap()
            }
        };
        assert_eq!(call("/k?api_key=ocik_abc").await, "ocik_abc");
        assert_eq!(call("/k?x=1&dd-api-key=ocik_def").await, "ocik_def");
        assert_eq!(call("/k").await, "none");
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
