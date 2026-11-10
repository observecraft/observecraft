//! ObserveCraft telemetry intake.
//!
//! The only public way in for telemetry. Every request must carry an ingest API key
//! (`Authorization: Bearer ocik_…`, `OC-API-Key: ocik_…` or Datadog's `DD-API-KEY: ocik_…`); the key
//! is resolved to its organization and the request is forwarded with `x-oc-org: <org>` (any
//! client-sent value is dropped). The OTel Collector turns that header into the `oc.org_id` resource
//! attribute every table is filtered by; for JSON logs, Vector does the same.
//!
//! - `POST /v1/traces | /v1/logs | /v1/metrics`  OTLP/HTTP (protobuf or JSON, optionally gzip) → Collector; `/v1/logs`
//!   takes `?ddsource=&service=` (or `X-OC-Source` / `X-OC-Service`) for every record without its own (push packages)
//! - OTLP/gRPC on OC_INTAKE_GRPC_ADDR             the same three services (passed through undecoded) → Collector
//! - `POST /api/v2/logs`                         JSON log events (object, array or NDJSON; Datadog's format) → Vector;
//!   `?ddsource=&service=&hostname=&ddtags=` (`X-OC-Source` / `X-OC-Service`) apply to every log (webhooks, `log_query`)
//! - `POST /api/v2/secruntime`, `/api/v2/secinfo` the Agent's runtime security events (Workload Protection), as logs
//! - `POST /api/v2/series`, `/api/v1/series`     Datadog metrics (JSON or the Agent's protobuf) → OTLP → Collector
//! - `POST /api/beta/sketches`                   Datadog distribution metrics (DDSketch) → OTLP exponential histograms
//! - `POST /api/v0.2/traces`, `/api/v0.2/stats`  the Datadog Agent's APM traces (dd-trace libraries) → OTLP spans
//! - `POST /api/v0.1/pipeline_stats`            Data Streams Monitoring pathway checkpoints (dd-trace libraries, via the Agent) → otel.otel_pathways
//! - `POST /api/v2/orchmanif`                    Datadog Cluster Agent manifests → Kubernetes Explorer objects
//! - `GET /api/v1/validate`                      Datadog's API key check
//! - `POST /api/v1/check_run`                    Datadog service checks → the `service_check.status` gauge
//! - `POST /api/v1/collector`, `/api/v1/container`  Datadog live processes and containers → process.* / container.* metrics
//! - `POST /api/v1/connections`                  Datadog network connections (system-probe) → otel.otel_network
//! - `POST /api/v2/profile`                      Datadog profiler uploads → the server (otel.profiles)
//! - `POST /api/v2/apmtelemetry`                 tracers' instrumentation telemetry (via the Agent) → the server (Code Security's runtime libraries)
//! - `POST /api/v2/cicodescan`                   `datadog-ci sarif upload` reports → the server (Code Security findings)
//! - `POST /api/v2/autoscaling/recommend`         the Cluster Agent's external recommender → the server's replicas
//! - `POST /intake/`, `/api/v2/host_metadata`   Datadog Agent host metadata → Postgres `datadog_hosts`; its events → events.
//!   Its host tags are added to that host's series, sketches and service checks (see `host_tags`)
//! - `POST /api/v1/events`, `/api/v2/events`     Datadog events (API, Agent) → otel.otel_events
//! - `POST /intake/webhook/{name}`               vendors' alert webhooks (SolarWinds, Alertmanager, Zabbix) → events;
//!   connector packages' webhooks (signature checked, mapped by the package) → logs and events (`connector_push`)
//! - `POST /api/v2/rum`                          RUM events of the browser and mobile SDKs (client token `pub…`) → otel.otel_rum
//! - `POST /api/v2/replay`                       Session Replay segments of the browser SDK (multipart) → otel.otel_rum_replay
//! - `POST /api/v2/service_checks`               the Agent's service checks (as `/api/v1/check_run`)
//! - `POST /services/collector[/event|/raw]`     Splunk HEC-compatible logs (`Authorization: Splunk <ingest key>`:
//!   GitHub's audit log streaming, see `splunk_hec`)
//! - `GET /.well-known/fastly/logging/challenge` Fastly logging endpoints' check (OC_INTAKE_FASTLY_SERVICES, else `*`)
//! - `POST /ofrep/v1/evaluate/flags[/{key}]`     feature flag evaluation for OpenFeature SDKs (OFREP, see `ofrep`)
//! - `POST /api/v1/metadata`                     the Agent's inventory (features, configuration) → `datadog_hosts.agent_inventory`;
//!   other inventories are acknowledged
//! - `POST /api/v2/orch`                         the Agent's orchestrator summaries, acknowledged but not stored
//!
//! With OC_KAFKA_BROKERS set (docs/adr/0004-kafka-ingestion-buffer.md), OTLP and JSON logs are written to
//! Kafka instead (topics per signal, keyed by org, the org's headers as record headers) and acknowledged once
//! Kafka has them; the Collector and Vector consume from there. Without it they are forwarded over HTTP.
//!
//! Each org is rate limited (requests and bytes per second); over the limit answers 429.
//!
//! Env: OC_KAFKA_BROKERS (unset: HTTP forwarding), OC_KAFKA_ORG_SPREAD (12 partitions per org),
//! OC_INTAKE_ADDR (0.0.0.0:4318), OC_INTAKE_GRPC_ADDR (0.0.0.0:4317), OC_POSTGRES_URL,
//! OC_INTAKE_COLLECTOR_URL (http://localhost:14318), OC_INTAKE_LOGS_URL (http://localhost:8687),
//! OC_INTAKE_PROFILES_URL (http://localhost:8090/ingest/profiles),
//! OC_INTAKE_APM_TELEMETRY_URL (http://localhost:8090/ingest/apm-telemetry),
//! OC_INTAKE_SBOM_URL (http://localhost:8090/ingest/sbom),
//! OC_INTAKE_CODE_SCANS_URL (http://localhost:8090/ingest/code-scans),
//! OC_INTAKE_FLAG_EVALUATIONS_URL (http://localhost:8090/ingest/feature-flag-evaluations),
//! OC_INTAKE_CONNECTORS_URL (http://localhost:8090/ingest/connectors), OC_INTAKE_FASTLY_SERVICES (Fastly service ids
//! allowed by the logging challenge; unset: any), OC_INTERNAL_TOKEN,
//! OC_INTAKE_MAX_BODY_MB (16), OC_INTAKE_ORG_RPS (500), OC_INTAKE_ORG_MBPS (50),
//! OC_INTAKE_SERIES_CACHE (6000000 series × hour entries, ~20 bytes each; 0 = no series marking, see `series_cache`).
//! It must exceed the active series a replica sees: past it, points keep rewriting their series and tags rows.

