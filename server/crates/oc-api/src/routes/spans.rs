//! Span explorer: search over every span (or service entry spans, or whole traces), facet counts,
//! analytics (timeseries / grouped values for the explorer's visualizations), the requests /
//! errors / latency summary and a span's latency percentile among its peers.
//!
//! Filters combine the search query (`service:billing @http.route:/orders/* -env:qa`, see
//! `oc_query::SpansSchema`), facet selections and a duration range.

use std::collections::{BTreeMap, HashMap};

use axum::extract::State;
use axum::Json;
use oc_query::{build_where, parse_query, Param, SpansSchema};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::{bind, TimeRange};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const ENTRY: &str = "SpanKind IN ('Server', 'Consumer')";
const METHOD: &str = "if(SpanAttributes['http.request.method'] != '', SpanAttributes['http.request.method'], SpanAttributes['http.method'])";
const HTTP_STATUS: &str = "if(SpanAttributes['http.response.status_code'] != '', SpanAttributes['http.response.status_code'], SpanAttributes['http.status_code'])";

/// Facet / group-by dimensions: (API name, SQL expression).
const DIMENSIONS: &[(&str, &str)] = &[
    ("service", "toString(ServiceName)"),
    ("resource", "toString(SpanName)"),
    ("env", "ResourceAttributes['deployment.environment']"),
    ("status", "if(StatusCode = 'Error', 'error', 'ok')"),
    ("http_status", HTTP_STATUS),
    ("method", METHOD),
    ("kind", "lower(toString(SpanKind))"),
    ("host", "ResourceAttributes['host.name']"),
    ("db_system", "SpanAttributes['db.system']"),
    ("version", "ResourceAttributes['service.version']"),
];

fn dimension(name: &str) -> Result<&'static str, ApiError> {
    DIMENSIONS.iter().find(|d| d.0 == name).map(|d| d.1).ok_or_else(|| ApiError::BadRequest(format!("unknown dimension '{name}'")))
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpanScope {
    /// Every span.
    #[default]
    All,
    /// Spans where a request enters a service (server / consumer spans).
    ServiceEntry,
    /// Root spans: one row per trace.
    Traces,
}

#[derive(Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SpanQuery {
    pub range: TimeRange,
    #[serde(default)]
    pub scope: SpanScope,
    #[serde(default)]
    pub query: String,
    /// Facet selections: dimension → accepted values (see the facets response for dimension names).
    #[serde(default)]
    pub filters: BTreeMap<String, Vec<String>>,
    pub min_duration_ms: Option<f64>,
    pub max_duration_ms: Option<f64>,
}

/// WHERE clause and params; `skip` ignores one facet's own selection (for its counts).
fn where_clause(q: &SpanQuery, skip: Option<&str>) -> Result<(String, Vec<Param>), ApiError> {
    q.range.validate()?;
    let mut parts = vec!["Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)".to_string()];
    let mut params = vec![Param::Int(q.range.from), Param::Int(q.range.to)];
    match q.scope {
        SpanScope::All => {}
        SpanScope::ServiceEntry => parts.push(ENTRY.into()),
        SpanScope::Traces => parts.push("ParentSpanId = ''".into()),
    }
    let f = build_where(&SpansSchema, &parse_query(&q.query), None);
    parts.push(format!("({})", f.clause));
    params.extend(f.params);
    for (dim, values) in &q.filters {
        if values.is_empty() || Some(dim.as_str()) == skip {
            continue;
        }
        let expr = dimension(dim)?;
        parts.push(format!("{expr} IN ({})", vec!["?"; values.len()].join(", ")));
        params.extend(values.iter().map(|v| Param::Str(v.clone())));
    }
    if let Some(min) = q.min_duration_ms.filter(|v| v.is_finite() && *v > 0.0) {
        parts.push("Duration >= ?".into());
        params.push(Param::Int((min * 1e6) as i64));
    }
    if let Some(max) = q.max_duration_ms.filter(|v| v.is_finite() && *v > 0.0) {
        parts.push("Duration <= ?".into());
        params.push(Param::Int((max * 1e6) as i64));
    }
    Ok((parts.join(" AND "), params))
}

