//! Metric queries for dashboards (`avg:system.cpu.utilization{env:prod} by {host}`).
//!
//! Sources:
//! - OTel metrics written by the Collector's ClickHouse exporter (`otel.otel_metrics_*`).
//!   Gauges and non-monotonic sums are averaged per series per interval; cumulative monotonic sums
//!   (counters) become per-second rates; delta sums are summed and divided by the interval.
//!   Histograms, exponential histograms and summaries are queried through a statistic suffix:
//!   `<name>.count` / `.sum` (per second), `.avg`, `.min`, `.max`, `.p50` `.p75` `.p90` `.p95` `.p99`.
//!   Cumulative series are turned into per-interval deltas first. Histogram percentiles merge the
//!   bucket counts of every series in a group, then interpolate linearly inside the bucket;
//!   exponential histograms do the same per series (bucket layouts differ between series) and the
//!   query's aggregator combines series; summaries use the quantiles the SDK reported.
//! - `trace.*` metrics computed on the fly from service entry spans (see [`SPAN_METRICS`]).
//! - Familiar host metric aliases (`system.cpu.user`, `system.mem.used`, …) over the OTel names (see [`ALIASES`]).
//!
//! Evaluation follows the usual model: values are first aggregated in time per series (a series is one
//! distinct set of resource + data-point attributes), then across series in space with the query's
//! aggregator, per `by` group. Maps are sorted before hashing: exporters don't keep key order stable.
//!
//! Tenancy: every query filters on `OrgId = {oc_org:String}` (see `AppState::ch`).