mod check_messages;
mod connector_push;
mod datadog;
mod datadog_apm;
mod datadog_dns;
mod datadog_dsm;
mod datadog_events;
mod datadog_network;
mod datadog_orch;
mod datadog_process;
mod datadog_replay;
mod datadog_rum;
mod datadog_sbom;
mod datadog_tags;
mod datadog_webhooks;
mod grpc;
mod host_tags;
mod kafka;
mod log_identity;
mod log_query;
mod otlp_log_identity;
mod metric_rules;
mod ofrep;
mod otlp;
mod series_cache;
mod splunk_hec;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use axum::body::Bytes;
use kafka::Sink;
use axum::extract::{DefaultBodyLimit, RawQuery, State};
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
    /// The server's internal listener for feature flag evaluation counts.
    flag_evaluations_url: String,
    /// The server's internal listeners for Code Security: tracer telemetry and SARIF uploads.
    apm_telemetry_url: String,
    sbom_url: String,
    code_scans_url: String,
    /// The server's internal listener for the Cluster Agent's external recommender calls.
    autoscaling_url: String,
    /// The server's internal listener for connector packages' push definitions and signing secrets.
    connectors_url: String,
    /// Fastly service ids whose logging endpoints may send here (`/.well-known/fastly/logging/challenge`); empty: any.
    fastly_services: Vec<String>,
    /// Bearer token of the server's internal listener (OC_INTERNAL_TOKEN), if it requires one.
    internal_token: Option<String>,
    max_body: usize,
    org_rps: f64,
    org_bytes_per_sec: f64,
    /// Series remembered as forwarded in their hour (see `series_cache`); 0 turns marking off.
    series_cache: usize,
    /// Kafka bootstrap brokers (`host:port,…`); unset forwards over HTTP to the Collector and Vector.
    kafka_brokers: Option<String>,
    /// Partitions an organization's records spread over (see `kafka`).
    kafka_spread: usize,
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
            flag_evaluations_url: env("OC_INTAKE_FLAG_EVALUATIONS_URL", "http://localhost:8090/ingest/feature-flag-evaluations"),
            apm_telemetry_url: env("OC_INTAKE_APM_TELEMETRY_URL", "http://localhost:8090/ingest/apm-telemetry"),
            sbom_url: env("OC_INTAKE_SBOM_URL", "http://localhost:8090/ingest/sbom"),
            code_scans_url: env("OC_INTAKE_CODE_SCANS_URL", "http://localhost:8090/ingest/code-scans"),
            autoscaling_url: env("OC_INTAKE_AUTOSCALING_URL", "http://localhost:8090/ingest/autoscaling-recommendation"),
            connectors_url: env("OC_INTAKE_CONNECTORS_URL", "http://localhost:8090/ingest/connectors").trim_end_matches('/').to_string(),
            fastly_services: env("OC_INTAKE_FASTLY_SERVICES", "").split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
            internal_token: std::env::var("OC_INTERNAL_TOKEN")
                .ok()
                .filter(|t| !t.is_empty()),
            max_body: env("OC_INTAKE_MAX_BODY_MB", "16")
                .parse::<usize>()
                .unwrap_or(16)
                * 1024
                * 1024,
            // 0: unlimited (the default); instance admins can still limit an org (`org_data_settings`).
            org_rps: env("OC_INTAKE_ORG_RPS", "0").parse().unwrap_or(0.0),
            org_bytes_per_sec: env("OC_INTAKE_ORG_MBPS", "0").parse::<f64>().unwrap_or(0.0) * 1024.0 * 1024.0,
            series_cache: env("OC_INTAKE_SERIES_CACHE", "6000000").parse().unwrap_or(6_000_000),
            kafka_brokers: std::env::var("OC_KAFKA_BROKERS").ok().filter(|v| !v.trim().is_empty()),
            kafka_spread: env("OC_KAFKA_ORG_SPREAD", &kafka::DEFAULT_ORG_SPREAD.to_string()).parse().unwrap_or(kafka::DEFAULT_ORG_SPREAD),
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
    /// The feature flag environment its OFREP evaluations use when the context names none.
    pub flag_environment_id: Option<String>,
    /// The org's own ingest limits (MiB/s, requests/s); None follows the install's.
    pub ingest_mbps: Option<f64>,
    pub ingest_rps: Option<f64>,
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
    /// Datadog host tags per (org, host), added to the host's series.
    host_tags: host_tags::HostTags,
    /// Feature flag configuration per org and evaluation counts (OFREP).
    flags: ofrep::FlagState,
    /// Ingestion controls per org (Metrics → Settings → Rules).
    metric_rules: metric_rules::MetricRules,
    /// The last service check messages written per org, check and host.
    check_messages: check_messages::CheckMessages,
    /// The Kafka producer when OC_KAFKA_BROKERS is set.
    kafka: Option<kafka::Kafka>,
    /// Series forwarded per hour, so points of known series are sent marked.
    series: series_cache::SeriesCache,
    /// RUM client tokens seen lately.
    rum_tokens: datadog_rum::TokenCache,
    /// Connector packages' push definitions and orgs' signing secrets.
    push: connector_push::PushState,
}

