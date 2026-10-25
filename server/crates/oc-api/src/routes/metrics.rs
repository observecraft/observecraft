//! Metric queries for dashboards (`avg:system.cpu.utilization{env:prod} by {host}`).
//!
//! Sources:
//! - OTel metrics written by the Collector's ClickHouse exporter (`otel.otel_metrics_gauge` / `_sum`).
//!   Gauges and non-monotonic sums are averaged per series per interval; cumulative monotonic sums
//!   (counters) become per-second rates; delta sums are summed and divided by the interval.
//! - `trace.*` metrics computed on the fly from service entry spans (see [`SPAN_METRICS`]).
//! - Familiar host metric aliases (`system.cpu.user`, `system.mem.used`, …) over the OTel names (see [`ALIASES`]).
//!
//! Evaluation follows the usual model: values are first aggregated in time per series (a series is one
//! distinct set of resource + data-point attributes), then across series in space with the query's
//! aggregator, per `by` group. Maps are sorted before hashing: exporters don't keep key order stable.
//!
//! Tenancy: like traces, the exporter's metric tables have no org column yet (single-tenant).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::Json;
use oc_query::{metric_field, metric_filter, parse_metric_query, short_tag, span_field, MetricQuery, Param};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::{bind, TimeRange};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const ENTRY: &str = "SpanKind IN ('Server', 'Consumer')";

/// `trace.*` metrics: (name, SQL value per series per interval — `{step}` is the interval in seconds, unit, description).
const SPAN_METRICS: &[(&str, &str, &str, &str)] = &[
    ("trace.requests", "count() / {step}", "req/s", "Requests handled by services (entry spans)"),
    ("trace.errors", "countIf(StatusCode = 'Error') / {step}", "err/s", "Requests that ended in an error"),
    ("trace.error_rate", "countIf(StatusCode = 'Error') * 100 / count()", "%", "Share of requests that ended in an error"),
    ("trace.latency.avg", "avg(Duration) / 1e6", "ms", "Average request latency"),
    ("trace.latency.p50", "quantile(0.5)(Duration) / 1e6", "ms", "Median request latency"),
    ("trace.latency.p95", "quantile(0.95)(Duration) / 1e6", "ms", "p95 request latency"),
    ("trace.latency.p99", "quantile(0.99)(Duration) / 1e6", "ms", "p99 request latency"),
    ("trace.latency.max", "max(Duration) / 1e6", "ms", "Slowest request"),
];
const SPAN_TAGS: &[&str] = &["service", "env", "resource", "version", "host"];

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const MIB: f64 = 1024.0 * 1024.0;

/// Familiar host metric names over the OTel `hostmetrics` names: (alias, base metric, implied
/// filter, scale, unit, description). `system.cpu.user` = `system.cpu.utilization{state:user}` × 100.
const ALIASES: &[(&str, &str, Option<(&str, &str)>, f64, &str, &str)] = &[
    ("system.cpu.user", "system.cpu.utilization", Some(("state", "user")), 100.0, "%", "CPU time in user space"),
    ("system.cpu.system", "system.cpu.utilization", Some(("state", "system")), 100.0, "%", "CPU time in the kernel"),
    ("system.cpu.iowait", "system.cpu.utilization", Some(("state", "wait")), 100.0, "%", "CPU time waiting on I/O"),
    ("system.cpu.idle", "system.cpu.utilization", Some(("state", "idle")), 100.0, "%", "Idle CPU time"),
    ("system.mem.used", "system.memory.usage", Some(("state", "used")), 1.0 / GIB, "GiB", "Memory in use"),
    ("system.mem.pct_used", "system.memory.utilization", Some(("state", "used")), 100.0, "%", "Share of memory in use"),
    ("system.load.1", "system.cpu.load_average.1m", None, 1.0, "", "Load average (1m)"),
    ("system.load.5", "system.cpu.load_average.5m", None, 1.0, "", "Load average (5m)"),
    ("system.load.15", "system.cpu.load_average.15m", None, 1.0, "", "Load average (15m)"),
    ("system.disk.in_use", "system.filesystem.utilization", None, 100.0, "%", "Share of disk space used"),
    ("system.net.bytes_rcvd", "system.network.io", Some(("direction", "receive")), 1.0, "B/s", "Bytes received per second"),
    ("system.net.bytes_sent", "system.network.io", Some(("direction", "transmit")), 1.0, "B/s", "Bytes sent per second"),
    ("process.cpu.pct", "process.cpu.utilization", None, 100.0, "%", "Share of host CPU used by the process"),
    ("process.mem.rss", "process.memory.usage", None, 1.0 / MIB, "MiB", "Process resident memory"),
];

