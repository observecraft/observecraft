//! APM endpoints computed from spans in `otel.otel_traces` (OTel Collector ClickHouse exporter schema).
//!
//! Conventions:
//! - **Entry spans** (Server/Consumer) measure an instrumented service's own requests.
//! - **Inferred services** (databases, caches, queues, external APIs) have no spans of their own; they are
//!   measured from callers' Client/Producer spans carrying `peer.service`.
//! - Durations are stored in nanoseconds; responses use milliseconds.
//!
//! Tenancy: the exporter's tables have no org column yet, so traces are single-tenant. Adding an
//! `oc.org` resource attribute in the collector and filtering on it here is the path to multi-tenancy.
//! Scale: these queries aggregate raw spans; per-minute materialized rollups are the next step at volume.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::Json;
use oc_query::{build_where, parse_query, Param, SpansSchema};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::{bind, TimeRange};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const ENTRY: &str = "SpanKind IN ('Server', 'Consumer')";
const OUTBOUND: &str = "SpanKind IN ('Client', 'Producer')";
const PEER: &str = "SpanAttributes['peer.service']";

fn time_clause(range: TimeRange) -> (String, Vec<Param>) {
    ("Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)".into(), vec![Param::Int(range.from), Param::Int(range.to)])
}

/// Time + optional env + optional search query, ANDed.
fn base(range: TimeRange, env: &Option<String>, query: &str) -> (String, Vec<Param>) {
    let (mut clause, mut params) = time_clause(range);
    if let Some(e) = env.as_deref().filter(|e| !e.is_empty() && *e != "*") {
        clause.push_str(" AND ResourceAttributes['deployment.environment'] = ?");
        params.push(Param::Str(e.to_string()));
    }
    let f = build_where(&SpansSchema, &parse_query(query), None);
    clause.push_str(&format!(" AND ({})", f.clause));
    params.extend(f.params);
    (clause, params)
}

fn rate(errors: u64, hits: u64) -> f64 {
    if hits == 0 {
        0.0
    } else {
        errors as f64 / hits as f64
    }
}

fn finite(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    Web,
    Worker,
    Db,
    Cache,
    Queue,
    Http,
}

fn inferred_kind(db_system: &str, messaging: &str) -> ServiceKind {
    match (db_system, messaging) {
        ("redis" | "memcached", _) => ServiceKind::Cache,
        (d, _) if !d.is_empty() => ServiceKind::Db,
        (_, m) if !m.is_empty() => ServiceKind::Queue,
        _ => ServiceKind::Http,
    }
}

// ================================================================ services