type Shared = Arc<Inner>;

fn reject(status: StatusCode, msg: &str) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], format!("{{\"error\":\"{msg}\"}}")).into_response()
}

fn api_key(headers: &HeaderMap) -> Option<&str> {
    // `Splunk <token>`: the HEC endpoint's token (`splunk_hec`).
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("Splunk ")));
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
         COALESCE(s.metrics_5m_days, 0) AS metrics_5m_days, COALESCE(s.metrics_1h_days, 0) AS metrics_1h_days, k.flag_environment_id::text AS flag_environment_id, \
         s.ingest_mbps, s.ingest_rps FROM ingest_keys k JOIN orgs o ON o.id = k.org_id \
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

/// Whether the org may send `bytes` now: its own limits, else the install's; 0 (the default) is unlimited.
fn within_limits(st: &Shared, org: &KeyOrg, bytes: usize) -> bool {
    let rps = org.ingest_rps.unwrap_or(st.cfg.org_rps);
    let bps = org.ingest_mbps.map_or(st.cfg.org_bytes_per_sec, |m| m * 1024.0 * 1024.0);
    if rps <= 0.0 && bps <= 0.0 {
        return true;
    }
    let mut limits = st.limits.lock().unwrap();
    let l = limits.entry(org.org_id.clone()).or_insert_with(|| Limits { requests: Bucket { tokens: rps, at: Instant::now() }, bytes: Bucket { tokens: bps, at: Instant::now() } });
    (rps <= 0.0 || l.requests.take(1.0, rps)) && (bps <= 0.0 || l.bytes.take(bytes as f64, bps))
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
    if !within_limits(st, &org, bytes) {
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
    target: Sink,
    content_type: Option<&HeaderValue>,
    content_encoding: Option<&HeaderValue>,
    body: Bytes,
) -> Result<Upstream, Rejection> {
    send_with(st, org, target, content_type, content_encoding, body, &[]).await
}

/// The HTTP endpoint of a sink when there's no Kafka.
fn http_target(cfg: &Config, sink: Sink) -> String {
    match sink {
        Sink::Traces => format!("{}/v1/traces", cfg.collector_url),
        Sink::Metrics => format!("{}/v1/metrics", cfg.collector_url),
        Sink::Logs | Sink::Signals => format!("{}/v1/logs", cfg.collector_url),
        Sink::JsonLogs => cfg.logs_url.clone(),
    }
}

/// Writes one admitted body to its Kafka topic: decompressed (Kafka compresses batches itself), keyed by
/// org, the org's headers as record headers. Answers like an OTLP receiver once Kafka acknowledged it.
async fn produce(
    kafka: &kafka::Kafka,
    org: &KeyOrg,
    sink: Sink,
    content_type: Option<&HeaderValue>,
    content_encoding: Option<&HeaderValue>,
    body: Bytes,
    extra: &[(&str, &str)],
) -> Result<Upstream, Rejection> {
    let json = sink == Sink::JsonLogs || content_type.and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("json"));
    let value = match content_encoding {
        Some(enc) => {
            let mut h = HeaderMap::new();
            h.insert(header::CONTENT_ENCODING, enc.clone());
            datadog::inflate(&h, &body).map_err(|_| Rejection::new(StatusCode::BAD_REQUEST, "cannot decompress body"))?
        }
        None => body.to_vec(),
    };
    let mut headers = std::collections::BTreeMap::new();
    for (name, value) in org_headers(org) {
        headers.insert(name.to_string(), value.into_bytes());
    }
    for (name, value) in extra {
        headers.insert(name.to_string(), value.as_bytes().to_vec());
    }
    let topic = sink.topic(json);
    match kafka.produce(topic, &org.org_id, value, headers).await {
        Ok(()) => Ok(Upstream {
            status: StatusCode::OK,
            content_type: Some(HeaderValue::from_static(if json { "application/json" } else { "application/x-protobuf" })),
            // An empty Export*ServiceResponse.
            body: if json { Bytes::from_static(b"{}") } else { Bytes::new() },
        }),
        Err(e) => {
            tracing::warn!(error = %e, topic, "Kafka unavailable");
            Err(Rejection::new(StatusCode::SERVICE_UNAVAILABLE, "pipeline unavailable, retry later"))
        }
    }
}