/// Rewrites an alias to its base metric (+ implied filter); returns the value scale.
fn resolve_alias(q: &MetricQuery) -> (MetricQuery, f64) {
    match ALIASES.iter().find(|a| a.0 == q.metric) {
        Some((_, base, filter, scale, ..)) => {
            let mut r = q.clone();
            r.metric = base.to_string();
            if let Some((k, v)) = filter {
                r.filters.push(oc_query::TagFilter { key: k.to_string(), value: v.to_string(), negate: false });
            }
            (r, *scale)
        }
        None => (q.clone(), 1.0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Gauge,
    /// Non-monotonic sum (e.g. memory usage): behaves like a gauge.
    UpDown,
    Counter,
    Delta,
}

impl Kind {
    fn table(self) -> &'static str {
        match self {
            Kind::Gauge => "otel.otel_metrics_gauge",
            _ => "otel.otel_metrics_sum",
        }
    }
}

fn step_ms(range: TimeRange, points: u32, min_ms: i64) -> i64 {
    let raw = (range.to - range.from) / points.max(1) as i64;
    const STEPS: [i64; 11] = [10_000, 20_000, 30_000, 60_000, 120_000, 300_000, 600_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000];
    STEPS.iter().copied().find(|s| *s >= raw.max(min_ms)).unwrap_or(43_200_000)
}

/// Metric name → kind, cached briefly so every widget query doesn't re-probe the tables.
async fn metric_kind(st: &AppState, name: &str) -> Result<Option<Kind>, ApiError> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Kind, Instant)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((k, at)) = cache.lock().unwrap().get(name) {
        if at.elapsed() < Duration::from_secs(300) {
            return Ok(Some(*k));
        }
    }
    #[derive(clickhouse::Row, Deserialize)]
    struct Probe {
        src: String,
        temporality: i32,
        monotonic: bool,
    }
    let rows = st
        .clickhouse
        .query(
            "SELECT 'gauge' AS src, toInt32(0) AS temporality, false AS monotonic FROM otel.otel_metrics_gauge WHERE MetricName = ? LIMIT 1 \
             UNION ALL SELECT 'sum' AS src, AggregationTemporality AS temporality, IsMonotonic AS monotonic FROM otel.otel_metrics_sum WHERE MetricName = ? LIMIT 1",
        )
        .bind(name)
        .bind(name)
        .fetch_all::<Probe>()
        .await?;
    let Some(p) = rows.first() else { return Ok(None) };
    let kind = match (p.src.as_str(), p.temporality, p.monotonic) {
        ("gauge", ..) => Kind::Gauge,
        (_, 1, _) => Kind::Delta,
        (_, _, true) => Kind::Counter,
        _ => Kind::UpDown,
    };
    cache.lock().unwrap().insert(name.to_string(), (kind, Instant::now()));
    Ok(Some(kind))
}

/// One `by` group's series.
pub(crate) struct Group {
    pub tags: BTreeMap<String, String>,
    pub points: Vec<(i64, f64)>,
}