#[path = "metrics/dashboard.rs"]
pub mod dashboard;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::Json;
use oc_query::{
    metric_field, metric_filter, parse_metric_query, short_tag, span_field, MetricQuery, Param,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::{bind, TimeRange};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const ENTRY: &str = "SpanKind IN ('Server', 'Consumer')";

/// `trace.*` metrics: (name, SQL value per series per interval — `{step}` is the interval in seconds, unit, description).
const SPAN_METRICS: &[(&str, &str, &str, &str)] = &[
    (
        "trace.requests",
        "count() / {step}",
        "req/s",
        "Requests handled by services (entry spans)",
    ),
    (
        "trace.errors",
        "countIf(StatusCode = 'Error') / {step}",
        "err/s",
        "Requests that ended in an error",
    ),
    (
        "trace.error_rate",
        "countIf(StatusCode = 'Error') * 100 / count()",
        "%",
        "Share of requests that ended in an error",
    ),
    (
        "trace.latency.avg",
        "avg(Duration) / 1e6",
        "ms",
        "Average request latency",
    ),
    (
        "trace.latency.p50",
        "quantile(0.5)(Duration) / 1e6",
        "ms",
        "Median request latency",
    ),
    (
        "trace.latency.p95",
        "quantile(0.95)(Duration) / 1e6",
        "ms",
        "p95 request latency",
    ),
    (
        "trace.latency.p99",
        "quantile(0.99)(Duration) / 1e6",
        "ms",
        "p99 request latency",
    ),
    (
        "trace.latency.max",
        "max(Duration) / 1e6",
        "ms",
        "Slowest request",
    ),
];
const SPAN_TAGS: &[&str] = &["service", "env", "resource", "version", "host"];

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const MIB: f64 = 1024.0 * 1024.0;

/// Familiar host metric names over the OTel `hostmetrics` names: (alias, base metric, implied
/// filter, scale, unit, description). `system.cpu.user` = `system.cpu.utilization{state:user}` × 100.
const ALIASES: &[(&str, &str, Option<(&str, &str)>, f64, &str, &str)] = &[
    (
        "system.cpu.user",
        "system.cpu.utilization",
        Some(("state", "user")),
        100.0,
        "%",
        "CPU time in user space",
    ),
    (
        "system.cpu.system",
        "system.cpu.utilization",
        Some(("state", "system")),
        100.0,
        "%",
        "CPU time in the kernel",
    ),
    (
        "system.cpu.iowait",
        "system.cpu.utilization",
        Some(("state", "wait")),
        100.0,
        "%",
        "CPU time waiting on I/O",
    ),
    (
        "system.cpu.idle",
        "system.cpu.utilization",
        Some(("state", "idle")),
        100.0,
        "%",
        "Idle CPU time",
    ),
    (
        "system.mem.used",
        "system.memory.usage",
        Some(("state", "used")),
        1.0 / GIB,
        "GiB",
        "Memory in use",
    ),
    (
        "system.mem.pct_used",
        "system.memory.utilization",
        Some(("state", "used")),
        100.0,
        "%",
        "Share of memory in use",
    ),
    (
        "system.load.1",
        "system.cpu.load_average.1m",
        None,
        1.0,
        "",
        "Load average (1m)",
    ),
    (
        "system.load.5",
        "system.cpu.load_average.5m",
        None,
        1.0,
        "",
        "Load average (5m)",
    ),
    (
        "system.load.15",
        "system.cpu.load_average.15m",
        None,
        1.0,
        "",
        "Load average (15m)",
    ),
    (
        "system.disk.in_use",
        "system.filesystem.utilization",
        None,
        100.0,
        "%",
        "Share of disk space used",
    ),
    (
        "system.net.bytes_rcvd",
        "system.network.io",
        Some(("direction", "receive")),
        1.0,
        "B/s",
        "Bytes received per second",
    ),
    (
        "system.net.bytes_sent",
        "system.network.io",
        Some(("direction", "transmit")),
        1.0,
        "B/s",
        "Bytes sent per second",
    ),
    (
        "process.cpu.pct",
        "process.cpu.utilization",
        None,
        100.0,
        "%",
        "Share of host CPU used by the process",
    ),
    (
        "process.mem.rss",
        "process.memory.usage",
        None,
        1.0 / MIB,
        "MiB",
        "Process resident memory",
    ),
];

/// Rewrites an alias to its base metric (+ implied filter); returns the value scale.
fn resolve_alias(q: &MetricQuery) -> (MetricQuery, f64) {
    match ALIASES.iter().find(|a| a.0 == q.metric) {
        Some((_, base, filter, scale, ..)) => {
            let mut r = q.clone();
            r.metric = base.to_string();
            if let Some((k, v)) = filter {
                r.filters.push(oc_query::TagFilter {
                    key: k.to_string(),
                    value: v.to_string(),
                    negate: false,
                });
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

/// Distribution metric families (queried with a statistic suffix).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dist {
    Histogram,
    ExpHistogram,
    Summary,
}

impl Dist {
    fn table(self) -> &'static str {
        match self {
            Dist::Histogram => "otel.otel_metrics_histogram",
            Dist::ExpHistogram => "otel.otel_metrics_exponential_histogram",
            Dist::Summary => "otel.otel_metrics_summary",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Dist::Histogram => "histogram",
            Dist::ExpHistogram => "exponential_histogram",
            Dist::Summary => "summary",
        }
    }
}

/// Statistics a distribution metric answers, as name suffixes.
const DIST_STATS: &[(&str, &str)] = &[
    ("count", "Observations per second"),
    ("sum", "Sum of observed values per second"),
    ("avg", "Average observed value"),
    ("min", "Smallest observed value"),
    ("max", "Largest observed value"),
    ("p50", "Median"),
    ("p75", "75th percentile"),
    ("p90", "90th percentile"),
    ("p95", "95th percentile"),
    ("p99", "99th percentile"),
];

/// `http.server.duration.p95` → (`http.server.duration`, `p95`).
fn split_stat(name: &str) -> Option<(&str, &str)> {
    let (base, stat) = name.rsplit_once('.')?;
    DIST_STATS
        .iter()
        .any(|(s, _)| *s == stat)
        .then_some((base, stat))
}

fn quantile_of(stat: &str) -> Option<f64> {
    stat.strip_prefix('p')
        .and_then(|n| n.parse::<f64>().ok())
        .map(|n| n / 100.0)
}

/// Which distribution table holds `name` for this org, cached like [`metric_kind`].
async fn dist_kind(st: &AppState, org: &str, name: &str) -> Result<Option<Dist>, ApiError> {
    type Cache = Mutex<HashMap<String, (Option<Dist>, Instant)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = format!("{org}\u{0}{name}");
    if let Some((d, at)) = cache.lock().unwrap().get(&key) {
        if at.elapsed() < Duration::from_secs(300) {
            return Ok(*d);
        }
    }
    let rows: Vec<String> = st
        .ch(org)
        .query(
            "SELECT 'histogram' FROM otel.otel_metrics_histogram WHERE OrgId = {oc_org:String} AND MetricName = ? LIMIT 1 \
             UNION ALL SELECT 'exponential_histogram' FROM otel.otel_metrics_exponential_histogram WHERE OrgId = {oc_org:String} AND MetricName = ? LIMIT 1 \
             UNION ALL SELECT 'summary' FROM otel.otel_metrics_summary WHERE OrgId = {oc_org:String} AND MetricName = ? LIMIT 1",
        )
        .bind(name)
        .bind(name)
        .bind(name)
        .fetch_all()
        .await?;
    let d = rows.first().map(|k| match k.as_str() {
        "histogram" => Dist::Histogram,
        "exponential_histogram" => Dist::ExpHistogram,
        _ => Dist::Summary,
    });
    cache.lock().unwrap().insert(key, (d, Instant::now()));
    Ok(d)
}

/// Per-interval change of a cumulative column (or the sum of a delta column) for one series.
fn delta(col: &str, temporality: &str) -> String {
    format!("if({temporality} = 2, greatest(toFloat64(argMax({col}, TimeUnix)) - toFloat64(argMin({col}, TimeUnix)), 0), toFloat64(sum({col})))")
}

/// Per-second rate of a cumulative column (over the time its samples span, like counters) or of a
/// delta column (over the interval).
fn rate(col: &str, temporality: &str, step_s: i64) -> String {
    format!(
        "if({temporality} = 2, {d} * 1000 / greatest(dateDiff('millisecond', min(TimeUnix), max(TimeUnix)), 1), {d} / {step_s})",
        d = delta(col, temporality)
    )
}

/// Per-interval change of cumulative bucket counts (element-wise; a reset or layout change counts as nothing).
fn delta_buckets(col: &str, temporality: &str) -> String {
    format!(
        "if({temporality} = 2, if(length(argMax({col}, TimeUnix)) = length(argMin({col}, TimeUnix)), \
         arrayMap((a, b) -> if(a >= b, toUInt64(a - b), toUInt64(0)), argMax({col}, TimeUnix), argMin({col}, TimeUnix)), arrayMap(x -> toUInt64(0), argMax({col}, TimeUnix))), sumForEach({col}))"
    )
}

/// Linear interpolation of quantile `q` inside bucket counts `c` with bucket bounds given by
/// `lower(i)` / `upper(i)` (1-based bucket index `i`). NaN when there were no observations.
fn bucket_quantile(c: &str, q: f64, lower: &str, upper: &str) -> String {
    // `qi`: the first bucket whose cumulative count reaches the target rank (aliases are visible to the whole expression).
    format!(
        "if(arraySum({c}) = 0, nan, ({lower}) + (({upper}) - ({lower})) * ({q} * arraySum({c}) - if(qi <= 1, 0, arrayCumSum({c})[qi - 1])) \
           / greatest({c}[(arrayFirstIndex(x -> x >= {q} * arraySum({c}), arrayCumSum({c})) AS qi)], 1))",
        lower = lower.replace("{i}", "qi"),
        upper = upper.replace("{i}", "qi"),
    )
}

/// Inner query (g, t, sid, v) for a distribution metric statistic.
fn dist_query(
    dist: Dist,
    stat: &str,
    g: &str,
    step_s: i64,
    filter: &str,
) -> Result<String, ApiError> {
    let table = dist.table();
    // Summaries have no temporality column: they are always cumulative.
    let temporality = if dist == Dist::Summary {
        "2"
    } else {
        "any(AggregationTemporality)"
    };
    let series = "cityHash64(toString(mapSort(ResourceAttributes)), toString(mapSort(Attributes)))";
    let base = format!(
        "FROM {table} WHERE OrgId = {{oc_org:String}} AND MetricName = ? AND TimeUnix >= fromUnixTimestamp64Milli(?) AND TimeUnix <= fromUnixTimestamp64Milli(?) AND {filter}"
    );
    let head = format!("SELECT {g} AS g, toStartOfInterval(TimeUnix, INTERVAL {step_s} SECOND) AS t, {series} AS sid");
    let simple = |v: String| format!("{head}, {v} AS v {base} GROUP BY g, t, sid");
    let unsupported =
        || ApiError::BadRequest(format!("{} metrics don't record {stat}", dist.name()));
    Ok(match (stat, quantile_of(stat)) {
        // A cumulative series needs two samples in the interval to have a rate.
        ("count", _) => format!("{} HAVING count() > 1 OR {temporality} != 2", simple(rate("Count", temporality, step_s))),
        ("sum", _) => format!("{} HAVING count() > 1 OR {temporality} != 2", simple(rate("Sum", temporality, step_s))),
        ("avg", _) => simple(format!("if({c} = 0, nan, {s} / {c})", s = delta("Sum", temporality), c = delta("Count", temporality))),
        ("min" | "max", _) if dist == Dist::Summary => {
            let q = if stat == "min" { 0.0 } else { 1.0 };
            simple(format!("avgIf(`ValueAtQuantiles.Value`[indexOf(`ValueAtQuantiles.Quantile`, {q})], has(`ValueAtQuantiles.Quantile`, {q}))"))
        }
        ("min", _) => simple("min(Min)".into()),
        ("max", _) => simple("max(Max)".into()),
        (_, Some(q)) => match dist {
            Dist::Summary => simple(format!("avgIf(`ValueAtQuantiles.Value`[indexOf(`ValueAtQuantiles.Quantile`, {q})], has(`ValueAtQuantiles.Quantile`, {q}))")),
            // Explicit buckets share their layout across series: merge the group's counts, then interpolate.
            Dist::Histogram => {
                let v = bucket_quantile("cs", q, "if({i} <= 1, least(0, bs[1]), bs[{i} - 1])", "if({i} <= length(bs), bs[{i}], bs[length(bs)])");
                format!(
                    "SELECT g, t, toUInt64(0) AS sid, if(length(bs) = 0, nan, {v}) AS v FROM (\
                       SELECT g, t, sumForEach(c) AS cs, any(b) AS bs FROM (\
                         {head}, {c} AS c, any(ExplicitBounds) AS b {base} GROUP BY g, t, sid) GROUP BY g, t)",
                    c = delta_buckets("BucketCounts", temporality),
                )
            }
            // Bucket k (1-based) of an exponential histogram covers (base^(offset+k-1), base^(offset+k)].
            Dist::ExpHistogram => {
                let v = bucket_quantile("c", q, "pow(pow(2, pow(2, -sc)), off + {i} - 1)", "pow(pow(2, pow(2, -sc)), off + {i})");
                format!(
                    "SELECT g, t, sid, {v} AS v FROM (\
                       {head}, {c} AS c, any(Scale) AS sc, any(PositiveOffset) AS off {base} GROUP BY g, t, sid)",
                    c = delta_buckets("PositiveBucketCounts", temporality),
                )
            }
        },
        _ => return Err(unsupported()),
    })
}

fn step_ms(range: TimeRange, points: u32, min_ms: i64) -> i64 {
    let raw = (range.to - range.from) / points.max(1) as i64;
    const STEPS: [i64; 11] = [
        10_000, 20_000, 30_000, 60_000, 120_000, 300_000, 600_000, 1_800_000, 3_600_000,
        10_800_000, 21_600_000,
    ];
    STEPS
        .iter()
        .copied()
        .find(|s| *s >= raw.max(min_ms))
        .unwrap_or(43_200_000)
}

/// (org, metric name) → kind, cached briefly so every widget query doesn't re-probe the tables.
async fn metric_kind(st: &AppState, org: &str, name: &str) -> Result<Option<Kind>, ApiError> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Kind, Instant)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = format!("{org}\u{0}{name}");
    if let Some((k, at)) = cache.lock().unwrap().get(&key) {
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
        .ch(org)
        .query(
            "SELECT 'gauge' AS src, toInt32(0) AS temporality, false AS monotonic FROM otel.otel_metrics_gauge WHERE OrgId = {oc_org:String} AND MetricName = ? LIMIT 1 \
             UNION ALL SELECT 'sum' AS src, AggregationTemporality AS temporality, IsMonotonic AS monotonic FROM otel.otel_metrics_sum WHERE OrgId = {oc_org:String} AND MetricName = ? LIMIT 1",
        )
        .bind(name)
        .bind(name)
        .fetch_all::<Probe>()
        .await?;
    let Some(p) = rows.first() else {
        return Ok(None);
    };
    let kind = match (p.src.as_str(), p.temporality, p.monotonic) {
        ("gauge", ..) => Kind::Gauge,
        (_, 1, _) => Kind::Delta,
        (_, _, true) => Kind::Counter,
        _ => Kind::UpDown,
    };
    cache.lock().unwrap().insert(key, (kind, Instant::now()));
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
async fn inner_query(
    st: &AppState,
    org: &str,
    q: &MetricQuery,
    range: TimeRange,
    points: u32,
) -> Result<(String, Vec<Param>, i64), ApiError> {
    inner_query_at_step(st, org, q, range, points, None).await
}

async fn inner_query_at_step(
    st: &AppState,
    org: &str,
    q: &MetricQuery,
    range: TimeRange,
    points: u32,
    fixed_step: Option<i64>,
) -> Result<(String, Vec<Param>, i64), ApiError> {
    let mut params = Vec::new();
    if let Some((_, expr, _, _)) = SPAN_METRICS.iter().find(|m| m.0 == q.metric) {
        let step = fixed_step.unwrap_or_else(|| step_ms(range, points, 10_000));
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
             WHERE OrgId = {{oc_org:String}} AND {ENTRY} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) AND {filter} GROUP BY g, t, sid",
            g = group_array(&by),
            v = expr.replace("{step}", &step_s.to_string()),
            filter = f.clause,
        );
        return Ok((sql, params, step));
    }
    let (q, scale) = resolve_alias(q);
    let q = &q;
    let Some(kind) = metric_kind(st, org, &q.metric).await? else {
        // `<distribution>.<stat>`: histogram, exponential histogram or summary.
        let unknown = || ApiError::BadRequest(format!("Unknown metric: {}", q.metric));
        let (base, stat) = split_stat(&q.metric).ok_or_else(unknown)?;
        let dist = dist_kind(st, org, base).await?.ok_or_else(unknown)?;
        // Cumulative series need two samples per interval (SDKs export every 10–60 s).
        let step = fixed_step.unwrap_or_else(|| step_ms(range, points, 30_000));
        let mut by = Vec::new();
        for k in &q.by {
            let (e, p) = metric_field(k);
            by.push(e);
            params.extend(p);
        }
        let f = metric_filter(&q.filters, metric_field);
        params.extend([
            Param::Str(base.to_string()),
            Param::Int(range.from),
            Param::Int(range.to),
        ]);
        params.extend(f.params);
        let sql = dist_query(dist, stat, &group_array(&by), step / 1000, &f.clause)?;
        return Ok((sql, params, step));
    };
    // Counters need at least two samples per interval (collectors report every 10s).
    let step = fixed_step.unwrap_or_else(|| {
        step_ms(
            range,
            points,
            if kind == Kind::Counter {
                30_000
            } else {
                10_000
            },
        )
    });
    let step_s = step / 1000;
    let v = match kind {
        Kind::Gauge | Kind::UpDown => "avg(Value)".to_string(),
        Kind::Counter => "greatest(max(Value) - min(Value), 0) * 1000 / greatest(dateDiff('millisecond', min(TimeUnix), max(TimeUnix)), 1)".to_string(),
        Kind::Delta => format!("sum(Value) / {step_s}"),
    };
    let v = if scale == 1.0 {
        v
    } else {
        format!("({v}) * {scale:e}")
    };
    let mut by = Vec::new();
    for k in &q.by {
        let (e, p) = metric_field(k);
        by.push(e);
        params.extend(p);
    }
    let f = metric_filter(&q.filters, metric_field);
    params.extend([
        Param::Str(q.metric.clone()),
        Param::Int(range.from),
        Param::Int(range.to),
    ]);
    params.extend(f.params);
    let having = if kind == Kind::Counter {
        " HAVING count() > 1"
    } else {
        ""
    };
    let sql = format!(
        "SELECT {g} AS g, toStartOfInterval(TimeUnix, INTERVAL {step_s} SECOND) AS t, cityHash64(toString(mapSort(ResourceAttributes)), toString(mapSort(Attributes))) AS sid, {v} AS v \
         FROM {table} WHERE OrgId = {{oc_org:String}} AND MetricName = ? AND TimeUnix >= fromUnixTimestamp64Milli(?) AND TimeUnix <= fromUnixTimestamp64Milli(?) AND {filter} \
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
        format!(
            "[{}]",
            exprs
                .iter()
                .map(|e| format!("toString({e})"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn parse(query: &str) -> Result<MetricQuery, ApiError> {
    parse_metric_query(query)
        .map_err(|e| ApiError::BadRequest(format!("Invalid query '{query}': {e}")))
}

async fn evaluate(
    st: &AppState,
    org: &str,
    query: &str,
    range: TimeRange,
    points: u32,
) -> Result<(MetricQuery, Vec<Group>), ApiError> {
    range.validate()?;
    let q = parse(query)?;
    let groups = evaluate_query(st, org, &q, range, points).await?;
    Ok((q, groups))
}

/// Evaluates an already-built query (used by the host views, which never format user input into query text).
pub(crate) async fn evaluate_query(
    st: &AppState,
    org: &str,
    q: &MetricQuery,
    range: TimeRange,
    points: u32,
) -> Result<Vec<Group>, ApiError> {
    let (inner, params, _) = inner_query(st, org, q, range, points).await?;
    let sql = format!(
        "SELECT g, toInt64(toUnixTimestamp(t)) * 1000 AS ts, toFloat64({agg}(v)) AS value FROM ({inner}) WHERE isFinite(v) GROUP BY g, t ORDER BY g, t",
        agg = q.agg.sql()
    );
    let rows = bind(st.ch(org).query(&sql), &params)
        .fetch_all::<Row>()
        .await?;
    let mut groups: Vec<Group> = Vec::new();
    for r in rows {
        let tags: BTreeMap<String, String> =
            q.by.iter()
                .cloned()
                .zip(
                    r.g.into_iter()
                        .map(|v| if v.is_empty() { "N/A".into() } else { v }),
                )
                .collect();
        match groups.last_mut() {
            Some(g) if g.tags == tags => g.points.push((r.ts, round(r.value))),
            _ => groups.push(Group {
                tags,
                points: vec![(r.ts, round(r.value))],
            }),
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
        tags.iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
            .join(", ")
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

#[derive(Clone, Serialize, ToSchema)]
pub struct MetricSeries {
    pub name: String,
    /// `[timestamp ms, value]` pairs.
    pub points: Vec<(i64, f64)>,
}

/// Timeseries for one or more queries; one series per `by` group.
#[utoipa::path(post, path = "/api/v1/query/timeseries", request_body = TimeseriesRequest, responses((status = 200, body = Vec<MetricSeries>)), tag = "metrics")]
pub async fn timeseries(
    State(st): State<AppState>,
    OrgId(org): OrgId,
    Json(req): Json<TimeseriesRequest>,
) -> ApiResult<Vec<MetricSeries>> {
    if req.queries.len() > 10 {
        return Err(ApiError::BadRequest(
            "at most 10 queries per request".into(),
        ));
    }
    let points = req.points.unwrap_or(120).clamp(10, 1000);
    let results = futures::future::try_join_all(
        req.queries
            .iter()
            .map(|q| evaluate(&st, &org, q, req.range, points)),
    )
    .await?;
    let mut out = Vec::new();
    for ((q, groups), text) in results.into_iter().zip(&req.queries) {
        for g in groups {
            // Ungrouped series are named by their query so several queries stay distinct.
            out.push(MetricSeries {
                name: if q.by.is_empty() {
                    text.clone()
                } else {
                    group_label(&g.tags)
                },
                points: g.points,
            });
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
pub async fn scalar(
    State(st): State<AppState>,
    OrgId(org): OrgId,
    Json(req): Json<ScalarRequest>,
) -> ApiResult<f64> {
    let (q, groups) = evaluate(&st, &org, &req.query, req.range, 60).await?;
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
pub async fn grouped(
    State(st): State<AppState>,
    OrgId(org): OrgId,
    Json(req): Json<ScalarRequest>,
) -> ApiResult<Vec<GroupedScalar>> {
    let (_, groups) = evaluate(&st, &org, &req.query, req.range, 60).await?;
    let mut out: Vec<GroupedScalar> = groups
        .into_iter()
        .map(|g| GroupedScalar {
            group: group_label(&g.tags),
            value: round(reduce(&g.points, req.aggregator)),
            tags: g.tags,
        })
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
pub async fn distribution(
    State(st): State<AppState>,
    OrgId(org): OrgId,
    Json(req): Json<DistributionRequest>,
) -> ApiResult<DistributionResponse> {
    req.range.validate()?;
    let q = parse(&req.query)?;
    let (inner, params, _) = inner_query(&st, &org, &q, req.range, 40).await?;
    let sql = format!("SELECT sid, toInt64(toUnixTimestamp(t)) * 1000 AS ts, toFloat64(v) AS value FROM ({inner}) WHERE isFinite(v) ORDER BY ts");
    let rows = bind(st.ch(&org).query(&sql), &params)
        .fetch_all::<SeriesRow>()
        .await?;
    const N: usize = 10;
    let max = rows
        .iter()
        .map(|r| r.value)
        .fold(0.0_f64, f64::max)
        .max(f64::MIN_POSITIVE);
    let times: Vec<i64> = rows
        .iter()
        .map(|r| r.ts)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let index: HashMap<i64, usize> = times.iter().enumerate().map(|(i, t)| (*t, i)).collect();
    let mut counts = vec![vec![0u32; N]; times.len()];
    for r in &rows {
        let b = ((r.value / max) * N as f64)
            .floor()
            .clamp(0.0, (N - 1) as f64) as usize;
        counts[index[&r.ts]][b] += 1;
        let _ = r.sid;
    }
    let buckets = (1..=N).map(|i| round(max * i as f64 / N as f64)).collect();
    Ok(Json(DistributionResponse {
        buckets,
        times,
        counts,
    }))
}

#[derive(Serialize, ToSchema)]
pub struct MetricCatalogEntry {
    pub name: String,
    pub tags: Vec<String>,
    pub unit: Option<String>,
    pub description: String,
    /// gauge | sum | histogram | exponential_histogram | summary | span (computed from traces)
    pub kind: String,
}

/// Metrics reported in the last day plus the span-derived `trace.*` metrics.
#[utoipa::path(get, path = "/api/v1/metrics", responses((status = 200, body = Vec<MetricCatalogEntry>)), tag = "metrics")]
pub async fn catalog(
    State(st): State<AppState>,
    OrgId(org): OrgId,
) -> ApiResult<Vec<MetricCatalogEntry>> {
    #[derive(clickhouse::Row, Deserialize)]
    struct Entry {
        name: String,
        kind: String,
        unit: String,
        description: String,
        attr_keys: Vec<String>,
        res_keys: Vec<String>,
    }
    let part = |table: &str, kind: &str| {
        format!(
            "SELECT MetricName AS name, '{kind}' AS kind, any(MetricUnit) AS unit, any(MetricDescription) AS description, \
             arrayDistinct(arrayFlatten(groupUniqArray(mapKeys(Attributes)))) AS attr_keys, \
             arrayDistinct(arrayFlatten(groupUniqArray(mapKeys(ResourceAttributes)))) AS res_keys \
             FROM {table} WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 1 DAY GROUP BY MetricName"
        )
    };
    let sql = [
        part("otel.otel_metrics_gauge", "gauge"),
        part("otel.otel_metrics_sum", "sum"),
        part(Dist::Histogram.table(), Dist::Histogram.name()),
        part(Dist::ExpHistogram.table(), Dist::ExpHistogram.name()),
        part(Dist::Summary.table(), Dist::Summary.name()),
    ]
    .join(" UNION ALL ");
    let rows = st.ch(&org).query(&sql).fetch_all::<Entry>().await?;
    let mut out: Vec<MetricCatalogEntry> = Vec::new();
    for e in rows {
        let mut tags: BTreeSet<String> = e.attr_keys.into_iter().collect();
        // `oc.org_id` is the tenant stamp, not something to filter or group on.
        for k in e.res_keys.into_iter().filter(|k| k != "oc.org_id") {
            tags.insert(short_tag(&k).map(str::to_string).unwrap_or(k));
        }
        let tags: Vec<String> = tags.into_iter().collect();
        let unit = Some(e.unit).filter(|u| !u.is_empty() && u != "1");
        if e.kind == "gauge" || e.kind == "sum" {
            out.push(MetricCatalogEntry {
                name: e.name,
                tags,
                unit,
                description: e.description,
                kind: e.kind,
            });
            continue;
        }
        // Distributions are queried per statistic.
        for (stat, what) in DIST_STATS {
            let unit = match *stat {
                "count" => Some("/s".to_string()),
                "sum" => unit.as_ref().map(|u| format!("{u}/s")),
                _ => unit.clone(),
            };
            let description = if e.description.is_empty() {
                what.to_string()
            } else {
                format!("{} — {}", e.description, what.to_lowercase())
            };
            out.push(MetricCatalogEntry {
                name: format!("{}.{stat}", e.name),
                tags: tags.clone(),
                unit,
                description,
                kind: e.kind.clone(),
            });
        }
    }
    let aliases: Vec<MetricCatalogEntry> = ALIASES
        .iter()
        .filter_map(|(alias, base, filter, _, unit, desc)| {
            let b = out.iter().find(|e| e.name == *base)?;
            let tags = b
                .tags
                .iter()
                .filter(|t| filter.map_or(true, |(k, _)| *t != k))
                .cloned()
                .collect();
            Some(MetricCatalogEntry {
                name: alias.to_string(),
                tags,
                unit: Some(unit.to_string()).filter(|u| !u.is_empty()),
                description: desc.to_string(),
                kind: b.kind.clone(),
            })
        })
        .collect();
    out.extend(aliases);
    out.extend(
        SPAN_METRICS
            .iter()
            .map(|(name, _, unit, desc)| MetricCatalogEntry {
                name: name.to_string(),
                tags: SPAN_TAGS.iter().map(|s| s.to_string()).collect(),
                unit: Some(unit.to_string()),
                description: desc.to_string(),
                kind: "span".into(),
            }),
    );
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(out))
}

/// Values seen for a tag in the last day, across metrics and spans (template variable pickers).
#[utoipa::path(get, path = "/api/v1/tags/{key}/values", params(("key" = String, Path)), responses((status = 200, body = Vec<String>)), tag = "metrics")]
pub async fn tag_values(
    State(st): State<AppState>,
    OrgId(org): OrgId,
    Path(key): Path<String>,
) -> ApiResult<Vec<String>> {
    if key.is_empty()
        || key.len() > 200
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/'))
    {
        return Err(ApiError::BadRequest("invalid tag key".into()));
    }
    let (m_expr, m_params) = metric_field(&key);
    let mut params = m_params.clone();
    params.extend(m_params.clone());
    params.extend(m_params);
    let mut sql = format!(
        "SELECT DISTINCT v FROM (\
           SELECT toString({m_expr}) AS v FROM otel.otel_metrics_gauge WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 1 DAY \
           UNION ALL SELECT toString({m_expr}) AS v FROM otel.otel_metrics_sum WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 1 DAY \
           UNION ALL SELECT toString({m_expr}) AS v FROM otel.otel_metrics_histogram WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 1 DAY"
    );
    if SPAN_TAGS.contains(&key.as_str()) {
        let (s_expr, s_params) = span_field(&key);
        sql.push_str(&format!(" UNION ALL SELECT toString({s_expr}) AS v FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND Timestamp > now() - INTERVAL 1 HOUR"));
        params.extend(s_params);
    }
    sql.push_str(") WHERE v != '' ORDER BY v LIMIT 500");
    let values = bind(st.ch(&org).query(&sql), &params)
        .fetch_all::<String>()
        .await?;
    Ok(Json(values))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_suffixes() {
        assert_eq!(
            split_stat("http.server.request.duration.p95"),
            Some(("http.server.request.duration", "p95"))
        );
        assert_eq!(
            split_stat("runtime.gc.pause.avg"),
            Some(("runtime.gc.pause", "avg"))
        );
        assert_eq!(split_stat("system.cpu.utilization"), None);
        assert_eq!(quantile_of("p99"), Some(0.99));
        assert_eq!(quantile_of("avg"), None);
    }

    #[test]
    fn stats_each_type_supports() {
        for d in [Dist::Histogram, Dist::ExpHistogram, Dist::Summary] {
            for (stat, _) in DIST_STATS {
                let sql = dist_query(d, stat, "[]", 30, "1").unwrap();
                assert!(
                    sql.contains("OrgId = {oc_org:String}"),
                    "{d:?} {stat} must be org-scoped"
                );
            }
        }
    }
}