/// [`send`] with `extra` headers for the pipeline (e.g. the series-known mark).
async fn send_with(
    st: &Shared,
    org: &KeyOrg,
    sink: Sink,
    content_type: Option<&HeaderValue>,
    mut content_encoding: Option<&HeaderValue>,
    mut body: Bytes,
    extra: &[(&str, &str)],
) -> Result<Upstream, Rejection> {
    if sink == Sink::JsonLogs || sink == Sink::Logs {
        let mut headers = HeaderMap::new();
        if let Some(encoding) = content_encoding {
            headers.insert(header::CONTENT_ENCODING, encoding.clone());
        }
        let decoded = datadog::inflate(&headers, &body).map_err(|_| Rejection::new(StatusCode::BAD_REQUEST, "cannot decompress body"))?;
        let prepared = if sink == Sink::JsonLogs {
            log_identity::prepare(&decoded)
        } else {
            let json = content_type.and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("json"));
            otlp_log_identity::prepare_limited(&decoded, json, st.cfg.max_body)
        };
        body = prepared
            .map_err(|message| {
                let status = if message == otlp_log_identity::TOO_LARGE { StatusCode::PAYLOAD_TOO_LARGE } else { StatusCode::BAD_REQUEST };
                Rejection::new(status, message)
            })?
            .into();
        content_encoding = None;
    }
    send_prepared(st, org, sink, content_type, content_encoding, body, extra).await
}

/// [`send_with`] for a body whose logs already carry their identities (`connector_push`: a webhook's records keep
/// theirs, so a retried delivery is stored once).
async fn send_prepared(
    st: &Shared,
    org: &KeyOrg,
    sink: Sink,
    content_type: Option<&HeaderValue>,
    content_encoding: Option<&HeaderValue>,
    body: Bytes,
    extra: &[(&str, &str)],
) -> Result<Upstream, Rejection> {
    if let Some(kafka) = &st.kafka {
        return produce(kafka, org, sink, content_type, content_encoding, body, extra).await;
    }
    let target = http_target(&st.cfg, sink);
    let target = target.as_str();
    let mut req = st.http.post(target).body(body);
    for (name, value) in org_headers(org) {
        req = req.header(name, value);
    }
    for (name, value) in extra {
        req = req.header(*name, *value);
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

/// Authenticates, rate limits and forwards one request body; JSON logs gain ingestion identities.
async fn forward(st: Shared, headers: HeaderMap, body: Bytes, target: Sink) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    forward_admitted(&st, &org, &headers, body, target).await
}

/// [`forward`] for a request already admitted.
async fn forward_admitted(st: &Shared, org: &KeyOrg, headers: &HeaderMap, body: Bytes, target: Sink) -> Response {
    match send(
        st,
        org,
        target,
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
    let url = st.cfg.profiles_url.clone();
    pass_to_server(st, headers, body, url, "profile").await
}

/// `POST /api/v2/apmtelemetry`: a tracer's instrumentation telemetry, forwarded by the Agent (the
/// libraries a service loads, for Code Security).
async fn apm_telemetry(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let url = st.cfg.apm_telemetry_url.clone();
    pass_to_server(st, headers, body, url, "telemetry").await
}

/// `POST /api/v2/sbom`: the Agent's software bills of materials (hosts and container images),
/// decoded and passed to the server as JSON (Cloud Security).
async fn sbom(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org.org_id,
        Err(r) => return r.into_response(),
    };
    let raw = match datadog::inflate(&headers, &body) {
        Ok(r) => r,
        Err(e) => return reject(StatusCode::BAD_REQUEST, &e),
    };
    let payload = match datadog_sbom::decode(&raw) {
        Ok(p) => p,
        Err(e) => return reject(StatusCode::BAD_REQUEST, &e),
    };
    let mut json_headers = HeaderMap::new();
    json_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let url = st.cfg.sbom_url.clone();
    send_to_server(&st, &org, &json_headers, Bytes::from(datadog_sbom::to_upload(&payload).to_string()), &url, "SBOM").await
}

/// `POST /api/v2/cicodescan`: a `datadog-ci sarif upload` (multipart: the scan's git context and
/// its SARIF report).
async fn code_scan(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let url = st.cfg.code_scans_url.clone();
    pass_to_server(st, headers, body, url, "code scan").await
}

/// `POST /api/v2/autoscaling/recommend?api_key=…`: the Datadog Cluster Agent's external recommender
/// (the `custom-recommender` endpoint of a DatadogPodAutoscaler), answered by the server with the
/// replicas the workload needs; its reply goes back unchanged.
async fn autoscaling_recommend(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org.org_id,
        Err(r) => return r.into_response(),
    };
    let mut req = st.http.post(&st.cfg.autoscaling_url).header(ORG_HEADER, &org).header(header::CONTENT_TYPE, "application/json").body(body);
    if let Some(t) = &st.cfg.internal_token {
        req = req.bearer_auth(t);
    }
    let res = match req.send().await {
        Ok(res) => res,
        Err(e) => {
            tracing::warn!(error = %e, "server unavailable for an autoscaling recommendation");
            return reject(StatusCode::SERVICE_UNAVAILABLE, "recommender unavailable, retry later");
        }
    };
    let status = StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    match res.bytes().await {
        Ok(reply) => (status, [(header::CONTENT_TYPE, "application/json")], reply).into_response(),
        Err(_) => reject(StatusCode::BAD_GATEWAY, "recommender reply lost"),
    }
}