#[derive(clickhouse::Row, Deserialize)]
struct Row {
    g: Vec<String>,
    ts: i64,
    value: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct SeriesRow {
    sid: u64,
    ts: i64,
    value: f64,
}

/// Builds the inner per-series query. Returns (SQL selecting g, t, sid, v; params in text order).
async fn inner_query(st: &AppState, q: &MetricQuery, range: TimeRange, points: u32) -> Result<(String, Vec<Param>, i64), ApiError> {
    let mut params = Vec::new();
    if let Some((_, expr, _, _)) = SPAN_METRICS.iter().find(|m| m.0 == q.metric) {
        let step = step_ms(range, points, 10_000);
        let step_s = step / 1000;
        let mut by = Vec::new();
        for k in &q.by {
            let (e, p) = span_field(k);
            by.push(e);
            params.extend(p);
        }
        let f = metric_filter(&q.filters, span_field);
        params.extend([Param::Int(range.from), Param::Int(range.to)]);
        params.extend(f.params);
        let sql = format!(
            "SELECT {g} AS g, toStartOfInterval(Timestamp, INTERVAL {step_s} SECOND) AS t, cityHash64(ServiceName, SpanName, ResourceAttributes['deployment.environment']) AS sid, \
             {v} AS v FROM otel.otel_traces \
             WHERE {ENTRY} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) AND {filter} GROUP BY g, t, sid",
            g = group_array(&by),
            v = expr.replace("{step}", &step_s.to_string()),
            filter = f.clause,
        );
        return Ok((sql, params, step));
    }
    let (q, scale) = resolve_alias(q);
    let q = &q;
    let kind = metric_kind(st, &q.metric).await?.ok_or_else(|| ApiError::BadRequest(format!("Unknown metric: {}", q.metric)))?;
    // Counters need at least two samples per interval (collectors report every 10s).
    let step = step_ms(range, points, if kind == Kind::Counter { 30_000 } else { 10_000 });
    let step_s = step / 1000;
    let v = match kind {
        Kind::Gauge | Kind::UpDown => "avg(Value)".to_string(),
        Kind::Counter => "greatest(max(Value) - min(Value), 0) * 1000 / greatest(dateDiff('millisecond', min(TimeUnix), max(TimeUnix)), 1)".to_string(),
        Kind::Delta => format!("sum(Value) / {step_s}"),
    };
    let v = if scale == 1.0 { v } else { format!("({v}) * {scale:e}") };
    let mut by = Vec::new();
    for k in &q.by {
        let (e, p) = metric_field(k);
        by.push(e);
        params.extend(p);
    }
    let f = metric_filter(&q.filters, metric_field);
    params.extend([Param::Str(q.metric.clone()), Param::Int(range.from), Param::Int(range.to)]);
    params.extend(f.params);
    let having = if kind == Kind::Counter { " HAVING count() > 1" } else { "" };
    let sql = format!(
        "SELECT {g} AS g, toStartOfInterval(TimeUnix, INTERVAL {step_s} SECOND) AS t, cityHash64(toString(mapSort(ResourceAttributes)), toString(mapSort(Attributes))) AS sid, {v} AS v \
         FROM {table} WHERE MetricName = ? AND TimeUnix >= fromUnixTimestamp64Milli(?) AND TimeUnix <= fromUnixTimestamp64Milli(?) AND {filter} \
         GROUP BY g, t, sid{having}",
        g = group_array(&by),
        table = kind.table(),
        filter = f.clause,
    );
    Ok((sql, params, step))
}

fn group_array(exprs: &[String]) -> String {
    if exprs.is_empty() {
        "CAST([], 'Array(String)')".into()
    } else {
        format!("[{}]", exprs.iter().map(|e| format!("toString({e})")).collect::<Vec<_>>().join(", "))
    }
}

fn parse(query: &str) -> Result<MetricQuery, ApiError> {
    parse_metric_query(query).map_err(|e| ApiError::BadRequest(format!("Invalid query '{query}': {e}")))
}

async fn evaluate(st: &AppState, query: &str, range: TimeRange, points: u32) -> Result<(MetricQuery, Vec<Group>), ApiError> {
    range.validate()?;
    let q = parse(query)?;
    let groups = evaluate_query(st, &q, range, points).await?;
    Ok((q, groups))
}

/// Evaluates an already-built query (used by the host views, which never format user input into query text).
pub(crate) async fn evaluate_query(st: &AppState, q: &MetricQuery, range: TimeRange, points: u32) -> Result<Vec<Group>, ApiError> {
    let (inner, params, _) = inner_query(st, q, range, points).await?;
    let sql = format!(
        "SELECT g, toInt64(toUnixTimestamp(t)) * 1000 AS ts, toFloat64({agg}(v)) AS value FROM ({inner}) WHERE isFinite(v) GROUP BY g, t ORDER BY g, t",
        agg = q.agg.sql()
    );
    let rows = bind(st.clickhouse.query(&sql), &params).fetch_all::<Row>().await?;
    let mut groups: Vec<Group> = Vec::new();
    for r in rows {
        let tags: BTreeMap<String, String> = q.by.iter().cloned().zip(r.g.into_iter().map(|v| if v.is_empty() { "N/A".into() } else { v })).collect();
        match groups.last_mut() {
            Some(g) if g.tags == tags => g.points.push((r.ts, round(r.value))),
            _ => groups.push(Group { tags, points: vec![(r.ts, round(r.value))] }),
        }
    }
    Ok(groups)
}

fn round(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

fn group_label(tags: &BTreeMap<String, String>) -> String {
    if tags.is_empty() {
        "*".into()
    } else {
        tags.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(", ")
    }
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TimeAggregator {
    Avg,
    Sum,
    Min,
    Max,
    Last,
}

fn reduce(points: &[(i64, f64)], agg: TimeAggregator) -> f64 {
    let vs = points.iter().map(|p| p.1);
    match agg {
        _ if points.is_empty() => 0.0,
        TimeAggregator::Last => points[points.len() - 1].1,
        TimeAggregator::Sum => vs.sum(),
        TimeAggregator::Min => vs.fold(f64::INFINITY, f64::min),
        TimeAggregator::Max => vs.fold(f64::NEG_INFINITY, f64::max),
        TimeAggregator::Avg => vs.sum::<f64>() / points.len() as f64,
    }
}

// ================================================================ endpoints

#[derive(Deserialize, ToSchema)]
pub struct TimeseriesRequest {
    /// Queries with template variables already resolved by the client.
    pub queries: Vec<String>,
    pub range: TimeRange,
    /// Rough number of points wanted per series (default 120).
    pub points: Option<u32>,
}

#[derive(Serialize, ToSchema)]
pub struct MetricSeries {
    pub name: String,
    /// `[timestamp ms, value]` pairs.
    pub points: Vec<(i64, f64)>,
}

/// Timeseries for one or more queries; one series per `by` group.
#[utoipa::path(post, path = "/api/v1/query/timeseries", request_body = TimeseriesRequest, responses((status = 200, body = Vec<MetricSeries>)), tag = "metrics")]
pub async fn timeseries(State(st): State<AppState>, Json(req): Json<TimeseriesRequest>) -> ApiResult<Vec<MetricSeries>> {
    if req.queries.len() > 10 {
        return Err(ApiError::BadRequest("at most 10 queries per request".into()));
    }
    let points = req.points.unwrap_or(120).clamp(10, 1000);
    let results = futures::future::try_join_all(req.queries.iter().map(|q| evaluate(&st, q, req.range, points))).await?;
    let mut out = Vec::new();
    for ((q, groups), text) in results.into_iter().zip(&req.queries) {
        for g in groups {
            // Ungrouped series are named by their query so several queries stay distinct.
            out.push(MetricSeries { name: if q.by.is_empty() { text.clone() } else { group_label(&g.tags) }, points: g.points });
        }
    }
    Ok(Json(out))
}

#[derive(Deserialize, ToSchema)]
pub struct ScalarRequest {
    pub query: String,
    pub aggregator: TimeAggregator,
    pub range: TimeRange,
}

/// The query reduced to one number (groups combined per timestamp with the space aggregator first).
#[utoipa::path(post, path = "/api/v1/query/scalar", request_body = ScalarRequest, responses((status = 200, body = f64, content_type = "application/json")), tag = "metrics")]
pub async fn scalar(State(st): State<AppState>, Json(req): Json<ScalarRequest>) -> ApiResult<f64> {
    let (q, groups) = evaluate(&st, &req.query, req.range, 60).await?;
    let mut by_ts: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
    for g in &groups {
        for (t, v) in &g.points {
            by_ts.entry(*t).or_default().push(*v);
        }
    }
    let merged: Vec<(i64, f64)> = by_ts
        .into_iter()
        .map(|(t, vs)| {
            let v = match q.agg {
                oc_query::SpaceAgg::Sum => vs.iter().sum(),
                oc_query::SpaceAgg::Min => vs.iter().copied().fold(f64::INFINITY, f64::min),
                oc_query::SpaceAgg::Max => vs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                oc_query::SpaceAgg::Avg => vs.iter().sum::<f64>() / vs.len() as f64,
            };
            (t, v)
        })
        .collect();
    Ok(Json(round(reduce(&merged, req.aggregator))))
}

#[derive(Serialize, ToSchema)]
pub struct GroupedScalar {
    /// "host:a, region:b", or "*" when ungrouped.
    pub group: String,
    pub tags: BTreeMap<String, String>,
    pub value: f64,
}

/// One value per `by` group, largest first (top lists, tables, pies, treemaps).
#[utoipa::path(post, path = "/api/v1/query/grouped", request_body = ScalarRequest, responses((status = 200, body = Vec<GroupedScalar>)), tag = "metrics")]
pub async fn grouped(State(st): State<AppState>, Json(req): Json<ScalarRequest>) -> ApiResult<Vec<GroupedScalar>> {
    let (_, groups) = evaluate(&st, &req.query, req.range, 60).await?;
    let mut out: Vec<GroupedScalar> = groups
        .into_iter()
        .map(|g| GroupedScalar { group: group_label(&g.tags), value: round(reduce(&g.points, req.aggregator)), tags: g.tags })
        .collect();
    out.sort_by(|a, b| b.value.total_cmp(&a.value));
    Ok(Json(out))
}

#[derive(Deserialize, ToSchema)]
pub struct DistributionRequest {
    pub query: String,
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema)]
pub struct DistributionResponse {
    /// Bucket upper bounds.
    pub buckets: Vec<f64>,
    pub times: Vec<i64>,
    /// `[timeIndex][bucketIndex]` series counts.
    pub counts: Vec<Vec<u32>>,
}

/// How individual series' values spread over time (heatmaps, distributions).
#[utoipa::path(post, path = "/api/v1/query/distribution", request_body = DistributionRequest, responses((status = 200, body = DistributionResponse)), tag = "metrics")]
pub async fn distribution(State(st): State<AppState>, Json(req): Json<DistributionRequest>) -> ApiResult<DistributionResponse> {
    req.range.validate()?;
    let q = parse(&req.query)?;
    let (inner, params, _) = inner_query(&st, &q, req.range, 40).await?;
    let sql = format!("SELECT sid, toInt64(toUnixTimestamp(t)) * 1000 AS ts, toFloat64(v) AS value FROM ({inner}) WHERE isFinite(v) ORDER BY ts");
    let rows = bind(st.clickhouse.query(&sql), &params).fetch_all::<SeriesRow>().await?;
    const N: usize = 10;
    let max = rows.iter().map(|r| r.value).fold(0.0_f64, f64::max).max(f64::MIN_POSITIVE);
    let times: Vec<i64> = rows.iter().map(|r| r.ts).collect::<BTreeSet<_>>().into_iter().collect();
    let index: HashMap<i64, usize> = times.iter().enumerate().map(|(i, t)| (*t, i)).collect();
    let mut counts = vec![vec![0u32; N]; times.len()];
    for r in &rows {
        let b = ((r.value / max) * N as f64).floor().clamp(0.0, (N - 1) as f64) as usize;
        counts[index[&r.ts]][b] += 1;
        let _ = r.sid;
    }
    let buckets = (1..=N).map(|i| round(max * i as f64 / N as f64)).collect();
    Ok(Json(DistributionResponse { buckets, times, counts }))
}

#[derive(Serialize, ToSchema)]
pub struct MetricCatalogEntry {
    pub name: String,
    pub tags: Vec<String>,
    pub unit: Option<String>,
    pub description: String,
}

/// Metrics reported in the last day plus the span-derived `trace.*` metrics.
#[utoipa::path(get, path = "/api/v1/metrics", responses((status = 200, body = Vec<MetricCatalogEntry>)), tag = "metrics")]
pub async fn catalog(State(st): State<AppState>) -> ApiResult<Vec<MetricCatalogEntry>> {
    #[derive(clickhouse::Row, Deserialize)]
    struct Entry {
        name: String,
        unit: String,
        description: String,
        attr_keys: Vec<String>,
        res_keys: Vec<String>,
    }
    let part = |table: &str| {
        format!(
            "SELECT MetricName AS name, any(MetricUnit) AS unit, any(MetricDescription) AS description, \
             arrayDistinct(arrayFlatten(groupUniqArray(mapKeys(Attributes)))) AS attr_keys, \
             arrayDistinct(arrayFlatten(groupUniqArray(mapKeys(ResourceAttributes)))) AS res_keys \
             FROM {table} WHERE TimeUnix > now() - INTERVAL 1 DAY GROUP BY MetricName"
        )
    };
    let sql = format!("{} UNION ALL {}", part("otel.otel_metrics_gauge"), part("otel.otel_metrics_sum"));
    let rows = st.clickhouse.query(&sql).fetch_all::<Entry>().await?;
    let mut out: Vec<MetricCatalogEntry> = rows
        .into_iter()
        .map(|e| {
            let mut tags: BTreeSet<String> = e.attr_keys.into_iter().collect();
            for k in e.res_keys {
                tags.insert(short_tag(&k).map(str::to_string).unwrap_or(k));
            }
            MetricCatalogEntry { name: e.name, tags: tags.into_iter().collect(), unit: Some(e.unit).filter(|u| !u.is_empty() && u != "1"), description: e.description }
        })
        .collect();
    let aliases: Vec<MetricCatalogEntry> = ALIASES
        .iter()
        .filter_map(|(alias, base, filter, _, unit, desc)| {
            let b = out.iter().find(|e| e.name == *base)?;
            let tags = b.tags.iter().filter(|t| filter.map_or(true, |(k, _)| *t != k)).cloned().collect();
            Some(MetricCatalogEntry { name: alias.to_string(), tags, unit: Some(unit.to_string()).filter(|u| !u.is_empty()), description: desc.to_string() })
        })
        .collect();
    out.extend(aliases);
    out.extend(SPAN_METRICS.iter().map(|(name, _, unit, desc)| MetricCatalogEntry {
        name: name.to_string(),
        tags: SPAN_TAGS.iter().map(|s| s.to_string()).collect(),
        unit: Some(unit.to_string()),
        description: desc.to_string(),
    }));
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(out))
}

/// Values seen for a tag in the last day, across metrics and spans (template variable pickers).
#[utoipa::path(get, path = "/api/v1/tags/{key}/values", params(("key" = String, Path)), responses((status = 200, body = Vec<String>)), tag = "metrics")]
pub async fn tag_values(State(st): State<AppState>, Path(key): Path<String>) -> ApiResult<Vec<String>> {
    if key.is_empty() || key.len() > 200 || !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/')) {
        return Err(ApiError::BadRequest("invalid tag key".into()));
    }
    let (m_expr, m_params) = metric_field(&key);
    let mut params = m_params.clone();
    params.extend(m_params);
    let mut sql = format!(
        "SELECT DISTINCT v FROM (\
           SELECT toString({m_expr}) AS v FROM otel.otel_metrics_gauge WHERE TimeUnix > now() - INTERVAL 1 DAY \
           UNION ALL SELECT toString({m_expr}) AS v FROM otel.otel_metrics_sum WHERE TimeUnix > now() - INTERVAL 1 DAY"
    );
    if SPAN_TAGS.contains(&key.as_str()) {
        let (s_expr, s_params) = span_field(&key);
        sql.push_str(&format!(" UNION ALL SELECT toString({s_expr}) AS v FROM otel.otel_traces WHERE Timestamp > now() - INTERVAL 1 HOUR"));
        params.extend(s_params);
    }
    sql.push_str(") WHERE v != '' ORDER BY v LIMIT 500");
    let values = bind(st.clickhouse.query(&sql), &params).fetch_all::<String>().await?;
    Ok(Json(values))
}