// ================================================================ search

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanSearchRequest {
    #[serde(flatten)]
    pub q: SpanQuery,
    /// date (default) | duration
    pub sort: Option<String>,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, Deserialize, ToSchema, clickhouse::Row)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerSpan {
    pub trace_id: String,
    pub span_id: String,
    /// Epoch ms.
    pub timestamp: i64,
    pub service: String,
    pub resource: String,
    pub kind: String,
    pub duration_ms: f64,
    /// ok | error
    pub status: String,
    pub env: String,
    pub method: String,
    pub http_status: String,
    /// `db.system` of client database spans (mongodb, postgresql, redis, …).
    pub db_system: String,
    pub peer_service: String,
    pub host: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TraceSummaryInfo {
    pub spans: u64,
    pub services: u64,
    pub errors: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanSearchResult {
    pub spans: Vec<ExplorerSpan>,
    pub total: u64,
    /// Matching spans per second over the range.
    pub per_second: f64,
    /// Traces scope: per trace id, the trace's span/service/error counts.
    pub traces: HashMap<String, TraceSummaryInfo>,
}

const ROW: &str = "TraceId AS trace_id, SpanId AS span_id, toInt64(toUnixTimestamp64Milli(Timestamp)) AS timestamp, toString(ServiceName) AS service, \
    toString(SpanName) AS resource, lower(toString(SpanKind)) AS kind, Duration / 1e6 AS duration_ms, if(StatusCode = 'Error', 'error', 'ok') AS status, \
    ResourceAttributes['deployment.environment'] AS env, {METHOD} AS method, {HTTP_STATUS} AS http_status, SpanAttributes['db.system'] AS db_system, \
    SpanAttributes['peer.service'] AS peer_service, ResourceAttributes['host.name'] AS host";

#[utoipa::path(post, path = "/api/v1/apm/spans/search", operation_id = "search_spans", request_body = SpanSearchRequest, responses((status = 200, body = SpanSearchResult)), tag = "apm")]
pub async fn search(State(st): State<AppState>, OrgId(_org): OrgId, Json(req): Json<SpanSearchRequest>) -> ApiResult<SpanSearchResult> {
    let (w, params) = where_clause(&req.q, None)?;
    let order = if req.sort.as_deref() == Some("duration") { "Duration DESC" } else { "Timestamp DESC" };
    let row = ROW.replace("{METHOD}", METHOD).replace("{HTTP_STATUS}", HTTP_STATUS);
    let sql = format!("SELECT {row} FROM otel.otel_traces WHERE {w} ORDER BY {order} LIMIT {} OFFSET {}", req.limit.clamp(1, 500), req.offset.clamp(0, 10_000));
    let rows_q = bind(st.clickhouse.query(&sql), &params);
    let count_q = bind(st.clickhouse.query(&format!("SELECT count() FROM otel.otel_traces WHERE {w}")), &params);
    let (spans, total) = tokio::try_join!(rows_q.fetch_all::<ExplorerSpan>(), count_q.fetch_one::<u64>())?;
    let mut traces = HashMap::new();
    if req.q.scope == SpanScope::Traces && !spans.is_empty() {
        #[derive(clickhouse::Row, Deserialize)]
        struct T {
            id: String,
            spans: u64,
            services: u64,
            errors: u64,
        }
        let ids: Vec<String> = spans.iter().map(|s| s.trace_id.clone()).collect();
        // Whole traces can start before / end after the range: widen it a little.
        let rows = st
            .clickhouse
            .query(
                "SELECT TraceId AS id, count() AS spans, uniqExact(ServiceName) AS services, countIf(StatusCode = 'Error') AS errors FROM otel.otel_traces \
                 WHERE has(?, TraceId) AND Timestamp >= fromUnixTimestamp64Milli(?) - INTERVAL 5 MINUTE AND Timestamp <= fromUnixTimestamp64Milli(?) + INTERVAL 5 MINUTE GROUP BY id",
            )
            .bind(ids)
            .bind(req.q.range.from)
            .bind(req.q.range.to)
            .fetch_all::<T>()
            .await?;
        traces = rows.into_iter().map(|t| (t.id, TraceSummaryInfo { spans: t.spans, services: t.services, errors: t.errors })).collect();
    }
    let secs = ((req.q.range.to - req.q.range.from) as f64 / 1000.0).max(1.0);
    Ok(Json(SpanSearchResult { spans, total, per_second: (total as f64 / secs * 100.0).round() / 100.0, traces }))
}

// ================================================================ facets

#[derive(Serialize, ToSchema, Clone)]
pub struct SpanFacetValue {
    pub value: String,
    pub count: u64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanFacetsRequest {
    #[serde(flatten)]
    pub q: SpanQuery,
    /// Dimensions to count (default: all).
    #[serde(default)]
    pub dimensions: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanFacets {
    /// Dimension → top values with counts (each ignoring its own selection).
    pub facets: BTreeMap<String, Vec<SpanFacetValue>>,
    pub min_duration_ms: f64,
    pub max_duration_ms: f64,
}

#[utoipa::path(post, operation_id = "span_facets", path = "/api/v1/apm/spans/facets", request_body = SpanFacetsRequest, responses((status = 200, body = SpanFacets)), tag = "apm")]
pub async fn facets(State(st): State<AppState>, OrgId(_org): OrgId, Json(req): Json<SpanFacetsRequest>) -> ApiResult<SpanFacets> {
    #[derive(clickhouse::Row, Deserialize)]
    struct V {
        value: String,
        count: u64,
    }
    let dims: Vec<&str> = if req.dimensions.is_empty() { DIMENSIONS.iter().map(|d| d.0).collect() } else { req.dimensions.iter().map(String::as_str).collect() };
    let mut jobs = Vec::new();
    for d in dims {
        let expr = dimension(d)?;
        let (w, params) = where_clause(&req.q, Some(d))?;
        let st = st.clone();
        jobs.push(async move {
            let sql = format!("SELECT toString({expr}) AS value, count() AS count FROM otel.otel_traces WHERE {w} AND value != '' GROUP BY value ORDER BY count DESC LIMIT 50");
            let rows = bind(st.clickhouse.query(&sql), &params).fetch_all::<V>().await?;
            Ok::<_, ApiError>((d.to_string(), rows.into_iter().map(|v| SpanFacetValue { value: v.value, count: v.count }).collect::<Vec<_>>()))
        });
    }
    let results = futures::future::try_join_all(jobs).await?;
    #[derive(clickhouse::Row, Deserialize)]
    struct MinMax {
        lo: f64,
        hi: f64,
    }
    let (w, params) = where_clause(&SpanQuery { min_duration_ms: None, max_duration_ms: None, ..req.q.clone() }, None)?;
    let mm = bind(st.clickhouse.query(&format!("SELECT min(Duration) / 1e6 AS lo, max(Duration) / 1e6 AS hi FROM otel.otel_traces WHERE {w}")), &params)
        .fetch_one::<MinMax>()
        .await?;
    Ok(Json(SpanFacets { facets: results.into_iter().collect(), min_duration_ms: finite(mm.lo), max_duration_ms: finite(mm.hi) }))
}

fn finite(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

// ================================================================ analytics

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpanMeasure {
    #[default]
    Count,
    Errors,
    Avg,
    P50,
    P95,
    P99,
    Max,
}

impl SpanMeasure {
    fn sql(self) -> &'static str {
        match self {
            SpanMeasure::Count => "toFloat64(count())",
            SpanMeasure::Errors => "toFloat64(countIf(StatusCode = 'Error'))",
            SpanMeasure::Avg => "avg(Duration) / 1e6",
            SpanMeasure::P50 => "quantile(0.5)(Duration) / 1e6",
            SpanMeasure::P95 => "quantile(0.95)(Duration) / 1e6",
            SpanMeasure::P99 => "quantile(0.99)(Duration) / 1e6",
            SpanMeasure::Max => "max(Duration) / 1e6",
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanAnalyticsRequest {
    #[serde(flatten)]
    pub q: SpanQuery,
    /// Dimension to group by (none = one series).
    pub group_by: Option<String>,
    #[serde(default)]
    pub measure: SpanMeasure,
    /// Number of groups to keep (default 10).
    pub top: Option<usize>,
}

#[derive(Serialize, ToSchema)]
pub struct AnalyticsSeries {
    pub group: String,
    /// One value per bucket (null when the bucket has no spans).
    pub values: Vec<Option<f64>>,
    /// The measure over the whole range.
    pub total: f64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanAnalytics {
    pub step_ms: i64,
    pub times: Vec<i64>,
    pub series: Vec<AnalyticsSeries>,
}

fn step_for(range: TimeRange, buckets: i64) -> i64 {
    let raw = (range.to - range.from) / buckets.max(1);
    const STEPS: [i64; 12] = [10_000, 30_000, 60_000, 120_000, 300_000, 600_000, 1_800_000, 3_600_000, 3 * 3_600_000, 6 * 3_600_000, 12 * 3_600_000, 86_400_000];
    STEPS.iter().copied().find(|s| *s >= raw).unwrap_or(86_400_000)
}

#[utoipa::path(post, operation_id = "span_analytics", path = "/api/v1/apm/spans/analytics", request_body = SpanAnalyticsRequest, responses((status = 200, body = SpanAnalytics)), tag = "apm")]
pub async fn analytics(State(st): State<AppState>, OrgId(_org): OrgId, Json(req): Json<SpanAnalyticsRequest>) -> ApiResult<SpanAnalytics> {
    let (w, params) = where_clause(&req.q, None)?;
    let group = match &req.group_by {
        Some(g) if !g.is_empty() => format!("toString({})", dimension(g)?),
        _ => "'all'".to_string(),
    };
    let measure = req.measure.sql();
    let top = req.top.unwrap_or(10).clamp(1, 50);
    #[derive(clickhouse::Row, Deserialize)]
    struct Total {
        g: String,
        v: f64,
    }
    // Top groups by span count, with the measure over the whole range.
    let totals = bind(
        st.clickhouse.query(&format!("SELECT {group} AS g, {measure} AS v FROM otel.otel_traces WHERE {w} GROUP BY g ORDER BY count() DESC LIMIT {top}")),
        &params,
    )
    .fetch_all::<Total>()
    .await?;
    let step = step_for(req.q.range, 60);
    let first = req.q.range.from / step * step;
    let times: Vec<i64> = (0..).map(|i| first + i * step).take_while(|t| *t <= req.q.range.to).collect();
    let index: HashMap<i64, usize> = times.iter().enumerate().map(|(i, t)| (*t, i)).collect();
    #[derive(clickhouse::Row, Deserialize)]
    struct B {
        g: String,
        t: i64,
        v: f64,
    }
    let groups: Vec<String> = totals.iter().map(|t| t.g.clone()).collect();
    let mut bparams = params.clone();
    bparams.extend(groups.iter().map(|g| Param::Str(g.clone())));
    let in_list = if groups.is_empty() { "''".to_string() } else { vec!["?"; groups.len()].join(", ") };
    let rows = bind(
        st.clickhouse.query(&format!(
            "SELECT {group} AS g, intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)), {step}) * {step} AS t, {measure} AS v \
             FROM otel.otel_traces WHERE {w} AND g IN ({in_list}) GROUP BY g, t"
        )),
        &bparams,
    )
    .fetch_all::<B>()
    .await?;
    let mut by: HashMap<String, Vec<Option<f64>>> = groups.iter().map(|g| (g.clone(), vec![None; times.len()])).collect();
    for r in rows {
        if let (Some(i), Some(vals)) = (index.get(&r.t), by.get_mut(&r.g)) {
            vals[*i] = Some(round(r.v));
        }
    }
    // Counts are zero (not missing) in empty buckets.
    if matches!(req.measure, SpanMeasure::Count | SpanMeasure::Errors) {
        for vals in by.values_mut() {
            for v in vals.iter_mut() {
                v.get_or_insert(0.0);
            }
        }
    }
    let series = totals.into_iter().map(|t| AnalyticsSeries { values: by.remove(&t.g).unwrap_or_default(), total: round(t.v), group: if t.g.is_empty() { "N/A".into() } else { t.g } }).collect();
    Ok(Json(SpanAnalytics { step_ms: step, times, series }))
}

fn round(v: f64) -> f64 {
    if v.is_finite() {
        (v * 1000.0).round() / 1000.0
    } else {
        0.0
    }
}

// ================================================================ requests / errors / latency

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanSummary {
    pub step_ms: i64,
    pub times: Vec<i64>,
    pub hits: Vec<u64>,
    pub errors: Vec<u64>,
    pub p50_ms: Vec<Option<f64>>,
    pub p95_ms: Vec<Option<f64>>,
    pub p99_ms: Vec<Option<f64>>,
    /// p95 over the whole range ("high latency" threshold).
    pub overall_p95_ms: f64,
}

#[utoipa::path(post, operation_id = "span_summary", path = "/api/v1/apm/spans/summary", request_body = SpanQuery, responses((status = 200, body = SpanSummary)), tag = "apm")]
pub async fn summary(State(st): State<AppState>, OrgId(_org): OrgId, Json(q): Json<SpanQuery>) -> ApiResult<SpanSummary> {
    let (w, params) = where_clause(&q, None)?;
    let step = step_for(q.range, 40);
    #[derive(clickhouse::Row, Deserialize)]
    struct R {
        t: i64,
        hits: u64,
        errors: u64,
        p50: f64,
        p95: f64,
        p99: f64,
    }
    let rows = bind(
        st.clickhouse.query(&format!(
            "SELECT intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)), {step}) * {step} AS t, count() AS hits, countIf(StatusCode = 'Error') AS errors, \
             quantile(0.5)(Duration) / 1e6 AS p50, quantile(0.95)(Duration) / 1e6 AS p95, quantile(0.99)(Duration) / 1e6 AS p99 \
             FROM otel.otel_traces WHERE {w} GROUP BY t ORDER BY t"
        )),
        &params,
    )
    .fetch_all::<R>()
    .await?;
    let overall = bind(st.clickhouse.query(&format!("SELECT quantile(0.95)(Duration) / 1e6 FROM otel.otel_traces WHERE {w}")), &params).fetch_one::<f64>().await?;
    let first = q.range.from / step * step;
    let times: Vec<i64> = (0..).map(|i| first + i * step).take_while(|t| *t <= q.range.to).collect();
    let n = times.len();
    let mut out = SpanSummary { step_ms: step, hits: vec![0; n], errors: vec![0; n], p50_ms: vec![None; n], p95_ms: vec![None; n], p99_ms: vec![None; n], overall_p95_ms: round(overall), times };
    for r in rows {
        let i = ((r.t - first) / step) as usize;
        if i < n {
            out.hits[i] = r.hits;
            out.errors[i] = r.errors;
            out.p50_ms[i] = Some(round(r.p50));
            out.p95_ms[i] = Some(round(r.p95));
            out.p99_ms[i] = Some(round(r.p99));
        }
    }
    Ok(Json(out))
}

// ================================================================ percentile

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanPercentileRequest {
    pub service: String,
    pub resource: String,
    pub duration_ms: f64,
    /// Epoch ms of the span; peers are taken from the hour before.
    pub at: i64,
}

#[derive(Serialize, ToSchema)]
pub struct SpanPercentile {
    /// 0–100: share of same-service, same-resource spans in the previous hour that were faster.
    pub percentile: f64,
    pub peers: u64,
}

#[utoipa::path(post, operation_id = "span_percentile", path = "/api/v1/apm/spans/percentile", request_body = SpanPercentileRequest, responses((status = 200, body = SpanPercentile)), tag = "apm")]
pub async fn percentile(State(st): State<AppState>, OrgId(_org): OrgId, Json(req): Json<SpanPercentileRequest>) -> ApiResult<SpanPercentile> {
    #[derive(clickhouse::Row, Deserialize)]
    struct R {
        faster: u64,
        total: u64,
    }
    let r = st
        .clickhouse
        .query(
            "SELECT countIf(Duration < ?) AS faster, count() AS total FROM otel.otel_traces \
             WHERE ServiceName = ? AND SpanName = ? AND Timestamp >= fromUnixTimestamp64Milli(?) - INTERVAL 1 HOUR AND Timestamp <= fromUnixTimestamp64Milli(?) + INTERVAL 1 MINUTE",
        )
        .bind((req.duration_ms * 1e6) as i64)
        .bind(&req.service)
        .bind(&req.resource)
        .bind(req.at)
        .bind(req.at)
        .fetch_one::<R>()
        .await?;
    let percentile = if r.total == 0 { 0.0 } else { (r.faster as f64 * 100.0 / r.total as f64).round() };
    Ok(Json(SpanPercentile { percentile, peers: r.total }))
}