/// Passes an upload unchanged to one of the server's internal listeners, with the org resolved from
/// its key.
async fn pass_to_server(st: Shared, headers: HeaderMap, body: Bytes, url: String, what: &'static str) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org.org_id,
        Err(r) => return r.into_response(),
    };
    send_to_server(&st, &org, &headers, body, &url, what).await
}

/// Posts a body to one of the server's internal listeners for `org`.
async fn send_to_server(st: &Shared, org: &str, headers: &HeaderMap, body: Bytes, url: &str, what: &'static str) -> Response {
    let mut req = st.http.post(url).header(ORG_HEADER, org).body(body);
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
            tracing::warn!(status = %res.status(), what, "server refused an upload");
            reject(
                if res.status().is_client_error() {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                },
                &format!("{what} was not accepted"),
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, what, "server unavailable");
            reject(
                StatusCode::SERVICE_UNAVAILABLE,
                "pipeline unavailable, retry later",
            )
        }
    }
}

async fn traces(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    forward(st, headers, body, Sink::Traces).await
}

async fn metrics(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    forward(st, headers, body, Sink::Metrics).await
}

/// `POST /v1/logs`; `?ddsource=&service=` (a push package's OTLP endpoint, as its page shows it), else the
/// `X-OC-Source` / `X-OC-Service` headers (senders that take only a base URL, such as Modal), become every record's
/// attributes of that name unless it has its own, so the package's pipeline and facets match.
async fn otlp_logs(State(st): State<Shared>, RawQuery(query): RawQuery, mut headers: HeaderMap, body: Bytes) -> Response {
    let d = log_query::LogDefaults::from_request(query.as_deref(), &headers);
    let defaults: Vec<(&str, String)> = [("ddsource", d.ddsource), ("service", d.service)].into_iter().filter_map(|(k, v)| Some((k, v?))).collect();
    if defaults.is_empty() {
        return forward(st, headers, body, Sink::Logs).await;
    }
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let json = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("json"));
    let applied = datadog::inflate(&headers, &body).map_err(|_| "cannot decompress body").and_then(|raw| otlp_log_identity::with_defaults(&raw, json, &defaults));
    match applied {
        Ok(body) => {
            headers.remove(header::CONTENT_ENCODING);
            forward_admitted(&st, &org, &headers, Bytes::from(body), Sink::Logs).await
        }
        Err(message) => reject(StatusCode::BAD_REQUEST, message),
    }
}

/// `POST /api/v2/logs`: JSON logs (an object, an array or a JSON object per line); `ddsource`, `service`, `hostname` and `ddtags` in the query string apply to
/// every log of the body (`log_query`), as on Datadog's intake; `X-OC-Source` / `X-OC-Service` when the query has none.
async fn json_logs(State(st): State<Shared>, RawQuery(query): RawQuery, mut headers: HeaderMap, body: Bytes) -> Response {
    let defaults = log_query::LogDefaults::from_request(query.as_deref(), &headers);
    if defaults.is_empty() {
        return forward(st, headers, body, Sink::JsonLogs).await;
    }
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    // A connector package's signed log drain (Vercel): its signature over the body as sent, uncompressed.
    if let Some(source) = &defaults.ddsource {
        if let Some(def) = connector_push::logs_def(&st, source).await {
            let raw = match datadog::inflate(&headers, &body) {
                Ok(raw) => raw,
                Err(_) => return reject(StatusCode::BAD_REQUEST, "cannot decompress body"),
            };
            if let Err(r) = connector_push::check(&st, &org, &def, &headers, &raw).await {
                return r.into_response();
            }
        }
    }
    let applied = datadog::inflate(&headers, &body).map_err(|_| "cannot decompress body").and_then(|raw| defaults.apply(&raw));
    match applied {
        Ok(body) => {
            headers.remove(header::CONTENT_ENCODING);
            forward_admitted(&st, &org, &headers, Bytes::from(body), Sink::JsonLogs).await
        }
        Err(message) => reject(StatusCode::BAD_REQUEST, message),
    }
}