#[derive(Deserialize, ToSchema)]
pub struct ApmRequest {
    pub range: TimeRange,
    /// `deployment.environment`; empty or `*` for all.
    pub env: Option<String>,
    /// Optional span search (`service:x status:error @db.system:redis`).
    #[serde(default)]
    pub query: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStats {
    pub service: String,
    pub kind: ServiceKind,
    /// `telemetry.sdk.language` for instrumented services.
    pub language: Option<String>,
    pub version: Option<String>,
    pub requests: u64,
    pub req_per_sec: f64,
    pub errors: u64,
    pub error_rate: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    /// When the current version was first seen (epoch ms).
    pub last_deploy: Option<i64>,
    pub instrumented: bool,
}

#[derive(clickhouse::Row, Deserialize)]
struct EntryAgg {
    service: String,
    hits: u64,
    errors: u64,
    p50: f64,
    p95: f64,
    p99: f64,
    lang: String,
    version: String,
    web: u8,
}

#[derive(clickhouse::Row, Deserialize)]
struct PeerAgg {
    service: String,
    hits: u64,
    errors: u64,
    p50: f64,
    p95: f64,
    p99: f64,
    db_system: String,
    messaging: String,
}

#[derive(clickhouse::Row, Deserialize)]
struct VersionSeen {
    service: String,
    version: String,
    first_seen: i64,
}

async fn instrumented_services(st: &AppState, org: &str, range: TimeRange) -> Result<HashSet<String>, ApiError> {
    #[derive(clickhouse::Row, Deserialize)]
    struct Name {
        service: String,
    }
    let (clause, params) = time_clause(range);
    let rows = bind(st.ch(org).query(&format!("SELECT DISTINCT toString(ServiceName) AS service FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND {ENTRY}")), &params)
        .fetch_all::<Name>()
        .await?;
    Ok(rows.into_iter().map(|r| r.service).collect())
}

/// Every service seen in the range: instrumented (entry spans) and inferred (peers of client spans).
#[utoipa::path(post, path = "/api/v1/apm/services", request_body = ApmRequest, responses((status = 200, body = Vec<ServiceStats>)), tag = "apm")]
pub async fn services(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ApmRequest>) -> ApiResult<Vec<ServiceStats>> {
    req.range.validate()?;
    let secs = ((req.range.to - req.range.from) as f64 / 1000.0).max(1.0);
    let (clause, params) = base(req.range, &req.env, &req.query);

    let entry_q = bind(
        st.ch(&org).query(&format!(
            "SELECT toString(ServiceName) AS service, count() AS hits, countIf(StatusCode = 'Error') AS errors, \
             quantile(0.5)(Duration) / 1e6 AS p50, quantile(0.95)(Duration) / 1e6 AS p95, quantile(0.99)(Duration) / 1e6 AS p99, \
             any(ResourceAttributes['telemetry.sdk.language']) AS lang, anyLast(ResourceAttributes['service.version']) AS version, \
             toUInt8(countIf(SpanAttributes['http.route'] != '') > 0) AS web \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND {ENTRY} GROUP BY service"
        )),
        &params,
    );
    let peer_q = bind(
        st.ch(&org).query(&format!(
            "SELECT {PEER} AS service, count() AS hits, countIf(StatusCode = 'Error') AS errors, \
             quantile(0.5)(Duration) / 1e6 AS p50, quantile(0.95)(Duration) / 1e6 AS p95, quantile(0.99)(Duration) / 1e6 AS p99, \
             any(SpanAttributes['db.system']) AS db_system, any(SpanAttributes['messaging.system']) AS messaging \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND {OUTBOUND} AND {PEER} != '' GROUP BY service"
        )),
        &params,
    );
    let deploy_q = st.ch(&org).query(&format!(
        "SELECT toString(ServiceName) AS service, ResourceAttributes['service.version'] AS version, toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first_seen \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND Timestamp >= now() - INTERVAL 7 DAY AND {ENTRY} GROUP BY service, version"
    ));
    let (entries, peers, versions) = tokio::try_join!(entry_q.fetch_all::<EntryAgg>(), peer_q.fetch_all::<PeerAgg>(), deploy_q.fetch_all::<VersionSeen>())?;

    let mut latest: HashMap<String, (String, i64)> = HashMap::new();
    for v in versions {
        let e = latest.entry(v.service.clone()).or_insert((v.version.clone(), v.first_seen));
        if v.first_seen > e.1 {
            *e = (v.version, v.first_seen);
        }
    }

    let known: HashSet<String> = entries.iter().map(|e| e.service.clone()).collect();
    let mut out: Vec<ServiceStats> = entries
        .into_iter()
        .map(|e| ServiceStats {
            kind: if e.web == 1 { ServiceKind::Web } else { ServiceKind::Worker },
            language: (!e.lang.is_empty()).then_some(e.lang),
            version: (!e.version.is_empty()).then_some(e.version),
            last_deploy: latest.get(&e.service).map(|v| v.1),
            req_per_sec: e.hits as f64 / secs,
            error_rate: rate(e.errors, e.hits),
            requests: e.hits,
            errors: e.errors,
            p50_ms: finite(e.p50),
            p95_ms: finite(e.p95),
            p99_ms: finite(e.p99),
            instrumented: true,
            service: e.service,
        })
        .collect();
    out.extend(peers.into_iter().filter(|p| !known.contains(&p.service)).map(|p| ServiceStats {
        kind: inferred_kind(&p.db_system, &p.messaging),
        language: None,
        version: None,
        last_deploy: None,
        req_per_sec: p.hits as f64 / secs,
        error_rate: rate(p.errors, p.hits),
        requests: p.hits,
        errors: p.errors,
        p50_ms: finite(p.p50),
        p95_ms: finite(p.p95),
        p99_ms: finite(p.p99),
        instrumented: false,
        service: p.service,
    }));
    out.sort_by(|a, b| b.requests.cmp(&a.requests));
    Ok(Json(out))
}

// ================================================================ endpoints / resources

#[derive(Deserialize, ToSchema)]
pub struct ResourcesRequest {
    pub range: TimeRange,
    pub env: Option<String>,
    /// Restrict to one service (instrumented or inferred).
    pub service: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceStats {
    pub service: String,
    pub resource: String,
    pub requests: u64,
    pub req_per_sec: f64,
    pub errors: u64,
    pub error_rate: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    /// Share of the service's total time spent in this resource (0..1).
    pub time_share: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct ResourceAgg {
    service: String,
    resource: String,
    hits: u64,
    errors: u64,
    p50: f64,
    p95: f64,
    p99: f64,
    total: u64,
}

/// Entry-span filter for a service: its own Server/Consumer spans, or callers' spans if inferred.
fn service_filter(service: &str, instrumented: bool) -> (String, Vec<Param>, &'static str) {
    if instrumented {
        (format!("ServiceName = ? AND {ENTRY}"), vec![Param::Str(service.into())], "toString(ServiceName)")
    } else {
        (format!("{PEER} = ? AND {OUTBOUND}"), vec![Param::Str(service.into())], PEER)
    }
}

/// Endpoints (span names of entry spans) with throughput, errors, latency and time share.
#[utoipa::path(post, path = "/api/v1/apm/resources", request_body = ResourcesRequest, responses((status = 200, body = Vec<ResourceStats>)), tag = "apm")]
pub async fn resources(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ResourcesRequest>) -> ApiResult<Vec<ResourceStats>> {
    req.range.validate()?;
    let secs = ((req.range.to - req.range.from) as f64 / 1000.0).max(1.0);
    let (mut clause, mut params) = base(req.range, &req.env, "");
    let svc_expr = match &req.service {
        Some(s) => {
            let instrumented = instrumented_services(&st, &org, req.range).await?.contains(s);
            let (f, p, expr) = service_filter(s, instrumented);
            clause.push_str(&format!(" AND {f}"));
            params.extend(p);
            expr
        }
        None => {
            clause.push_str(&format!(" AND {ENTRY}"));
            "toString(ServiceName)"
        }
    };
    let rows = bind(
        st.ch(&org).query(&format!(
            "SELECT {svc_expr} AS service, toString(SpanName) AS resource, count() AS hits, countIf(StatusCode = 'Error') AS errors, \
             quantile(0.5)(Duration) / 1e6 AS p50, quantile(0.95)(Duration) / 1e6 AS p95, quantile(0.99)(Duration) / 1e6 AS p99, sum(Duration) AS total \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} GROUP BY service, resource ORDER BY hits DESC LIMIT 500"
        )),
        &params,
    )
    .fetch_all::<ResourceAgg>()
    .await?;
    let mut totals: HashMap<String, u64> = HashMap::new();
    for r in &rows {
        *totals.entry(r.service.clone()).or_default() += r.total;
    }
    Ok(Json(
        rows.into_iter()
            .map(|r| ResourceStats {
                time_share: r.total as f64 / totals.get(&r.service).copied().unwrap_or(1).max(1) as f64,
                req_per_sec: r.hits as f64 / secs,
                error_rate: rate(r.errors, r.hits),
                requests: r.hits,
                errors: r.errors,
                p50_ms: finite(r.p50),
                p95_ms: finite(r.p95),
                p99_ms: finite(r.p99),
                service: r.service,
                resource: r.resource,
            })
            .collect(),
    ))
}

// ================================================================ service summary

#[derive(Deserialize, ToSchema)]
pub struct ServiceRequest {
    pub range: TimeRange,
    pub service: String,
    pub env: Option<String>,
    /// Restrict to one resource (span name).
    pub resource: Option<String>,
    pub version: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeriesPoint {
    pub t: i64,
    pub hits: u64,
    pub errors: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Ok,
    Warn,
    Critical,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSummary {
    pub service: String,
    pub kind: ServiceKind,
    pub instrumented: bool,
    pub language: Option<String>,
    pub health: Health,
    pub requests: u64,
    pub errors: u64,
    pub error_rate: f64,
    pub req_per_sec: f64,
    pub p95_ms: f64,
    pub envs: Vec<String>,
    pub versions: Vec<String>,
    pub resources: Vec<String>,
    pub hosts: Vec<String>,
    pub series: Vec<SeriesPoint>,
    /// Bucket width of `series`, ms.
    pub bucket_ms: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct BucketAgg {
    t: i64,
    hits: u64,
    errors: u64,
    p50: f64,
    p95: f64,
    p99: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct Meta {
    envs: Vec<String>,
    versions: Vec<String>,
    resources: Vec<String>,
    hosts: Vec<String>,
    lang: String,
    web: u8,
    db_system: String,
    messaging: String,
}

pub(crate) fn bucket_ms(range: TimeRange, target_buckets: i64) -> i64 {
    let target = (range.to - range.from) / target_buckets;
    [10_000, 30_000, 60_000, 120_000, 300_000, 600_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000]
        .into_iter()
        .find(|s| *s >= target)
        .unwrap_or(43_200_000)
}

fn narrow(req: &ServiceRequest, instrumented: bool) -> (String, Vec<Param>) {
    let (mut clause, mut params) = base(req.range, &req.env, "");
    let (f, p, _) = service_filter(&req.service, instrumented);
    clause.push_str(&format!(" AND {f}"));
    params.extend(p);
    if let Some(r) = req.resource.as_deref().filter(|r| !r.is_empty()) {
        clause.push_str(" AND SpanName = ?");
        params.push(Param::Str(r.into()));
    }
    if let Some(v) = req.version.as_deref().filter(|v| !v.is_empty() && *v != "*") {
        clause.push_str(" AND ResourceAttributes['service.version'] = ?");
        params.push(Param::Str(v.into()));
    }
    (clause, params)
}

/// Header facts, health and the requests/errors/latency series for one service.
#[utoipa::path(post, path = "/api/v1/apm/service", request_body = ServiceRequest, responses((status = 200, body = ServiceSummary)), tag = "apm")]
pub async fn service(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ServiceRequest>) -> ApiResult<ServiceSummary> {
    req.range.validate()?;
    let instrumented = instrumented_services(&st, &org, req.range).await?.contains(&req.service);
    let (clause, params) = narrow(&req, instrumented);
    let size = bucket_ms(req.range, 60);
    let mut sparams = vec![Param::Int(size / 1000)];
    sparams.extend(params.clone());
    let series_q = bind(
        st.ch(&org).query(&format!(
            "SELECT toInt64(toUnixTimestamp(toStartOfInterval(Timestamp, toIntervalSecond(?)))) * 1000 AS t, count() AS hits, countIf(StatusCode = 'Error') AS errors, \
             quantile(0.5)(Duration) / 1e6 AS p50, quantile(0.95)(Duration) / 1e6 AS p95, quantile(0.99)(Duration) / 1e6 AS p99 \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} GROUP BY t ORDER BY t"
        )),
        &sparams,
    );
    // Facets for the header selects ignore resource/version narrowing so every option stays visible.
    let (mclause, mparams) = narrow(&ServiceRequest { resource: None, version: None, env: None, ..req_clone(&req) }, instrumented);
    let meta_q = bind(
        st.ch(&org).query(&format!(
            "SELECT groupUniqArray(20)(ResourceAttributes['deployment.environment']) AS envs, groupUniqArray(20)(ResourceAttributes['service.version']) AS versions, \
             groupUniqArray(50)(toString(SpanName)) AS resources, groupUniqArray(50)(ResourceAttributes['host.name']) AS hosts, \
             any(ResourceAttributes['telemetry.sdk.language']) AS lang, toUInt8(countIf(SpanAttributes['http.route'] != '') > 0) AS web, \
             any(SpanAttributes['db.system']) AS db_system, any(SpanAttributes['messaging.system']) AS messaging \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {mclause}"
        )),
        &mparams,
    );
    let (series, meta) = tokio::try_join!(series_q.fetch_all::<BucketAgg>(), meta_q.fetch_one::<Meta>())?;

    let requests: u64 = series.iter().map(|b| b.hits).sum();
    let errors: u64 = series.iter().map(|b| b.errors).sum();
    let error_rate = rate(errors, requests);
    let p95 = series.iter().filter(|b| b.hits > 0).map(|b| finite(b.p95)).fold(0.0, f64::max);
    let health = if error_rate > 0.05 { Health::Critical } else if error_rate > 0.01 { Health::Warn } else { Health::Ok };
    let kind = if instrumented {
        if meta.web == 1 {
            ServiceKind::Web
        } else {
            ServiceKind::Worker
        }
    } else {
        inferred_kind(&meta.db_system, &meta.messaging)
    };
    let clean = |mut v: Vec<String>| {
        v.retain(|s| !s.is_empty());
        v.sort();
        v
    };
    Ok(Json(ServiceSummary {
        service: req.service.clone(),
        kind,
        instrumented,
        language: (!meta.lang.is_empty()).then_some(meta.lang),
        health,
        requests,
        errors,
        error_rate,
        req_per_sec: requests as f64 / ((req.range.to - req.range.from) as f64 / 1000.0).max(1.0),
        p95_ms: p95,
        envs: clean(meta.envs),
        versions: clean(meta.versions),
        resources: clean(meta.resources),
        hosts: clean(meta.hosts),
        series: series
            .into_iter()
            .map(|b| SeriesPoint { t: b.t, hits: b.hits, errors: b.errors, p50_ms: finite(b.p50), p95_ms: finite(b.p95), p99_ms: finite(b.p99) })
            .collect(),
        bucket_ms: size,
    }))
}

fn req_clone(r: &ServiceRequest) -> ServiceRequest {
    ServiceRequest { range: r.range, service: r.service.clone(), env: r.env.clone(), resource: r.resource.clone(), version: r.version.clone() }
}

// ================================================================ deployments

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    pub version: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub requests: u64,
    pub errors: u64,
    pub error_rate: f64,
    pub p95_ms: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct DeployAgg {
    version: String,
    first_seen: i64,
    last_seen: i64,
    hits: u64,
    errors: u64,
    p95: f64,
}

/// Versions of a service seen in the range, newest first.
#[utoipa::path(post, path = "/api/v1/apm/deployments", request_body = ServiceRequest, responses((status = 200, body = Vec<Deployment>)), tag = "apm")]
pub async fn deployments(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ServiceRequest>) -> ApiResult<Vec<Deployment>> {
    req.range.validate()?;
    let (clause, params) = narrow(&ServiceRequest { version: None, ..req_clone(&req) }, true);
    let rows = bind(
        st.ch(&org).query(&format!(
            "SELECT ResourceAttributes['service.version'] AS version, toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first_seen, \
             toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last_seen, count() AS hits, countIf(StatusCode = 'Error') AS errors, quantile(0.95)(Duration) / 1e6 AS p95 \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} GROUP BY version ORDER BY first_seen DESC"
        )),
        &params,
    )
    .fetch_all::<DeployAgg>()
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| Deployment { error_rate: rate(r.errors, r.hits), version: r.version, first_seen: r.first_seen, last_seen: r.last_seen, requests: r.hits, errors: r.errors, p95_ms: finite(r.p95) })
            .collect(),
    ))
}

// ================================================================ dependencies

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub service: String,
    pub kind: ServiceKind,
    pub requests: u64,
    pub req_per_sec: f64,
    pub errors: u64,
    pub error_rate: f64,
    pub p95_ms: f64,
    /// 20-bucket sparklines across the range.
    pub error_series: Vec<u64>,
    pub p95_series: Vec<f64>,
}

#[derive(Serialize, ToSchema)]
pub struct Dependencies {
    pub upstream: Vec<Dependency>,
    pub downstream: Vec<Dependency>,
}

#[derive(clickhouse::Row, Deserialize)]
struct EdgeAgg {
    peer: String,
    bucket: i64,
    hits: u64,
    errors: u64,
    p95: f64,
    db_system: String,
    messaging: String,
}

async fn edges(st: &AppState, org: &str, req: &ServiceRequest, downstream: bool, known: &HashSet<String>) -> Result<Vec<Dependency>, ApiError> {
    const BUCKETS: i64 = 20;
    let (clause, mut params) = base(req.range, &req.env, "");
    let (peer_expr, filter) = if downstream { (PEER, "toString(ServiceName) = ?") } else { ("toString(ServiceName)", "SpanAttributes['peer.service'] = ?") };
    let size = ((req.range.to - req.range.from) / BUCKETS).max(1);
    let mut all = vec![Param::Int(req.range.from), Param::Int(size)];
    all.append(&mut params);
    all.push(Param::Str(req.service.clone()));
    let rows = bind(
        st.ch(org).query(&format!(
            "SELECT {peer_expr} AS peer, intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)) - ?, ?) AS bucket, count() AS hits, countIf(StatusCode = 'Error') AS errors, \
             quantile(0.95)(Duration) / 1e6 AS p95, any(SpanAttributes['db.system']) AS db_system, any(SpanAttributes['messaging.system']) AS messaging \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND {OUTBOUND} AND {PEER} != '' AND {filter} GROUP BY peer, bucket"
        )),
        &all,
    )
    .fetch_all::<EdgeAgg>()
    .await?;
    let secs = ((req.range.to - req.range.from) as f64 / 1000.0).max(1.0);
    let mut by_peer: HashMap<String, Dependency> = HashMap::new();
    for r in rows {
        let kind = if known.contains(&r.peer) { ServiceKind::Web } else { inferred_kind(&r.db_system, &r.messaging) };
        let d = by_peer.entry(r.peer.clone()).or_insert_with(|| Dependency {
            service: r.peer.clone(),
            kind,
            requests: 0,
            req_per_sec: 0.0,
            errors: 0,
            error_rate: 0.0,
            p95_ms: 0.0,
            error_series: vec![0; BUCKETS as usize],
            p95_series: vec![0.0; BUCKETS as usize],
        });
        d.requests += r.hits;
        d.errors += r.errors;
        d.p95_ms = d.p95_ms.max(finite(r.p95));
        if let Some(i) = usize::try_from(r.bucket).ok().filter(|i| *i < BUCKETS as usize) {
            d.error_series[i] += r.errors;
            d.p95_series[i] = finite(r.p95);
        }
    }
    let mut out: Vec<Dependency> = by_peer
        .into_values()
        .map(|mut d| {
            d.req_per_sec = d.requests as f64 / secs;
            d.error_rate = rate(d.errors, d.requests);
            d
        })
        .collect();
    out.sort_by(|a, b| b.requests.cmp(&a.requests));
    Ok(out)
}

/// Services this one calls (downstream) and services calling it (upstream).
#[utoipa::path(post, path = "/api/v1/apm/dependencies", request_body = ServiceRequest, responses((status = 200, body = Dependencies)), tag = "apm")]
pub async fn dependencies(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ServiceRequest>) -> ApiResult<Dependencies> {
    req.range.validate()?;
    let known = instrumented_services(&st, &org, req.range).await?;
    let (downstream, upstream) = tokio::try_join!(edges(&st, &org, &req, true, &known), edges(&st, &org, &req, false, &known))?;
    Ok(Json(Dependencies { upstream, downstream }))
}

// ================================================================ databases

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStats {
    pub db_system: String,
    pub db_name: String,
    pub statement: String,
    pub requests: u64,
    pub errors: u64,
    pub p95_ms: f64,
    pub time_share: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct DbAgg {
    db_system: String,
    db_name: String,
    statement: String,
    hits: u64,
    errors: u64,
    p95: f64,
    total: u64,
}

/// Database calls made by a service, grouped by system, database and statement.
#[utoipa::path(post, path = "/api/v1/apm/databases", request_body = ServiceRequest, responses((status = 200, body = Vec<DatabaseStats>)), tag = "apm")]
pub async fn databases(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ServiceRequest>) -> ApiResult<Vec<DatabaseStats>> {
    req.range.validate()?;
    let (clause, mut params) = base(req.range, &req.env, "");
    params.push(Param::Str(req.service.clone()));
    let rows = bind(
        st.ch(&org).query(&format!(
            "SELECT SpanAttributes['db.system'] AS db_system, SpanAttributes['db.name'] AS db_name, SpanAttributes['db.statement'] AS statement, \
             count() AS hits, countIf(StatusCode = 'Error') AS errors, quantile(0.95)(Duration) / 1e6 AS p95, sum(Duration) AS total \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND SpanKind = 'Client' AND ServiceName = ? AND SpanAttributes['db.system'] != '' \
             GROUP BY db_system, db_name, statement ORDER BY total DESC LIMIT 100"
        )),
        &params,
    )
    .fetch_all::<DbAgg>()
    .await?;
    let total: u64 = rows.iter().map(|r| r.total).sum::<u64>().max(1);
    Ok(Json(
        rows.into_iter()
            .map(|r| DatabaseStats { time_share: r.total as f64 / total as f64, db_system: r.db_system, db_name: r.db_name, statement: r.statement, requests: r.hits, errors: r.errors, p95_ms: finite(r.p95) })
            .collect(),
    ))
}

// ================================================================ errors

#[derive(Deserialize, ToSchema)]
pub struct ErrorsRequest {
    pub range: TimeRange,
    pub env: Option<String>,
    pub service: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanErrorGroup {
    pub service: String,
    pub resource: String,
    pub error_type: String,
    pub message: String,
    pub count: u64,
    pub first_seen: i64,
    pub last_seen: i64,
    pub sample_trace_id: String,
}

#[derive(clickhouse::Row, Deserialize)]
struct ErrAgg {
    service: String,
    resource: String,
    error_type: String,
    message: String,
    n: u64,
    first_seen: i64,
    last_seen: i64,
    sample: String,
}

/// Error spans grouped by service, resource, exception type and message.
#[utoipa::path(post, path = "/api/v1/apm/errors", request_body = ErrorsRequest, responses((status = 200, body = Vec<SpanErrorGroup>)), tag = "apm")]
pub async fn errors(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ErrorsRequest>) -> ApiResult<Vec<SpanErrorGroup>> {
    req.range.validate()?;
    let (mut clause, mut params) = base(req.range, &req.env, "");
    if let Some(s) = &req.service {
        clause.push_str(" AND ServiceName = ?");
        params.push(Param::Str(s.clone()));
    }
    let rows = bind(
        st.ch(&org).query(&format!(
            "SELECT toString(ServiceName) AS service, toString(SpanName) AS resource, \
             arrayElement(Events.Attributes, indexOf(Events.Name, 'exception'))['exception.type'] AS error_type, StatusMessage AS message, \
             count() AS n, toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first_seen, toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last_seen, any(TraceId) AS sample \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND StatusCode = 'Error' AND {ENTRY} \
             GROUP BY service, resource, error_type, message ORDER BY n DESC LIMIT 100"
        )),
        &params,
    )
    .fetch_all::<ErrAgg>()
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| SpanErrorGroup {
                service: r.service,
                resource: r.resource,
                error_type: r.error_type,
                message: r.message,
                count: r.n,
                first_seen: r.first_seen,
                last_seen: r.last_seen,
                sample_trace_id: r.sample,
            })
            .collect(),
    ))
}

// ================================================================ traces

#[derive(Deserialize, ToSchema)]
pub struct TraceSearchRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    pub env: Option<String>,
    pub limit: Option<u64>,
    /// Opaque cursor from `nextCursor`.
    pub cursor: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanRow {
    pub trace_id: String,
    pub span_id: String,
    /// Epoch ms.
    pub timestamp: i64,
    pub service: String,
    pub resource: String,
    pub kind: String,
    pub duration_ms: f64,
    pub status: String,
    pub env: String,
    pub http_status: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TraceSearchPage {
    pub spans: Vec<SpanRow>,
    pub total: u64,
    pub next_cursor: Option<String>,
}

#[derive(clickhouse::Row, Deserialize)]
struct SpanAgg {
    trace_id: String,
    span_id: String,
    ts_ns: i64,
    service: String,
    resource: String,
    kind: String,
    duration: u64,
    status: String,
    env: String,
    http_status: String,
}

#[derive(clickhouse::Row, Deserialize)]
struct Count {
    n: u64,
}

/// Service entry spans matching the query, newest first (keyset-paginated).
#[utoipa::path(post, path = "/api/v1/apm/traces/search", request_body = TraceSearchRequest, responses((status = 200, body = TraceSearchPage)), tag = "apm")]
pub async fn search_traces(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<TraceSearchRequest>) -> ApiResult<TraceSearchPage> {
    req.range.validate()?;
    let limit = req.limit.unwrap_or(100).clamp(1, 500);
    let (clause, params) = base(req.range, &req.env, &req.query);
    let clause = format!("{clause} AND {ENTRY}");
    let mut page_clause = clause.clone();
    let mut page_params = params.clone();
    if let Some(c) = &req.cursor {
        let (ts, id) = c.split_once(':').ok_or_else(|| ApiError::BadRequest("invalid cursor".into()))?;
        let ts: i64 = ts.parse().map_err(|_| ApiError::BadRequest("invalid cursor".into()))?;
        page_clause.push_str(" AND (Timestamp, SpanId) < (fromUnixTimestamp64Nano(?), ?)");
        page_params.push(Param::Int(ts));
        page_params.push(Param::Str(id.to_string()));
    }
    page_params.push(Param::Int(limit as i64));
    let rows_q = bind(
        st.ch(&org).query(&format!(
            "SELECT TraceId AS trace_id, SpanId AS span_id, toInt64(toUnixTimestamp64Nano(Timestamp)) AS ts_ns, toString(ServiceName) AS service, toString(SpanName) AS resource, \
             toString(SpanKind) AS kind, Duration AS duration, toString(StatusCode) AS status, ResourceAttributes['deployment.environment'] AS env, \
             SpanAttributes['http.response.status_code'] AS http_status \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {page_clause} ORDER BY Timestamp DESC, SpanId DESC LIMIT ?"
        )),
        &page_params,
    );
    let count_q = bind(st.ch(&org).query(&format!("SELECT count() AS n FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause}")), &params);
    let (rows, count) = tokio::try_join!(rows_q.fetch_all::<SpanAgg>(), count_q.fetch_one::<Count>())?;
    let next_cursor = (rows.len() as u64 == limit).then(|| rows.last().map(|r| format!("{}:{}", r.ts_ns, r.span_id))).flatten();
    Ok(Json(TraceSearchPage {
        spans: rows
            .into_iter()
            .map(|r| SpanRow {
                trace_id: r.trace_id,
                span_id: r.span_id,
                timestamp: r.ts_ns / 1_000_000,
                service: r.service,
                resource: r.resource,
                kind: r.kind,
                duration_ms: r.duration as f64 / 1e6,
                status: r.status,
                env: r.env,
                http_status: r.http_status,
            })
            .collect(),
        total: count.n,
        next_cursor,
    }))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TraceSpan {
    pub span_id: String,
    pub parent_span_id: String,
    pub service: String,
    pub name: String,
    pub kind: String,
    /// Offset from the trace start, microseconds.
    pub start_us: f64,
    pub duration_us: f64,
    pub status: String,
    pub status_message: String,
    pub attributes: HashMap<String, String>,
    pub resource: HashMap<String, String>,
    pub events: Vec<SpanEvent>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanEvent {
    pub name: String,
    pub offset_us: f64,
    pub attributes: HashMap<String, String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TraceDetail {
    pub trace_id: String,
    /// Epoch ms of the earliest span.
    pub start: i64,
    pub duration_ms: f64,
    pub spans: Vec<TraceSpan>,
}

#[derive(clickhouse::Row, Deserialize)]
struct Bounds {
    start: i64,
    end: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct FullSpan {
    span_id: String,
    parent: String,
    service: String,
    name: String,
    kind: String,
    start_ns: i64,
    duration: u64,
    status: String,
    message: String,
    attrs: String,
    res: String,
    ev_names: Vec<String>,
    ev_ts: Vec<i64>,
    ev_attrs: String,
}

/// All spans of one trace (waterfall / flame graph).
#[utoipa::path(get, path = "/api/v1/apm/traces/{trace_id}", params(("trace_id" = String, Path, description = "32-char hex trace id")),
    responses((status = 200, body = TraceDetail), (status = 404, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn trace(State(st): State<AppState>, OrgId(org): OrgId, Path(trace_id): Path<String>) -> Result<Json<TraceDetail>, ApiError> {
    if trace_id.len() != 32 || !trace_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest("trace id must be 32 hex characters".into()));
    }
    // The exporter's trace-id index gives the time bounds, so the main scan prunes by partition.
    let bounds = st
        .ch(&org)
        // Start/End are second-precision DateTime here; the span query below pads by a second each side.
        .query("SELECT toInt64(toUnixTimestamp(min(Start))) * 1000000000 AS start, toInt64(toUnixTimestamp(max(End))) * 1000000000 AS end FROM otel.otel_traces_trace_id_ts WHERE TraceId = ?")
        .bind(trace_id.as_str())
        .fetch_one::<Bounds>()
        .await?;
    if bounds.start <= 0 {
        return Err(ApiError::NotFound("trace not found".into()));
    }
    let spans = st
        .ch(&org)
        .query(
            "SELECT SpanId AS span_id, ParentSpanId AS parent, toString(ServiceName) AS service, toString(SpanName) AS name, toString(SpanKind) AS kind, \
             toInt64(toUnixTimestamp64Nano(Timestamp)) AS start_ns, Duration AS duration, toString(StatusCode) AS status, StatusMessage AS message, \
             toJSONString(SpanAttributes) AS attrs, toJSONString(ResourceAttributes) AS res, arrayMap(x -> toString(x), Events.Name) AS ev_names, \
             arrayMap(x -> toInt64(toUnixTimestamp64Nano(x)), Events.Timestamp) AS ev_ts, toJSONString(Events.Attributes) AS ev_attrs \
             FROM otel.otel_traces WHERE OrgId = {oc_org:String} AND TraceId = ? AND Timestamp >= fromUnixTimestamp64Nano(?) - INTERVAL 1 SECOND AND Timestamp <= fromUnixTimestamp64Nano(?) + INTERVAL 1 SECOND \
             ORDER BY Timestamp LIMIT 5000",
        )
        .bind(trace_id.as_str())
        .bind(bounds.start)
        .bind(bounds.end)
        .fetch_all::<FullSpan>()
        .await?;
    let start = spans.iter().map(|s| s.start_ns).min().unwrap_or(bounds.start);
    let end = spans.iter().map(|s| s.start_ns + s.duration as i64).max().unwrap_or(bounds.end);
    Ok(Json(TraceDetail {
        trace_id,
        start: start / 1_000_000,
        duration_ms: (end - start) as f64 / 1e6,
        spans: spans
            .into_iter()
            .map(|s| {
                let ev_attrs: Vec<HashMap<String, String>> = serde_json::from_str(&s.ev_attrs).unwrap_or_default();
                TraceSpan {
                    events: s
                        .ev_names
                        .iter()
                        .zip(s.ev_ts.iter())
                        .enumerate()
                        .map(|(i, (n, t))| SpanEvent { name: n.clone(), offset_us: (t - start) as f64 / 1e3, attributes: ev_attrs.get(i).cloned().unwrap_or_default() })
                        .collect(),
                    span_id: s.span_id,
                    parent_span_id: s.parent,
                    service: s.service,
                    name: s.name,
                    kind: s.kind,
                    start_us: (s.start_ns - start) as f64 / 1e3,
                    duration_us: s.duration as f64 / 1e3,
                    status: s.status,
                    status_message: s.message,
                    attributes: serde_json::from_str(&s.attrs).unwrap_or_default(),
                    resource: serde_json::from_str(&s.res).unwrap_or_default(),
                }
            })
            .collect(),
    }))
}

// ================================================================ recommendations & anomalies

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationKind {
    RepeatedSequentialCalls,
    HighErrorRate,
    SlowEndpoint,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub id: String,
    pub kind: RecommendationKind,
    pub priority: Priority,
    pub service: String,
    pub resource: String,
    /// Occurrences (traces affected / errors / slow requests) in the range.
    pub occurrences: u64,
    pub requests: u64,
    /// Rule-specific number (error rate, p95 ms, calls per trace).
    pub value: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct NPlusOne {
    service: String,
    resource: String,
    traces: u64,
    calls: u64,
    per_trace: f64,
}

/// Rule-based recommendations computed from spans: N+1 call patterns, error-prone and slow endpoints.
#[utoipa::path(post, path = "/api/v1/apm/recommendations", request_body = ApmRequest, responses((status = 200, body = Vec<Recommendation>)), tag = "apm")]
pub async fn recommendations(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ApmRequest>) -> ApiResult<Vec<Recommendation>> {
    req.range.validate()?;
    let (clause, params) = base(req.range, &req.env, &req.query);
    let n1_q = bind(
        st.ch(&org).query(&format!(
            "SELECT service, resource, count() AS traces, sum(n) AS calls, avg(n) AS per_trace FROM ( \
               SELECT TraceId, ParentSpanId, toString(ServiceName) AS service, toString(SpanName) AS resource, count() AS n \
               FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND SpanKind = 'Client' GROUP BY TraceId, ParentSpanId, service, resource HAVING n >= 5 \
             ) GROUP BY service, resource ORDER BY traces DESC LIMIT 10"
        )),
        &params,
    );
    let ep_q = bind(
        st.ch(&org).query(&format!(
            "SELECT toString(ServiceName) AS service, toString(SpanName) AS resource, count() AS hits, countIf(StatusCode = 'Error') AS errors, \
             quantile(0.5)(Duration) / 1e6 AS p50, quantile(0.95)(Duration) / 1e6 AS p95, quantile(0.99)(Duration) / 1e6 AS p99, sum(Duration) AS total \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND {ENTRY} GROUP BY service, resource HAVING hits >= 20"
        )),
        &params,
    );
    let (n1, eps) = tokio::try_join!(n1_q.fetch_all::<NPlusOne>(), ep_q.fetch_all::<ResourceAgg>())?;
    let mut out: Vec<Recommendation> = n1
        .into_iter()
        .map(|r| Recommendation {
            id: format!("n1:{}:{}", r.service, r.resource),
            kind: RecommendationKind::RepeatedSequentialCalls,
            priority: if r.traces >= 50 { Priority::Medium } else { Priority::Low },
            requests: r.calls,
            occurrences: r.traces,
            value: finite(r.per_trace),
            service: r.service,
            resource: r.resource,
        })
        .collect();
    for e in eps {
        let er = rate(e.errors, e.hits);
        if er >= 0.02 {
            out.push(Recommendation {
                id: format!("err:{}:{}", e.service, e.resource),
                kind: RecommendationKind::HighErrorRate,
                priority: if er >= 0.1 { Priority::High } else { Priority::Medium },
                occurrences: e.errors,
                requests: e.hits,
                value: er,
                service: e.service.clone(),
                resource: e.resource.clone(),
            });
        }
        if finite(e.p95) >= 250.0 {
            out.push(Recommendation {
                id: format!("slow:{}:{}", e.service, e.resource),
                kind: RecommendationKind::SlowEndpoint,
                priority: Priority::Low,
                occurrences: e.hits / 20,
                requests: e.hits,
                value: finite(e.p95),
                service: e.service,
                resource: e.resource,
            });
        }
    }
    let rank = |p: &Priority| match p {
        Priority::High => 0,
        Priority::Medium => 1,
        Priority::Low => 2,
    };
    out.sort_by(|a, b| rank(&a.priority).cmp(&rank(&b.priority)).then(b.occurrences.cmp(&a.occurrences)));
    Ok(Json(out))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Anomaly {
    pub service: String,
    pub resource: String,
    pub metric: String,
    pub previous: f64,
    pub current: f64,
    pub requests: u64,
}

/// Endpoints whose p95 latency or error rate rose versus the previous period of equal length.
#[utoipa::path(post, path = "/api/v1/apm/anomalies", request_body = ApmRequest, responses((status = 200, body = Vec<Anomaly>)), tag = "apm")]
pub async fn anomalies(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ApmRequest>) -> ApiResult<Vec<Anomaly>> {
    req.range.validate()?;
    let span = req.range.to - req.range.from;
    let prev = TimeRange { from: req.range.from - span, to: req.range.from };
    let q = |range: TimeRange| {
        let (clause, params) = base(range, &req.env, &req.query);
        bind(
            st.ch(&org).query(&format!(
                "SELECT toString(ServiceName) AS service, toString(SpanName) AS resource, count() AS hits, countIf(StatusCode = 'Error') AS errors, \
                 quantile(0.5)(Duration) / 1e6 AS p50, quantile(0.95)(Duration) / 1e6 AS p95, quantile(0.99)(Duration) / 1e6 AS p99, sum(Duration) AS total \
                 FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND {ENTRY} GROUP BY service, resource HAVING hits >= 20"
            )),
            &params,
        )
    };
    let (now, before) = tokio::try_join!(q(req.range).fetch_all::<ResourceAgg>(), q(prev).fetch_all::<ResourceAgg>())?;
    let before: HashMap<(String, String), ResourceAgg> = before.into_iter().map(|r| ((r.service.clone(), r.resource.clone()), r)).collect();
    let mut out = Vec::new();
    for r in now {
        let Some(b) = before.get(&(r.service.clone(), r.resource.clone())) else { continue };
        let (p_now, p_before) = (finite(r.p95), finite(b.p95));
        if p_before > 0.0 && p_now > p_before * 1.5 && p_now - p_before > 20.0 {
            out.push(Anomaly { service: r.service.clone(), resource: r.resource.clone(), metric: "p95_latency_ms".into(), previous: p_before, current: p_now, requests: r.hits });
        }
        let (e_now, e_before) = (rate(r.errors, r.hits), rate(b.errors, b.hits));
        if e_now - e_before > 0.01 {
            out.push(Anomaly { service: r.service, resource: r.resource, metric: "error_rate".into(), previous: e_before, current: e_now, requests: r.hits });
        }
    }
    Ok(Json(out))
}

// ================================================================ environments

#[derive(Deserialize, ToSchema)]
pub struct RangeRequest {
    pub range: TimeRange,
}

#[derive(clickhouse::Row, Deserialize)]
struct Envs {
    envs: Vec<String>,
}

/// `deployment.environment` values seen in the range (for env selectors).
#[utoipa::path(post, path = "/api/v1/apm/envs", request_body = RangeRequest, responses((status = 200, body = Vec<String>)), tag = "apm")]
pub async fn envs(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<RangeRequest>) -> ApiResult<Vec<String>> {
    req.range.validate()?;
    let (clause, params) = time_clause(req.range);
    let row = bind(
        st.ch(&org).query(&format!("SELECT groupUniqArray(50)(ResourceAttributes['deployment.environment']) AS envs FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause}")),
        &params,
    )
    .fetch_one::<Envs>()
    .await?;
    let mut envs: Vec<String> = row.envs.into_iter().filter(|e| !e.is_empty()).collect();
    envs.sort();
    Ok(Json(envs))
}