/// `GET /.well-known/fastly/logging/challenge`: Fastly's HTTPS logging endpoints check that the host accepts their
/// service before sending (the hex SHA-256 of each allowed service id, one per line; `*` allows any). The challenge
/// carries no key, so it is the install's (OC_INTAKE_FASTLY_SERVICES); the logs themselves need the org's ingest key.
async fn fastly_challenge(State(st): State<Shared>) -> Response {
    let body = if st.cfg.fastly_services.is_empty() {
        "*\n".to_string()
    } else {
        st.cfg.fastly_services.iter().map(|id| Sha256::digest(id.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>() + "\n").collect()
    };
    ([(header::CONTENT_TYPE, "text/plain")], body).into_response()
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

fn state(cfg: Config, pg: PgPool, http: reqwest::Client, kafka: Option<kafka::Kafka>) -> Shared {
    let series = series_cache::SeriesCache::new(cfg.series_cache);
    Arc::new(Inner { cfg, pg, http, kafka, keys: Mutex::default(), limits: Mutex::default(), host_tags: host_tags::HostTags::default(), flags: ofrep::FlagState::default(), metric_rules: metric_rules::MetricRules::default(), check_messages: check_messages::CheckMessages::default(), series, rum_tokens: Mutex::default(), push: connector_push::PushState::default() })
}

/// An intake for tests: `cfg`, a Postgres pool that never connects unless used.
#[cfg(test)]
fn test_state(cfg: Config) -> Shared {
    let pg = PgPoolOptions::new().connect_lazy("postgres://localhost/unused").expect("lazy pool");
    state(cfg, pg, reqwest::Client::new(), None)
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
    let kafka = match &cfg.kafka_brokers {
        Some(brokers) => {
            let k = kafka::Kafka::connect(brokers, cfg.kafka_spread).await.with_context(|| format!("connect to Kafka at {brokers}"))?;
            tracing::info!(%brokers, "writing telemetry to Kafka");
            Some(k)
        }
        None => {
            tracing::info!(collector = %cfg.collector_url, "OC_KAFKA_BROKERS unset: forwarding telemetry over HTTP");
            None
        }
    };
    let st = state(cfg, pg, http, kafka);
    let flusher = st.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(ofrep::FLUSH_EVERY);
        loop {
            tick.tick().await;
            ofrep::flush(&flusher).await;
        }
    });
    let ofrep_routes = Router::new()
        .route("/ofrep/v1/evaluate/flags", post(ofrep::evaluate_flags))
        .route("/ofrep/v1/evaluate/flags/{key}", post(ofrep::evaluate_flag))
        .layer(ofrep::cors());
    let rum_routes = Router::new().route("/api/v2/rum", post(datadog_rum::post)).route("/api/v2/replay", post(datadog_replay::post)).layer(datadog_rum::cors());
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/traces", post(traces))
        .route("/v1/metrics", post(metrics))
        .route("/v1/logs", post(otlp_logs))
        .route("/api/v2/logs", post(json_logs))
        // The Agent's runtime security (Workload Protection) events and remediation results: logs
        // payloads on their own tracks (`runtime_security_config.endpoints`).
        .route("/api/v2/secruntime", post(json_logs))
        .route("/api/v2/secinfo", post(json_logs))
        .route("/api/v2/series", post(datadog::series_v2))
        .route("/api/v1/series", post(datadog::series_v1))
        .route("/api/v1/validate", get(datadog::validate))
        .route("/api/beta/sketches", post(datadog::sketches))
        .route("/api/v0.2/traces", post(datadog_apm::traces))
        .route("/api/v0.2/stats", post(datadog_apm::stats))
        .route("/api/v0.1/pipeline_stats", post(datadog_dsm::pipeline_stats))
        .route("/api/v2/orchmanif", post(datadog_orch::manifests))
        .route("/api/v2/orch", post(datadog::acknowledge))
        .route("/api/v1/collector", post(datadog_process::collector))
        .route("/api/v1/container", post(datadog_process::collector))
        .route("/api/v1/connections", post(datadog_network::connections))
        .route("/api/v2/profile", post(profile))
        .route("/api/v2/apmtelemetry", post(apm_telemetry))
        .route("/api/v2/sbom", post(sbom))
        .route("/api/v2/cicodescan", post(code_scan))
        .route("/api/v2/autoscaling/recommend", post(autoscaling_recommend))
        .route("/api/v1/check_run", post(datadog::check_run))
        .route("/api/v1/metadata", post(datadog::metadata))
        .route("/intake/", post(datadog::intake))
        .route("/intake/webhook/{name}", post(datadog_webhooks::post))
        .route("/api/v2/host_metadata", post(datadog::intake))
        .route("/api/v1/events", post(datadog_events::post_v1))
        .route("/api/v2/events", post(datadog_events::post_agent))
        .route("/api/v2/service_checks", post(datadog::check_run))
        .route("/.well-known/fastly/logging/challenge", get(fastly_challenge))
        .route("/services/collector", post(splunk_hec::event))
        .route("/services/collector/event", post(splunk_hec::event))
        .route("/services/collector/event/1.0", post(splunk_hec::event))
        .route("/services/collector/raw", post(splunk_hec::raw))
        .route("/services/collector/raw/1.0", post(splunk_hec::raw))
        .route("/services/collector/health", get(splunk_hec::health))
        .route("/services/collector/health/1.0", get(splunk_hec::health))
        .merge(ofrep_routes)
        .merge(rum_routes)
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

    /// A webhook posting to `/api/v2/logs?ddsource=…`: its logs reach the pipeline with the query's source,
    /// service, host and tags, compressed or not; without parameters the body passes as it was.
    #[tokio::test]
    async fn log_query_parameters_reach_the_pipeline() {
        use sha2::{Digest, Sha256};
        use std::io::Write;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = Config::from_env();
        cfg.logs_url = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/", post(|body: Bytes| async move { body }))).await.unwrap();
        });
        let st = test_state(cfg);
        let org = KeyOrg { org_id: "example-org".into(), span_retention_days: 15, metrics_retention_days: 30, metrics_5m_days: 90, metrics_1h_days: 395, flag_environment_id: None, ingest_mbps: None, ingest_rps: None };
        st.keys.lock().unwrap().insert(Sha256::digest(b"ocik_webhook_test").to_vec(), CachedKey { org: Some(org), at: Instant::now(), touched: Some(Instant::now()) });
        let mut headers = HeaderMap::new();
        headers.insert("dd-api-key", HeaderValue::from_static("ocik_webhook_test"));
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let post = |query: Option<&str>, headers: HeaderMap, body: Vec<u8>| {
            let st = st.clone();
            let query = query.map(str::to_string);
            async move {
                let res = json_logs(State(st), RawQuery(query), headers, Bytes::from(body)).await;
                let status = res.status();
                (status, serde_json::from_slice::<serde_json::Value>(&axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap()).unwrap_or_default())
            }
        };
        let query = Some("dd-api-key=ocik_webhook_test&ddsource=solarwinds&service=orion&hostname=nms-01&ddtags=env%3Aprod");
        let (status, got) = post(query, headers.clone(), br#"{"alert_name":"Node down","host":"core-sw-01"}"#.to_vec()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!((&got["ddsource"], &got["service"], &got["hostname"], &got["ddtags"], &got["alert_name"]), (&serde_json::json!("solarwinds"), &serde_json::json!("orion"), &serde_json::json!("nms-01"), &serde_json::json!("env:prod"), &serde_json::json!("Node down")));
        assert!(got["_oc_ingest"]["id"].is_string(), "stamped as any JSON log");
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(br#"[{"message":"a"},{"message":"b","ddsource":"own"}]"#).unwrap();
        let mut gz_headers = headers.clone();
        gz_headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        let (status, got) = post(query, gz_headers, gzip.finish().unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!((&got[0]["ddsource"], &got[1]["ddsource"]), (&serde_json::json!("solarwinds"), &serde_json::json!("own")));
        let (_, got) = post(Some("api_key=ocik_webhook_test"), headers.clone(), br#"{"message":"plain"}"#.to_vec()).await;
        assert!(got.get("ddsource").is_none());
        for query in [query, None] {
            let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gzip.write_all(b"{\"message\":\"a\"}\n{\"message\":\"b\"}\n").unwrap();
            let mut gz_headers = headers.clone();
            gz_headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            let (status, got) = post(query, gz_headers, gzip.finish().unwrap()).await;
            assert_eq!((status, &got[1]["message"]), (StatusCode::OK, &serde_json::json!("b")), "NDJSON, {query:?}");
        }
        let (status, _) = post(query, headers.clone(), b"not json".to_vec()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = post(query, HeaderMap::new(), br#"{"message":"x"}"#.to_vec()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "admitted before the body is read");
    }

    /// Modal sends OTLP logs to a base URL with headers from a secret: `X-OC-Source` / `X-OC-Service` become the
    /// records' `ddsource` / `service`, as `?ddsource=&service=` do; the logs API reads them too.
    #[tokio::test]
    async fn source_and_service_headers_reach_the_pipeline() {
        use sha2::{Digest, Sha256};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = Config::from_env();
        cfg.collector_url = format!("http://{}", listener.local_addr().unwrap());
        cfg.logs_url = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/v1/logs", post(|body: Bytes| async move { body })).route("/", post(|body: Bytes| async move { body }))).await.unwrap();
        });
        let st = test_state(cfg);
        let org = KeyOrg { org_id: "example-org".into(), span_retention_days: 15, metrics_retention_days: 30, metrics_5m_days: 90, metrics_1h_days: 395, flag_environment_id: None, ingest_mbps: None, ingest_rps: None };
        st.keys.lock().unwrap().insert(Sha256::digest(b"ocik_modal_test").to_vec(), CachedKey { org: Some(org), at: Instant::now(), touched: Some(Instant::now()) });
        let mut headers = HeaderMap::new();
        headers.insert("dd-api-key", HeaderValue::from_static("ocik_modal_test"));
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("x-oc-source", HeaderValue::from_static("modal"));
        headers.insert("x-oc-service", HeaderValue::from_static("modal"));
        let otlp = br#"{"resourceLogs":[{"scopeLogs":[{"logRecords":[{"body":{"stringValue":"started"}}]}]}]}"#;
        let res = otlp_logs(State(st.clone()), RawQuery(None), headers.clone(), Bytes::from_static(otlp)).await;
        assert_eq!(res.status(), StatusCode::OK);
        let got: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap()).unwrap();
        let attrs = got["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["attributes"].as_array().unwrap().clone();
        for key in ["ddsource", "service"] {
            assert!(attrs.contains(&serde_json::json!({"key": key, "value": {"stringValue": "modal"}})), "{key}: {attrs:?}");
        }
        let res = json_logs(State(st), RawQuery(None), headers, Bytes::from_static(br#"{"message":"m"}"#)).await;
        let got: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap()).unwrap();
        assert_eq!((&got["ddsource"], &got["service"]), (&serde_json::json!("modal"), &serde_json::json!("modal")));
    }

    /// The Cluster Agent's external recommender call: admitted by its key, forwarded to the server
    /// with the org, and the server's reply (body and status) returned as is.
    #[tokio::test]
    async fn autoscaling_recommendations_are_answered_by_the_server() {
        use sha2::{Digest, Sha256};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = Config::from_env();
        cfg.autoscaling_url = format!("http://{}/ingest/autoscaling-recommendation", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/ingest/autoscaling-recommendation",
                    post(|headers: HeaderMap, body: Bytes| async move {
                        assert_eq!(headers[ORG_HEADER], "example-org");
                        if body.as_ref() == b"{}" {
                            return (StatusCode::BAD_REQUEST, "not a WorkloadRecommendationRequest".to_string());
                        }
                        (StatusCode::OK, r#"{"timestamp":"2026-11-01T18:00:00Z","targetReplicas":5,"observedTargets":[],"reason":"cpu"}"#.to_string())
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let st = test_state(cfg);
        st.keys.lock().unwrap().insert(
            Sha256::digest(b"ocik_autoscaling_test").to_vec(),
            CachedKey {
                org: Some(KeyOrg { org_id: "example-org".into(), span_retention_days: 15, metrics_retention_days: 30, metrics_5m_days: 90, metrics_1h_days: 395, flag_environment_id: None, ingest_mbps: None, ingest_rps: None }),
                at: Instant::now(),
                touched: Some(Instant::now()),
            },
        );
        let mut headers = HeaderMap::new();
        headers.insert("dd-api-key", HeaderValue::from_static("ocik_autoscaling_test"));
        let res = autoscaling_recommend(State(st.clone()), headers.clone(), Bytes::from_static(br#"{"targetRef":{"name":"storefront-api"}}"#)).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
        let body = axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap();
        assert!(std::str::from_utf8(&body).unwrap().contains(r#""targetReplicas":5"#));
        let res = autoscaling_recommend(State(st), headers, Bytes::from_static(b"{}")).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }
    use super::*;

    #[tokio::test]
    async fn ingest_is_unlimited_unless_the_install_or_the_org_sets_a_limit() {
        let mut cfg = Config::from_env();
        (cfg.org_rps, cfg.org_bytes_per_sec) = (0.0, 0.0);
        let st = test_state(cfg);
        let org = |id: &str, mbps: Option<f64>, rps: Option<f64>| KeyOrg { org_id: id.into(), span_retention_days: 0, metrics_retention_days: 0, metrics_5m_days: 0, metrics_1h_days: 0, flag_environment_id: None, ingest_mbps: mbps, ingest_rps: rps };
        let free = org("free", None, None);
        assert!((0..10_000).all(|_| within_limits(&st, &free, 100 << 20)), "no limit by default");
        let bytes = org("bytes", Some(1.0), None);
        assert!(within_limits(&st, &bytes, 512 << 10));
        assert!(!within_limits(&st, &bytes, 1 << 20), "1 MiB/s for this org");
        let requests = org("requests", None, Some(2.0));
        assert!(within_limits(&st, &requests, 1) && within_limits(&st, &requests, 1));
        assert!(!within_limits(&st, &requests, 1), "2 requests/s for this org");
        // An install-wide limit applies to orgs without their own.
        let mut cfg = Config::from_env();
        (cfg.org_rps, cfg.org_bytes_per_sec) = (1.0, 0.0);
        let st = test_state(cfg);
        assert!(within_limits(&st, &free, 1) && !within_limits(&st, &free, 1));
        assert!((0..100).all(|_| within_limits(&st, &org("own", None, Some(1000.0)), 1)), "the org's own limit wins");
    }

    #[test]
    fn org_headers_carry_its_retention() {
        let org = KeyOrg { org_id: "acme".into(), span_retention_days: 15, metrics_retention_days: 30, metrics_5m_days: 90, metrics_1h_days: 395, flag_environment_id: None, ingest_mbps: None, ingest_rps: None };
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

    #[tokio::test]
    async fn fastly_challenge_answers_the_allowed_services() {
        let body = |r: Response| async { String::from_utf8(axum::body::to_bytes(r.into_body(), 1024).await.unwrap().to_vec()).unwrap() };
        let st = test_state(Config::from_env());
        assert_eq!(body(fastly_challenge(State(st)).await).await, "*\n", "any service by default");
        let mut cfg = Config::from_env();
        cfg.fastly_services = vec!["SU1Z0isxPaozGVKXdv0eY".into(), "other".into()];
        let lines = body(fastly_challenge(State(test_state(cfg))).await).await;
        let hex = |s: &[u8]| Sha256::digest(s).iter().map(|b| format!("{b:02x}")).collect::<String>();
        let want = format!("{}\n{}\n", hex(b"SU1Z0isxPaozGVKXdv0eY"), hex(b"other"));
        assert_eq!(lines, want);
        assert_eq!(lines.lines().next().unwrap().len(), 64);
    }

    #[test]
    fn key_headers() {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer ocik_abc"));
        assert_eq!(api_key(&h), Some("ocik_abc"));
        let mut h = HeaderMap::new();
        h.insert("oc-api-key", HeaderValue::from_static("ocik_xyz"));
        assert_eq!(api_key(&h), Some("ocik_xyz"));
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Splunk ocik_hec"));
        assert_eq!(api_key(&h), Some("ocik_hec"), "a Splunk HEC token");
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
