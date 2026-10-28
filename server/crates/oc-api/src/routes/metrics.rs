//! Metric queries for dashboards (`avg:system.cpu.utilization{env:prod} by {host}`).
//!
//! Sources:
//! - The metrics storage (docs/adr/0001-metrics-storage.md): series + samples, or the 5-minute /
//!   1-hour rollups for long ranges (see [`resolution`]); distributions have their own samples and
//!   rollups (`otel.metrics_dist_*`).
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

pub mod dashboard;

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

/// [`resolve_alias`] only for a metric that isn't there itself: the Datadog Agent sends
/// `system.mem.used`, `system.cpu.user`, … under those names, with no OpenTelemetry base metric.
pub(super) async fn resolve_alias_if_missing(st: &AppState, org: &str, q: &MetricQuery) -> Result<(MetricQuery, f64), ApiError> {
    if ALIASES.iter().any(|a| a.0 == q.metric) && metric_kind(st, org, &q.metric).await?.is_some() {
        return Ok((q.clone(), 1.0));
    }
    Ok(resolve_alias(q))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Gauge,
    /// Non-monotonic sum (e.g. memory usage): behaves like a gauge.
    UpDown,
    Counter,
    Delta,
}

/// Distribution metric families (queried with a statistic suffix).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dist {
    Histogram,
    ExpHistogram,
    Summary,
}

impl Dist {
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
            "SELECT toString(MetricType) FROM otel.metrics_series \
             WHERE OrgId = {oc_org:String} AND MetricName = ? AND MetricType IN ('histogram', 'exponential_histogram', 'summary') ORDER BY MetricType LIMIT 1",
        )
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

/// Inner query (g, t, sid, v) for a distribution metric statistic over raw points selected `from`
/// a table or subquery with the exporter's columns.
fn dist_query(
    from: &str,
    dist: Dist,
    stat: &str,
    g: &str,
    step_s: i64,
    filter: &str,
) -> Result<String, ApiError> {
    let table = from;
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

/// A distribution statistic's per-series, per-interval values (g, t, sid, v) from the storage:
/// raw samples or a rollup. Returns the SQL and its params.
fn dist_points_query(
    dist: Dist,
    stat: &str,
    source: Source,
    q: &MetricQuery,
    range: TimeRange,
    step_s: i64,
) -> Result<(String, Vec<Param>), ApiError> {
    let base = split_stat(&q.metric)
        .map_or(q.metric.as_str(), |(b, _)| b)
        .to_string();
    let mut params = Vec::new();
    let mut by = Vec::new();
    for k in &q.by {
        let (e, p) = metric_field(k);
        by.push(e);
        params.extend(p);
    }
    let f = metric_filter(&q.filters, metric_field);
    let g = group_array(&by);
    let kind = dist.name();
    if source == Source::Samples {
        // The raw query over samples joined to their series, which carry the labels and temporality.
        let from = format!(
            "(SELECT x.OrgId AS OrgId, x.MetricName AS MetricName, s.ResourceAttributes AS ResourceAttributes, s.Attributes AS Attributes, s.AggregationTemporality AS AggregationTemporality, \
                     x.TimeUnix AS TimeUnix, x.Count AS Count, x.Sum AS Sum, x.Min AS Min, x.Max AS Max, x.BucketCounts AS BucketCounts, x.ExplicitBounds AS ExplicitBounds, \
                     x.Scale AS Scale, x.PositiveOffset AS PositiveOffset, x.PositiveBucketCounts AS PositiveBucketCounts, \
                     x.`ValueAtQuantiles.Quantile` AS `ValueAtQuantiles.Quantile`, x.`ValueAtQuantiles.Value` AS `ValueAtQuantiles.Value` FROM otel.metrics_dist_samples AS x \
              INNER JOIN (SELECT SeriesId, any(ResourceAttributes) AS ResourceAttributes, any(Attributes) AS Attributes, any(AggregationTemporality) AS AggregationTemporality \
                          FROM otel.metrics_series WHERE OrgId = {{oc_org:String}} AND MetricName = ? AND MetricType = '{kind}' AND LastSeen >= toDateTime(intDiv(?, 1000)) GROUP BY SeriesId) AS s \
                ON x.SeriesId = s.SeriesId \
              WHERE x.OrgId = {{oc_org:String}} AND x.MetricName = ? AND x.TimeUnix >= fromUnixTimestamp64Milli(?) AND x.TimeUnix <= fromUnixTimestamp64Milli(?))"
        );
        params.extend([
            Param::Str(base.clone()),
            Param::Int(range.from),
            Param::Str(base.clone()),
            Param::Int(range.from),
            Param::Int(range.to),
        ]);
        params.extend([
            Param::Str(base),
            Param::Int(range.from),
            Param::Int(range.to),
        ]);
        params.extend(f.params);
        return Ok((
            dist_query(&from, dist, stat, &g, step_s, &f.clause)?,
            params,
        ));
    }
    // Rollups: the same statistics from each bucket's first / last / total values.
    let table = if source == Source::Rollup5m {
        "otel.metrics_dist_rollup_5m"
    } else {
        "otel.metrics_dist_rollup_1h"
    };
    let temporality = "any(s.temporality)";
    // Change over the interval: last − first for cumulative series, the total for delta ones.
    let delta = |first: &str, last: &str, total: &str| {
        format!("if({temporality} = 2, greatest(toFloat64(argMaxMerge({last})) - toFloat64(argMinMerge({first})), 0), toFloat64(sum({total})))")
    };
    let rate = |d: String| {
        format!("if({temporality} = 2, {d} * 1000 / greatest(dateDiff('millisecond', min(Start), max(End)), 1), {d} / {step_s})")
    };
    let buckets = format!(
        "if({temporality} = 2, if(length(argMaxMerge(BucketsLast)) = length(argMinMerge(BucketsFirst)), \
         arrayMap((a, b) -> if(a >= b, toUInt64(a - b), toUInt64(0)), argMaxMerge(BucketsLast), argMinMerge(BucketsFirst)), arrayMap(x -> toUInt64(0), argMaxMerge(BucketsLast))), \
         sumForEachMerge(BucketsTotal))"
    );
    let quantile = |q: f64| {
        format!("if(has(any(Quantiles), {q}), sumForEachMerge(QuantileTotals)[indexOf(any(Quantiles), {q})] / sum(Points), nan)")
    };
    let head = format!("SELECT s.g AS g, toStartOfInterval(x.Bucket, INTERVAL {step_s} SECOND) AS t, x.SeriesId AS sid");
    let from = format!(
        "FROM {table} AS x \
         INNER JOIN (SELECT SeriesId, any({g}) AS g, any(AggregationTemporality) AS temporality FROM otel.metrics_series \
                     WHERE OrgId = {{oc_org:String}} AND MetricName = ? AND MetricType = '{kind}' AND LastSeen >= toDateTime(intDiv(?, 1000)) AND {filter} GROUP BY SeriesId) AS s \
           ON x.SeriesId = s.SeriesId \
         WHERE x.OrgId = {{oc_org:String}} AND x.MetricName = ? \
           AND x.Bucket >= toStartOfInterval(fromUnixTimestamp64Milli(?), INTERVAL {bucket_s} SECOND) AND x.Bucket <= fromUnixTimestamp64Milli(?)",
        filter = f.clause,
        bucket_s = source.bucket_ms() / 1000,
    );
    params.extend([Param::Str(base.clone()), Param::Int(range.from)]);
    params.extend(f.params);
    params.extend([
        Param::Str(base),
        Param::Int(range.from),
        Param::Int(range.to),
    ]);
    let simple = |v: String| format!("{head}, {v} AS v {from} GROUP BY g, t, sid");
    let unsupported =
        || ApiError::BadRequest(format!("{} metrics don't record {stat}", dist.name()));
    let sql = match (stat, quantile_of(stat)) {
        // A cumulative series needs two samples in the interval to have a rate.
        ("count", _) => format!(
            "{} HAVING sum(Points) > 1 OR {temporality} != 2",
            simple(rate(delta("CountFirst", "CountLast", "CountTotal")))
        ),
        ("sum", _) => format!(
            "{} HAVING sum(Points) > 1 OR {temporality} != 2",
            simple(rate(delta("SumFirst", "SumLast", "SumTotal")))
        ),
        ("avg", _) => simple(format!(
            "if({c} = 0, nan, {s} / {c})",
            s = delta("SumFirst", "SumLast", "SumTotal"),
            c = delta("CountFirst", "CountLast", "CountTotal")
        )),
        ("min" | "max", _) if dist == Dist::Summary => {
            simple(quantile(if stat == "min" { 0.0 } else { 1.0 }))
        }
        ("min", _) => simple("min(Min)".into()),
        ("max", _) => simple("max(Max)".into()),
        (_, Some(q)) => match dist {
            Dist::Summary => simple(quantile(q)),
            // Explicit buckets share their layout across series: merge the group's counts, then interpolate.
            Dist::Histogram => {
                let v = bucket_quantile(
                    "cs",
                    q,
                    "if({i} <= 1, least(0, bs[1]), bs[{i} - 1])",
                    "if({i} <= length(bs), bs[{i}], bs[length(bs)])",
                );
                format!(
                    "SELECT g, t, toUInt64(0) AS sid, if(length(bs) = 0, nan, {v}) AS v FROM (\
                       SELECT g, t, sumForEach(c) AS cs, any(b) AS bs FROM (\
                         {head}, {buckets} AS c, any(ExplicitBounds) AS b {from} GROUP BY g, t, sid) GROUP BY g, t)"
                )
            }
            Dist::ExpHistogram => {
                let v = bucket_quantile(
                    "c",
                    q,
                    "pow(pow(2, pow(2, -sc)), off + {i} - 1)",
                    "pow(pow(2, pow(2, -sc)), off + {i})",
                );
                format!("SELECT g, t, sid, {v} AS v FROM ({head}, {buckets} AS c, any(Scale) AS sc, any(PositiveOffset) AS off {from} GROUP BY g, t, sid)")
            }
        },
        _ => return Err(unsupported()),
    };
    Ok((sql, params))
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

/// Where gauge and sum points are read from (docs/adr/0001-metrics-storage.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Samples,
    Rollup5m,
    Rollup1h,
}

impl Source {
    /// Bucket size in milliseconds (0: raw samples).
    fn bucket_ms(self) -> i64 {
        match self {
            Source::Samples => 0,
            Source::Rollup5m => 300_000,
            Source::Rollup1h => 3_600_000,
        }
    }
}

/// How many days raw samples and each rollup are kept.
#[derive(Clone, Copy, Debug)]
struct Tiers {
    raw_days: u32,
    rollup_5m_days: u32,
    rollup_1h_days: u32,
}

/// The coarsest source whose buckets fit in `step` and that still holds the range's start, and the
/// step to use. When only a coarser one still has the data, the step grows to its bucket.
fn resolution(range: TimeRange, step: i64, now: i64, tiers: Tiers) -> (Source, i64) {
    let kept = |days: u32| range.from >= now - days as i64 * 86_400_000;
    let sources = [
        (Source::Rollup1h, tiers.rollup_1h_days),
        (Source::Rollup5m, tiers.rollup_5m_days),
        (Source::Samples, tiers.raw_days),
    ];
    if let Some((s, _)) = sources
        .iter()
        .find(|(s, days)| s.bucket_ms() <= step && kept(*days))
    {
        return (*s, step);
    }
    // Finest of the coarser sources that still has it, or the longest-kept one.
    let s = sources
        .iter()
        .rev()
        .find(|(_, days)| kept(*days))
        .map_or(Source::Rollup1h, |(s, _)| *s);
    (s, step.max(s.bucket_ms()))
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
            "SELECT toString(MetricType) AS src, AggregationTemporality AS temporality, IsMonotonic AS monotonic FROM otel.metrics_series \
             WHERE OrgId = {oc_org:String} AND MetricName = ? AND MetricType IN ('gauge', 'sum') ORDER BY MetricType LIMIT 1",
        )
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
    forced: Option<(Source, i64)>,
) -> Result<(String, Vec<Param>, i64), ApiError> {
    let mut params = Vec::new();
    if let Some((_, expr, _, _)) = SPAN_METRICS.iter().find(|m| m.0 == q.metric) {
        let step = forced
            .map(|(_, step)| step)
            .unwrap_or_else(|| step_ms(range, points, 10_000));
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
    let (q, scale) = resolve_alias_if_missing(st, org, q).await?;
    let q = &q;
    let Some(kind) = metric_kind(st, org, &q.metric).await? else {
        // `<distribution>.<stat>`: histogram, exponential histogram or summary.
        let unknown = || ApiError::BadRequest(format!("Unknown metric: {}", q.metric));
        let (base, stat) = split_stat(&q.metric).ok_or_else(unknown)?;
        let dist = dist_kind(st, org, base).await?.ok_or_else(unknown)?;
        // Cumulative series need two samples per interval (SDKs export every 10–60 s).
        let step = step_ms(range, points, 30_000);
        let (source, step) = match forced {
            Some(value) => value,
            None => resolution(range, step, now_ms(), org_tiers(st, org).await?),
        };
        let (sql, p) = dist_points_query(dist, stat, source, q, range, step / 1000)?;
        params.extend(p);
        return Ok((sql, params, step));
    };
    // Counters need at least two samples per interval (collectors report every 10s).
    let step = step_ms(
        range,
        points,
        if kind == Kind::Counter {
            30_000
        } else {
            10_000
        },
    );
    let (source, step) = match forced {
        Some(value) => value,
        None => resolution(range, step, now_ms(), org_tiers(st, org).await?),
    };
    let (sql, p) = points_query(kind, source, q, range, step / 1000, scale);
    params.extend(p);
    Ok((sql, params, step))
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Upper bound of a window that ends "now" (samples a little ahead of this server's clock count).
pub(crate) const OPEN_END_MS: i64 = 3_600_000;

/// Sets the `{oc_from:Int64}` / `{oc_to:Int64}` window (epoch ms) that [`points_subquery`] reads.
pub(crate) fn with_window(c: clickhouse::Client, from: i64, to: i64) -> clickhouse::Client {
    c.with_option("param_oc_from", from.to_string())
        .with_option("param_oc_to", to.to_string())
}

/// Points of the `kind` ("gauge" or "sum") metrics `names` in the window, with the columns of the
/// exporter's tables that direct readers use (OrgId, MetricName, ServiceName, ResourceAttributes,
/// Attributes, TimeUnix, Value), from series + samples. `names` are the caller's constants.
pub(crate) fn points_subquery(kind: &str, names: &[&str]) -> String {
    assert!(matches!(kind, "gauge" | "sum"));
    let names = name_list(names);
    format!(
        "(SELECT x.OrgId AS OrgId, x.MetricName AS MetricName, s.ServiceName AS ServiceName, s.ResourceAttributes AS ResourceAttributes, s.Attributes AS Attributes, \
                 x.TimeUnix AS TimeUnix, x.Value AS Value FROM otel.metrics_samples AS x \
          INNER JOIN (SELECT SeriesId, any(ServiceName) AS ServiceName, any(ResourceAttributes) AS ResourceAttributes, any(Attributes) AS Attributes FROM otel.metrics_series \
                      WHERE OrgId = {{oc_org:String}} AND MetricName IN ({names}) AND MetricType = '{kind}' AND LastSeen >= toDateTime(intDiv({{oc_from:Int64}}, 1000)) \
                      GROUP BY SeriesId) AS s ON x.SeriesId = s.SeriesId \
          WHERE x.OrgId = {{oc_org:String}} AND x.MetricName IN ({names}) \
            AND x.TimeUnix >= fromUnixTimestamp64Milli({{oc_from:Int64}}) AND x.TimeUnix <= fromUnixTimestamp64Milli({{oc_to:Int64}}))"
    )
}

/// `'a', 'b'` for an SQL `IN (...)` of metric name constants (never user input).
pub(crate) fn name_list(names: &[&str]) -> String {
    names
        .iter()
        .map(|n| {
            assert!(
                n.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_')),
                "metric name constant: {n}"
            );
            format!("'{n}'")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Per-series, per-interval values (g, t, sid, v) of a gauge or sum metric from series + samples
/// or a rollup. Returns the SQL and its params.
fn points_query(
    kind: Kind,
    source: Source,
    q: &MetricQuery,
    range: TimeRange,
    step_s: i64,
    scale: f64,
) -> (String, Vec<Param>) {
    // Raw points (Value, TimeUnix) or rollup buckets (Sum, Count, Min, Max, Start, End).
    let v = match (kind, source != Source::Samples) {
        (Kind::Gauge | Kind::UpDown, false) => "avg(Value)".to_string(),
        (Kind::Gauge | Kind::UpDown, true) => "sum(Sum) / sum(Count)".to_string(),
        (Kind::Counter, false) => "greatest(max(Value) - min(Value), 0) * 1000 / greatest(dateDiff('millisecond', min(TimeUnix), max(TimeUnix)), 1)".to_string(),
        (Kind::Counter, true) => "greatest(max(Max) - min(Min), 0) * 1000 / greatest(dateDiff('millisecond', min(Start), max(End)), 1)".to_string(),
        (Kind::Delta, false) => format!("sum(Value) / {step_s}"),
        (Kind::Delta, true) => format!("sum(Sum) / {step_s}"),
    };
    let v = if scale == 1.0 {
        v
    } else {
        format!("({v}) * {scale:e}")
    };
    let having = match (kind, source != Source::Samples) {
        (Kind::Counter, false) => " HAVING count() > 1",
        (Kind::Counter, true) => " HAVING sum(Count) > 1",
        _ => "",
    };
    let mut params = Vec::new();
    let mut by = Vec::new();
    for k in &q.by {
        let (e, p) = metric_field(k);
        by.push(e);
        params.extend(p);
    }
    let f = metric_filter(&q.filters, metric_field);
    let g = group_array(&by);
    // Labels are filtered and grouped on the series table (one row per series and insert), then
    // joined to the points by SeriesId. A rollup bucket counts in the interval it starts in.
    let (table, time) = match source {
        Source::Samples => ("otel.metrics_samples", "TimeUnix"),
        Source::Rollup5m => ("otel.metrics_rollup_5m", "Bucket"),
        Source::Rollup1h => ("otel.metrics_rollup_1h", "Bucket"),
    };
    let bucket_s = (source.bucket_ms() / 1000).max(1);
    params.extend([Param::Str(q.metric.clone()), Param::Int(range.from)]);
    params.extend(f.params);
    params.extend([
        Param::Str(q.metric.clone()),
        Param::Int(range.from),
        Param::Int(range.to),
    ]);
    let sql = format!(
        "SELECT s.g AS g, toStartOfInterval(x.{time}, INTERVAL {step_s} SECOND) AS t, x.SeriesId AS sid, {v} AS v FROM {table} AS x \
         INNER JOIN (SELECT SeriesId, any({g}) AS g FROM otel.metrics_series \
                     WHERE OrgId = {{oc_org:String}} AND MetricName = ? AND LastSeen >= toDateTime(intDiv(?, 1000)) AND {filter} GROUP BY SeriesId) AS s ON x.SeriesId = s.SeriesId \
         WHERE x.OrgId = {{oc_org:String}} AND x.MetricName = ? \
           AND x.{time} >= toStartOfInterval(fromUnixTimestamp64Milli(?), INTERVAL {bucket_s} SECOND) AND x.{time} <= fromUnixTimestamp64Milli(?) \
         GROUP BY g, t, sid{having}",
        filter = f.clause,
    );
    (sql, params)
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

/// Longest range a metric query may span: what the org's longest-kept rollup holds.
async fn validate_metric_range(st: &AppState, org: &str, range: TimeRange) -> Result<(), ApiError> {
    let t = org_tiers(st, org).await?;
    range.validate_within(t.raw_days.max(t.rollup_5m_days).max(t.rollup_1h_days))
}

/// The org's metric retention (its choice or the install's default), cached for a minute.
async fn org_tiers(st: &AppState, org: &str) -> Result<Tiers, ApiError> {
    type Cache = Mutex<HashMap<String, (Tiers, Instant)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((t, at)) = cache.lock().unwrap().get(org) {
        if at.elapsed() < Duration::from_secs(60) {
            return Ok(*t);
        }
    }
    let s = super::log_indexes::data_settings(st, org).await?;
    let (m, _) = s
        .metrics
        .effective(super::log_indexes::install_metric_retention(
            &st.config.retention,
        ));
    let t = Tiers {
        raw_days: m.raw.max(1) as u32,
        rollup_5m_days: m.rollup_5m.max(1) as u32,
        rollup_1h_days: m.rollup_1h.max(1) as u32,
    };
    cache
        .lock()
        .unwrap()
        .insert(org.to_string(), (t, Instant::now()));
    Ok(t)
}

/// A metric query, or a formula between metric queries (`sum:a{*} by {x} / sum:b{*} by {x} * 100`):
/// the plain query (none for a formula) and the series.
async fn evaluate_formula(st: &AppState, org: &str, text: &str, range: TimeRange, points: u32) -> Result<(Option<MetricQuery>, Vec<Group>), ApiError> {
    let f = super::monitor_formula::parse_formula(text).map_err(|e| ApiError::BadRequest(format!("invalid query: {e}")))?;
    if f.expr == super::monitor_formula::Expr::Query(0) {
        let (q, groups) = evaluate(st, org, text, range, points).await?;
        return Ok((Some(q), groups));
    }
    let results = futures::future::try_join_all(f.queries.iter().map(|q| evaluate(st, org, q, range, points))).await?;
    let groups: Vec<Vec<Group>> = results.into_iter().map(|(_, g)| g).collect();
    Ok((None, super::monitor_formula::combine(&f.expr, &groups)))
}

async fn evaluate(
    st: &AppState,
    org: &str,
    query: &str,
    range: TimeRange,
    points: u32,
) -> Result<(MetricQuery, Vec<Group>), ApiError> {
    validate_metric_range(st, org, range).await?;
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

#[derive(Serialize, ToSchema)]
pub struct MetricSeries {
    pub name: String,
    /// `[timestamp ms, value]` pairs.
    pub points: Vec<(i64, f64)>,
    /// The metric's unit (`byte`, `nanocore`, `byte/second`…): its sender's, else its integration's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub unit: Option<String>,
}

#[derive(clickhouse::Row, Deserialize)]
struct UnitRow {
    name: String,
    unit: String,
}

/// Units of the queried metrics (a distribution statistic takes its base metric's).
async fn series_units(st: &AppState, org: &str, metrics: &[String]) -> Result<HashMap<String, String>, ApiError> {
    let bases: Vec<String> = metrics.iter().map(|n| split_stat(n).map(|(b, _)| b.to_string()).unwrap_or(n.clone())).collect();
    let marks = vec!["?"; bases.len()].join(",");
    let sql = format!("SELECT MetricName AS name, any(MetricUnit) AS unit FROM otel.metrics_series WHERE OrgId = {{oc_org:String}} AND MetricName IN ({marks}) GROUP BY MetricName");
    let params: Vec<Param> = bases.iter().cloned().map(Param::Str).collect();
    let reported: HashMap<String, String> = bind(st.ch(org).query(&sql), &params).fetch_all::<UnitRow>().await?.into_iter().map(|r| (r.name, r.unit)).collect();
    Ok(metrics
        .iter()
        .zip(&bases)
        .filter_map(|(m, b)| dashboard::metric_unit(b, reported.get(b).map(String::as_str).unwrap_or("")).map(|u| (m.clone(), u)))
        .collect())
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
            .map(|q| evaluate_formula(&st, &org, q, req.range, points)),
    )
    .await?;
    let metrics: Vec<String> = results.iter().filter_map(|(q, _)| q.as_ref().map(|q| q.metric.clone())).collect::<std::collections::HashSet<_>>().into_iter().collect();
    let units = series_units(&st, &org, &metrics).await?;
    let mut out = Vec::new();
    for ((q, groups), text) in results.into_iter().zip(&req.queries) {
        for g in groups {
            // Ungrouped series are named by their query so several queries stay distinct.
            out.push(MetricSeries {
                name: if g.tags.is_empty() {
                    text.clone()
                } else {
                    group_label(&g.tags)
                },
                points: g.points,
                // A formula's unit isn't its queries' (a ratio, a difference of rates…): none.
                unit: q.as_ref().and_then(|q| units.get(&q.metric).cloned()),
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
    validate_metric_range(&st, &org, req.range).await?;
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

/// Metrics reported in the last day: (name, kind, unit, description, attr_keys, res_keys) per
/// metric and kind, from the series table.
fn catalog_sql() -> String {
    "SELECT MetricName AS name, toString(MetricType) AS kind, any(MetricUnit) AS unit, any(MetricDescription) AS description, \
     arrayDistinct(arrayFlatten(groupUniqArray(mapKeys(Attributes)))) AS attr_keys, \
     arrayDistinct(arrayFlatten(groupUniqArray(mapKeys(ResourceAttributes)))) AS res_keys \
     FROM otel.metrics_series WHERE OrgId = {oc_org:String} AND LastSeen > now() - INTERVAL 1 DAY GROUP BY MetricName, MetricType"
        .to_string()
}

/// Values of the tag `expr` (with its `params`) on metrics in the last day, from the series table.
fn tag_values_sql(expr: &str, params: &[Param]) -> (String, Vec<Param>) {
    let sql = format!(
        "SELECT DISTINCT toString({expr}) AS v FROM otel.metrics_series WHERE OrgId = {{oc_org:String}} AND LastSeen > now() - INTERVAL 1 DAY AND v != ''"
    );
    (sql, params.to_vec())
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
    let sql = catalog_sql();
    let rows = st.ch(&org).query(&sql).fetch_all::<Entry>().await?;
    let mut out: Vec<MetricCatalogEntry> = Vec::new();
    for e in rows {
        let mut tags: BTreeSet<String> = e.attr_keys.into_iter().collect();
        // `oc.org_id` is the tenant stamp, not something to filter or group on.
        for k in e.res_keys.into_iter().filter(|k| k != "oc.org_id") {
            tags.insert(short_tag(&k).map(str::to_string).unwrap_or(k));
        }
        let tags: Vec<String> = tags.into_iter().collect();
        let unit = dashboard::metric_unit(&e.name, &e.unit).filter(|u| u != "1");
        let description = dashboard::metric_description(&e.name, &e.description);
        if e.kind == "gauge" || e.kind == "sum" {
            out.push(MetricCatalogEntry {
                name: e.name,
                tags,
                unit,
                description,
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
            let description = if description.is_empty() {
                what.to_string()
            } else {
                format!("{description} — {}", what.to_lowercase())
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
    let (metrics, mut params) = tag_values_sql(&m_expr, &m_params);
    let mut sql = format!("SELECT DISTINCT v FROM ({metrics}");
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

#[derive(clickhouse::Row, Deserialize)]
pub(crate) struct VolumeRow {
    pub name: String,
    pub kind: String,
    /// Distinct series (tag combinations) seen in the range.
    pub series: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricVolumeEntry {
    pub name: String,
    pub kind: String,
    pub series: u64,
    /// 0..1 of all the metrics' series.
    pub share: f64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricVolume {
    pub total_series: u64,
    pub metrics: Vec<MetricVolumeEntry>,
}

/// Metrics by active series, most first (then by name), with their share of the total.
pub(crate) fn volume_page(rows: Vec<VolumeRow>) -> MetricVolume {
    let total_series: u64 = rows.iter().map(|r| r.series).sum();
    let mut metrics: Vec<MetricVolumeEntry> = rows
        .into_iter()
        .map(|r| MetricVolumeEntry { share: if total_series == 0 { 0.0 } else { r.series as f64 / total_series as f64 }, name: r.name, kind: r.kind, series: r.series })
        .collect();
    metrics.sort_by(|a, b| b.series.cmp(&a.series).then_with(|| a.name.cmp(&b.name)));
    MetricVolume { total_series, metrics }
}

#[derive(Deserialize, ToSchema)]
pub struct VolumeRequest {
    pub range: TimeRange,
}

/// Metrics by cardinality: the distinct series each reported in the range.
#[utoipa::path(post, path = "/api/v1/metrics/volume", request_body = VolumeRequest, responses((status = 200, body = MetricVolume)), tag = "metrics")]
pub async fn volume(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<VolumeRequest>) -> ApiResult<MetricVolume> {
    req.range.validate()?;
    let rows = st
        .ch(&org)
        .query(
            "SELECT MetricName AS name, toString(any(MetricType)) AS kind, uniqExact(SeriesId) AS series FROM otel.metrics_series \
             WHERE OrgId = {oc_org:String} AND LastSeen >= toDateTime(intDiv(?, 1000)) GROUP BY MetricName",
        )
        .bind(req.range.from)
        .fetch_all::<VolumeRow>()
        .await?;
    Ok(Json(volume_page(rows)))
}

#[cfg(test)]
mod tests {

    #[test]
    fn volume_ranks_metrics_by_active_series_with_their_share() {
        let rows = vec![
            VolumeRow { name: "system.cpu.user".into(), kind: "gauge".into(), series: 10 },
            VolumeRow { name: "kubernetes.cpu.usage.total".into(), kind: "gauge".into(), series: 30 },
            VolumeRow { name: "http.requests".into(), kind: "sum".into(), series: 10 },
        ];
        let v = volume_page(rows);
        assert_eq!(v.total_series, 50);
        assert_eq!(v.metrics.iter().map(|m| (m.name.as_str(), m.series)).collect::<Vec<_>>(), vec![("kubernetes.cpu.usage.total", 30), ("http.requests", 10), ("system.cpu.user", 10)]);
        assert_eq!(v.metrics[0].share, 0.6);
        assert_eq!(volume_page(vec![]).total_series, 0);
    }

    use super::*;

    fn exporter_table(d: Dist) -> &'static str {
        match d {
            Dist::Histogram => "otel.otel_metrics_histogram",
            Dist::ExpHistogram => "otel.otel_metrics_exponential_histogram",
            Dist::Summary => "otel.otel_metrics_summary",
        }
    }

    // Oracles: the same queries on the OpenTelemetry exporter's own tables (one row per point, labels
    // on every row), which the parity tests compare the storage against.

    fn exporter_points_query(
        kind: Kind,
        q: &MetricQuery,
        range: TimeRange,
        step_s: i64,
    ) -> (String, Vec<Param>) {
        let v = match kind {
            Kind::Gauge | Kind::UpDown => "avg(Value)".to_string(),
            Kind::Counter => "greatest(max(Value) - min(Value), 0) * 1000 / greatest(dateDiff('millisecond', min(TimeUnix), max(TimeUnix)), 1)".to_string(),
            Kind::Delta => format!("sum(Value) / {step_s}"),
        };
        let having = if kind == Kind::Counter {
            " HAVING count() > 1"
        } else {
            ""
        };
        let mut params = Vec::new();
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
        let sql = format!(
            "SELECT {g} AS g, toStartOfInterval(TimeUnix, INTERVAL {step_s} SECOND) AS t, cityHash64(toString(mapSort(ResourceAttributes)), toString(mapSort(Attributes))) AS sid, {v} AS v \
             FROM {table} WHERE OrgId = {{oc_org:String}} AND MetricName = ? AND TimeUnix >= fromUnixTimestamp64Milli(?) AND TimeUnix <= fromUnixTimestamp64Milli(?) AND {filter} \
             GROUP BY g, t, sid{having}",
            g = group_array(&by),
            table = if kind == Kind::Gauge { "otel.otel_metrics_gauge" } else { "otel.otel_metrics_sum" },
            filter = f.clause,
        );
        (sql, params)
    }

    fn exporter_catalog_sql() -> String {
        ["gauge", "sum"]
            .map(|kind| {
                format!(
                    "SELECT MetricName AS name, '{kind}' AS kind, any(MetricUnit) AS unit, any(MetricDescription) AS description, \
                     arrayDistinct(arrayFlatten(groupUniqArray(mapKeys(Attributes)))) AS attr_keys, \
                     arrayDistinct(arrayFlatten(groupUniqArray(mapKeys(ResourceAttributes)))) AS res_keys \
                     FROM otel.otel_metrics_{kind} WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 1 DAY GROUP BY MetricName"
                )
            })
            .join(" UNION ALL ")
    }

    fn exporter_tag_values_sql(expr: &str, params: &[Param]) -> (String, Vec<Param>) {
        let sql = format!(
            "SELECT DISTINCT v FROM (\
               SELECT toString({expr}) AS v FROM otel.otel_metrics_gauge WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 1 DAY \
               UNION ALL SELECT toString({expr}) AS v FROM otel.otel_metrics_sum WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 1 DAY \
               UNION ALL SELECT toString({expr}) AS v FROM otel.otel_metrics_histogram WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 1 DAY) WHERE v != ''"
        );
        (sql, [params, params, params].concat())
    }

    const DAY: i64 = 86_400_000;
    const NOW: i64 = 1_793_000_000_000;
    const TIERS: Tiers = Tiers {
        raw_days: 30,
        rollup_5m_days: 90,
        rollup_1h_days: 395,
    };

    fn ago(from_days: f64, to_days: f64) -> TimeRange {
        TimeRange {
            from: NOW - (from_days * DAY as f64) as i64,
            to: NOW - (to_days * DAY as f64) as i64,
        }
    }

    /// The same points queried from the exporter's tables and from series + samples / rollups give
    /// the same series and values. Skipped unless OC_TEST_CLICKHOUSE_URL points at a ClickHouse
    /// server (user `default`, or OC_TEST_CLICKHOUSE_USER / OC_TEST_CLICKHOUSE_PASSWORD).
    #[tokio::test]
    async fn new_storage_matches_the_exporter_tables() {
        let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_URL") else {
            eprintln!("OC_TEST_CLICKHOUSE_URL unset: skipping");
            return;
        };
        let user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
        let password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
        let cfg = oc_store::ClickHouseConfig {
            url: url.clone(),
            user: user.clone(),
            password: password.clone(),
            database: "oc".into(),
            cluster: None,
        };
        oc_store::migrate_clickhouse(&cfg, false)
            .await
            .expect("migrate");
        let org = format!("parity-{}", std::process::id());
        let ch = clickhouse::Client::default()
            .with_url(&url)
            .with_user(&user)
            .with_password(&password)
            .with_option("param_oc_org", &org);

        // Two hours from an hour boundary, a point every 10s: 3 gauge series, a cumulative counter
        // with a reset, and a delta counter. Each goes into the old table and the ingest table.
        let start = 1_793_095_200_i64; // 2026-10-27 10:00:00 UTC
        let gauge = format!(
            "SELECT map('oc.org_id', '{org}', 'host.name', concat('i-0a1b2c3d4e5f6000', toString(number % 3))) AS ResourceAttributes, 'storefront-api' AS ServiceName, \
             'system.cpu.utilization' AS MetricName, map('state', if(number % 2 = 0, 'user', 'system')) AS Attributes, \
             toDateTime64({start} + intDiv(number, 3) * 10, 9) AS TimeUnix, (number % 17) / 17 AS Value FROM numbers(2160)"
        );
        let counter = format!(
            "SELECT map('oc.org_id', '{org}', 'host.name', 'i-0a1b2c3d4e5f60001') AS ResourceAttributes, 'order-service' AS ServiceName, 'http.server.requests' AS MetricName, \
             map('route', '/cart') AS Attributes, toDateTime64({start} + number * 10, 9) AS TimeUnix, toFloat64(if(number < 400, number * 7, (number - 400) * 3)) AS Value, \
             2 AS AggregationTemporality, true AS IsMonotonic FROM numbers(720)"
        );
        let delta = format!(
            "SELECT map('oc.org_id', '{org}') AS ResourceAttributes, 'billing' AS ServiceName, 'billing.charges' AS MetricName, map('currency', 'eur') AS Attributes, \
             toDateTime64({start} + number * 10, 9) AS TimeUnix, toFloat64(number % 5) AS Value, 1 AS AggregationTemporality, true AS IsMonotonic FROM numbers(720)"
        );
        for (table, select) in [("gauge", &gauge), ("sum", &counter), ("sum", &delta)] {
            let cols = "ResourceAttributes, ServiceName, MetricName, Attributes, TimeUnix, Value";
            let cols = if table == "sum" {
                format!("{cols}, AggregationTemporality, IsMonotonic")
            } else {
                cols.to_string()
            };
            for target in [
                format!("otel.otel_metrics_{table}"),
                format!("otel.metrics_ingest_{table}"),
            ] {
                ch.query(&format!(
                    "INSERT INTO {target} ({cols}) SELECT {cols} FROM ({select})"
                ))
                .execute()
                .await
                .unwrap_or_else(|e| panic!("{target}: {e}"));
            }
        }

        let range = TimeRange {
            from: start * 1000,
            to: (start + 7200) * 1000 - 1,
        };
        let cases = [
            ("avg:system.cpu.utilization{*}", Kind::Gauge),
            ("max:system.cpu.utilization{*} by {host}", Kind::Gauge),
            (
                "sum:system.cpu.utilization{state:user} by {host,state}",
                Kind::Gauge,
            ),
            ("sum:http.server.requests{*}", Kind::Counter),
            (
                "sum:billing.charges{currency:eur} by {service}",
                Kind::Delta,
            ),
        ];
        for (query, kind) in cases {
            let q = parse(query).unwrap();
            for (step_s, source) in [
                (30, Source::Samples),
                (600, Source::Rollup5m),
                (3600, Source::Rollup1h),
            ] {
                let run = |source: Option<Source>| {
                    let (inner, params) = match source {
                        Some(source) => points_query(kind, source, &q, range, step_s, 1.0),
                        None => exporter_points_query(kind, &q, range, step_s),
                    };
                    let sql = format!(
                        "SELECT g, toInt64(toUnixTimestamp(t)) * 1000 AS ts, toFloat64({agg}(v)) AS value FROM ({inner}) WHERE isFinite(v) GROUP BY g, t ORDER BY g, t",
                        agg = q.agg.sql()
                    );
                    let ch = ch.clone();
                    async move {
                        bind(ch.query(&sql), &params)
                            .fetch_all::<Row>()
                            .await
                            .unwrap_or_else(|e| panic!("{sql}: {e}"))
                    }
                };
                let (old, new) = (run(None).await, run(Some(source)).await);
                assert!(!old.is_empty(), "{query} step {step_s}: no data");
                assert_eq!(
                    old.len(),
                    new.len(),
                    "{query} step {step_s} from {source:?}: number of points"
                );
                for (o, n) in old.iter().zip(&new) {
                    assert_eq!(
                        (&o.g, o.ts),
                        (&n.g, n.ts),
                        "{query} step {step_s} from {source:?}"
                    );
                    assert!(
                        (o.value - n.value).abs() <= 1e-9 * o.value.abs().max(1.0),
                        "{query} step {step_s} from {source:?} at {}: {} vs {}",
                        o.ts,
                        o.value,
                        n.value
                    );
                }
            }
        }
    }

    /// Readers that query points directly (hosts, Kubernetes), the metric catalog and tag values
    /// give the same rows from the exporter's tables and from series + samples. Skipped unless
    /// OC_TEST_CLICKHOUSE_URL is set (see above).
    #[tokio::test]
    async fn direct_readers_match_the_exporter_tables() {
        let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_URL") else {
            eprintln!("OC_TEST_CLICKHOUSE_URL unset: skipping");
            return;
        };
        let user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
        let password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
        let cfg = oc_store::ClickHouseConfig {
            url: url.clone(),
            user: user.clone(),
            password: password.clone(),
            database: "oc".into(),
            cluster: None,
        };
        oc_store::migrate_clickhouse(&cfg, false)
            .await
            .expect("migrate");
        let org = format!("readers-{}", std::process::id());
        let now = now_ms();
        // Open-ended like the `now() - INTERVAL` readers: points written after `now` count too.
        let (from, to) = (now - 15 * 60_000, now + OPEN_END_MS);
        let ch = with_window(
            clickhouse::Client::default()
                .with_url(&url)
                .with_user(&user)
                .with_password(&password)
                .with_option("param_oc_org", &org),
            from,
            to,
        );

        // Two hosts with CPU by state and memory, pods with a process metric (sum), over 10 minutes.
        let gauge = format!(
            "SELECT map('oc.org_id', '{org}', 'host.name', concat('i-0a1b2c3d4e5f6000', toString(number % 2)), 'k8s.pod.name', concat('storefront-', toString(number % 4)), 'cloud.region', 'eu-west-1') AS ResourceAttributes, \
             'storefront-api' AS ServiceName, if(number % 3 = 0, 'system.memory.utilization', 'system.cpu.utilization') AS MetricName, \
             map('state', ['user', 'idle', 'used'][number % 3 + 1]) AS Attributes, now64(9) - toIntervalSecond(number) AS TimeUnix, (number % 11) / 11 AS Value FROM numbers(600)"
        );
        let sum = format!(
            "SELECT map('oc.org_id', '{org}', 'host.name', 'i-0a1b2c3d4e5f60001', 'process.pid', toString(number % 3)) AS ResourceAttributes, 'order-service' AS ServiceName, \
             'process.memory.usage' AS MetricName, map() AS Attributes, now64(9) - toIntervalSecond(number) AS TimeUnix, toFloat64(number * 1000) AS Value, 2 AS AggregationTemporality, false AS IsMonotonic FROM numbers(300)"
        );
        for (table, select) in [("gauge", &gauge), ("sum", &sum)] {
            let cols = "ResourceAttributes, ServiceName, MetricName, Attributes, TimeUnix, Value";
            let cols = if table == "sum" {
                format!("{cols}, AggregationTemporality, IsMonotonic")
            } else {
                cols.to_string()
            };
            for target in [
                format!("otel.otel_metrics_{table}"),
                format!("otel.metrics_ingest_{table}"),
            ] {
                ch.query(&format!(
                    "INSERT INTO {target} ({cols}) SELECT {cols} FROM ({select})"
                ))
                .execute()
                .await
                .unwrap_or_else(|e| panic!("{target}: {e}"));
            }
        }

        async fn rows(ch: &clickhouse::Client, sql: &str) -> Vec<String> {
            ch.query(&format!(
                "SELECT toString(tuple(*)) FROM ({sql}) ORDER BY 1"
            ))
            .fetch_all::<String>()
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
        }
        // Shaped like the host list and the Kubernetes pod usage queries.
        let host_query = |src: &str| {
            format!(
                "SELECT ResourceAttributes['host.name'] AS name, ServiceName, round(avgIf(Value, Attributes['state'] = 'user'), 9) AS user, count() AS n, \
                 toJSONString(mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), argMax(ResourceAttributes, TimeUnix)))) AS res \
                 FROM {src} WHERE OrgId = {{oc_org:String}} AND MetricName IN ('system.cpu.utilization', 'system.memory.utilization') \
                 AND TimeUnix >= fromUnixTimestamp64Milli({{oc_from:Int64}}) GROUP BY name, ServiceName"
            )
        };
        let pod_query = |src: &str| {
            format!("SELECT ResourceAttributes['k8s.pod.name'] AS pod, round(avg(Value), 9) AS cpu FROM {src} WHERE OrgId = {{oc_org:String}} AND MetricName = 'system.cpu.utilization' AND TimeUnix > now() - INTERVAL 15 MINUTE GROUP BY pod")
        };
        let process_query = |src: &str| {
            format!("SELECT ResourceAttributes['process.pid'] AS pid, argMax(Value, TimeUnix) AS mem FROM {src} WHERE OrgId = {{oc_org:String}} AND MetricName = 'process.memory.usage' GROUP BY pid")
        };
        let gauges = points_subquery(
            "gauge",
            &["system.cpu.utilization", "system.memory.utilization"],
        );
        let sums = points_subquery("sum", &["process.memory.usage"]);
        for (old, new) in [
            (host_query("otel.otel_metrics_gauge"), host_query(&gauges)),
            (pod_query("otel.otel_metrics_gauge"), pod_query(&gauges)),
            (process_query("otel.otel_metrics_sum"), process_query(&sums)),
        ] {
            let (o, n) = (rows(&ch, &old).await, rows(&ch, &new).await);
            assert!(!o.is_empty(), "no data: {old}");
            assert_eq!(o, n, "{new}");
        }

        // The catalog and tag values (`oc.*` resource keys are the org's stamps, not tags).
        let catalog = |new: bool| {
            let sql = if new {
                format!(
                    "SELECT * FROM ({}) WHERE kind IN ('gauge', 'sum')",
                    catalog_sql()
                )
            } else {
                exporter_catalog_sql()
            };
            format!("SELECT name, kind, unit, description, arraySort(arrayFilter(k -> NOT startsWith(k, 'oc.'), attr_keys)), arraySort(arrayFilter(k -> NOT startsWith(k, 'oc.'), res_keys)) FROM ({sql})")
        };
        assert_eq!(
            rows(&ch, &catalog(false)).await,
            rows(&ch, &catalog(true)).await
        );
        assert_eq!(rows(&ch, &catalog(true)).await.len(), 3);
        for key in ["host", "pod_name", "region", "state"] {
            let (expr, p) = metric_field(key);
            let values = |new: bool| {
                let sql = if new {
                    tag_values_sql(&expr, &p)
                } else {
                    exporter_tag_values_sql(&expr, &p)
                };
                let ch = ch.clone();
                async move {
                    bind(
                        ch.query(&format!("SELECT * FROM ({}) ORDER BY 1", sql.0)),
                        &sql.1,
                    )
                    .fetch_all::<String>()
                    .await
                    .unwrap_or_else(|e| panic!("{}: {e}", sql.0))
                }
            };
            let (o, n) = (values(false).await, values(true).await);
            assert!(!o.is_empty(), "no values for {key}");
            assert_eq!(o, n, "values of {key}");
        }
    }

    /// The distribution query on the exporter's tables (the oracle).
    fn exporter_dist_query(
        dist: Dist,
        stat: &str,
        q: &MetricQuery,
        range: TimeRange,
        step_s: i64,
    ) -> (String, Vec<Param>) {
        let mut params = Vec::new();
        let mut by = Vec::new();
        for k in &q.by {
            let (e, p) = metric_field(k);
            by.push(e);
            params.extend(p);
        }
        let f = metric_filter(&q.filters, metric_field);
        params.extend([
            Param::Str(split_stat(&q.metric).unwrap().0.to_string()),
            Param::Int(range.from),
            Param::Int(range.to),
        ]);
        params.extend(f.params);
        let sql = dist_query(
            exporter_table(dist),
            dist,
            stat,
            &group_array(&by),
            step_s,
            &f.clause,
        )
        .unwrap();
        (sql, params)
    }

    /// Histograms (cumulative and delta), exponential histograms and summaries give the same
    /// statistics from the storage, raw and from rollups, as from the exporter's tables. Skipped
    /// unless OC_TEST_CLICKHOUSE_URL is set (see above).
    #[tokio::test]
    async fn distributions_match_the_exporter_tables() {
        let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_URL") else {
            eprintln!("OC_TEST_CLICKHOUSE_URL unset: skipping");
            return;
        };
        let user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
        let password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
        let cfg = oc_store::ClickHouseConfig {
            url: url.clone(),
            user: user.clone(),
            password: password.clone(),
            database: "oc".into(),
            cluster: None,
        };
        oc_store::migrate_clickhouse(&cfg, false)
            .await
            .expect("migrate");
        let org = format!("dists-{}", std::process::id());
        let ch = clickhouse::Client::default()
            .with_url(&url)
            .with_user(&user)
            .with_password(&password)
            .with_option("param_oc_org", &org);

        // Two hours from an hour boundary, a point every 10s per series (two hosts).
        let start = 1_793_095_200_i64;
        let common = |name: &str| {
            format!(
                "map('oc.org_id', '{org}', 'host.name', concat('i-0a1b2c3d4e5f6000', toString(number % 2))) AS ResourceAttributes, 'checkout' AS ServiceName, \
                 '{name}' AS MetricName, 'ms' AS MetricUnit, map('route', '/cart') AS Attributes, toDateTime64({start} + intDiv(number, 2) * 10, 9) AS TimeUnix"
            )
        };
        // Per point: n grows with time; buckets (≤10, ≤100, ≤1000, +inf) take shares of it.
        let hist = |name: &str, temporality: i32| {
            let n = if temporality == 2 {
                "intDiv(number, 2) * 7 + number % 2 * 3"
            } else {
                "(number % 5) + 1"
            };
            format!(
                "SELECT {c}, ({n}) AS n, toUInt64(n * 4) AS Count, toFloat64(n * 4 * 55.5) AS Sum, [toUInt64(n), toUInt64(n * 2), toUInt64(n), toUInt64(0)] AS BucketCounts, \
                 [10., 100., 1000.] AS ExplicitBounds, toFloat64(1 + number % 7) AS Min, toFloat64(900 + number % 50) AS Max, {temporality} AS AggregationTemporality FROM numbers(1440)",
                c = common(name)
            )
        };
        let exp = format!(
            "SELECT {c}, intDiv(number, 2) * 5 + 1 AS n, toUInt64(n * 6) AS Count, toFloat64(n * 6 * 12.5) AS Sum, 2 AS Scale, toUInt64(0) AS ZeroCount, 3 AS PositiveOffset, \
             [toUInt64(n), toUInt64(n * 3), toUInt64(n * 2), toUInt64(0)] AS PositiveBucketCounts, 0 AS NegativeOffset, CAST([], 'Array(UInt64)') AS NegativeBucketCounts, \
             toFloat64(2) AS Min, toFloat64(40 + number % 9) AS Max, 2 AS AggregationTemporality FROM numbers(1440)",
            c = common("rpc.duration")
        );
        let summary = format!(
            "SELECT {c}, toUInt64(intDiv(number, 2) * 3) AS Count, toFloat64(intDiv(number, 2) * 3 * 20) AS Sum, [0., 0.5, 0.99, 1.] AS `ValueAtQuantiles.Quantile`, \
             [toFloat64(1 + number % 3), toFloat64(20 + number % 11), toFloat64(80 + number % 13), toFloat64(120 + number % 17)] AS `ValueAtQuantiles.Value` FROM numbers(1440)",
            c = common("gc.pause")
        );
        let hist_cols = "ResourceAttributes, ServiceName, MetricName, MetricUnit, Attributes, TimeUnix, Count, Sum, BucketCounts, ExplicitBounds, Min, Max, AggregationTemporality";
        let exp_cols = "ResourceAttributes, ServiceName, MetricName, MetricUnit, Attributes, TimeUnix, Count, Sum, Scale, ZeroCount, PositiveOffset, PositiveBucketCounts, NegativeOffset, NegativeBucketCounts, Min, Max, AggregationTemporality";
        let summary_cols = "ResourceAttributes, ServiceName, MetricName, MetricUnit, Attributes, TimeUnix, Count, Sum, `ValueAtQuantiles.Quantile`, `ValueAtQuantiles.Value`";
        for (table, cols, select) in [
            ("histogram", hist_cols, hist("http.server.duration", 2)),
            ("histogram", hist_cols, hist("queue.wait", 1)),
            ("exponential_histogram", exp_cols, exp),
            ("summary", summary_cols, summary),
        ] {
            for target in [
                format!("otel.otel_metrics_{table}"),
                format!("otel.metrics_ingest_{table}"),
            ] {
                ch.query(&format!(
                    "INSERT INTO {target} ({cols}) SELECT {cols} FROM ({select})"
                ))
                .execute()
                .await
                .unwrap_or_else(|e| panic!("{target}: {e}"));
            }
        }

        let range = TimeRange {
            from: start * 1000,
            to: (start + 7200) * 1000 - 1,
        };
        let metrics = [
            ("http.server.duration", Dist::Histogram),
            ("queue.wait", Dist::Histogram),
            ("rpc.duration", Dist::ExpHistogram),
            ("gc.pause", Dist::Summary),
        ];
        for (base, dist) in metrics {
            for stat in ["count", "sum", "avg", "min", "max", "p50", "p99"] {
                for by in ["", " by {host}"] {
                    let q = parse(&format!("avg:{base}.{stat}{{*}}{by}")).unwrap();
                    for (step_s, source) in [
                        (30, Source::Samples),
                        (600, Source::Rollup5m),
                        (3600, Source::Rollup1h),
                    ] {
                        let run = |source: Option<Source>| {
                            let (inner, params) = match source {
                                Some(source) => {
                                    dist_points_query(dist, stat, source, &q, range, step_s)
                                        .unwrap()
                                }
                                None => exporter_dist_query(dist, stat, &q, range, step_s),
                            };
                            let sql = format!("SELECT g, toInt64(toUnixTimestamp(t)) * 1000 AS ts, toFloat64(avg(v)) AS value FROM ({inner}) WHERE isFinite(v) GROUP BY g, t ORDER BY g, t");
                            let ch = ch.clone();
                            async move {
                                bind(ch.query(&sql), &params)
                                    .fetch_all::<Row>()
                                    .await
                                    .unwrap_or_else(|e| panic!("{sql}: {e}"))
                            }
                        };
                        let (old, new) = (run(None).await, run(Some(source)).await);
                        let what = format!("{base}.{stat}{by} step {step_s} from {source:?}");
                        assert!(!old.is_empty(), "{what}: no data");
                        assert_eq!(old.len(), new.len(), "{what}: number of points");
                        for (o, n) in old.iter().zip(&new) {
                            assert_eq!((&o.g, o.ts), (&n.g, n.ts), "{what}");
                            assert!(
                                (o.value - n.value).abs() <= 1e-9 * o.value.abs().max(1.0),
                                "{what} at {}: {} vs {}",
                                o.ts,
                                o.value,
                                n.value
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn metric_ranges_reach_the_longest_retention() {
        // Metrics read rollups for long ranges, so they may span what the 1-hour rollup keeps.
        assert!(ago(90.0, 0.0).validate_within(395).is_ok());
        assert!(ago(395.0, 0.0).validate_within(395).is_ok());
        assert!(ago(396.0, 0.0).validate_within(395).is_err());
        assert!(TimeRange {
            from: NOW,
            to: NOW - 1
        }
        .validate_within(395)
        .is_err());
    }

    #[test]
    fn resolution_follows_the_step() {
        // Short ranges read raw samples, longer ones the coarsest rollup that fits the step.
        assert_eq!(
            resolution(ago(1.0 / 24.0, 0.0), 30_000, NOW, TIERS),
            (Source::Samples, 30_000)
        );
        assert_eq!(
            resolution(ago(7.0, 0.0), 600_000, NOW, TIERS),
            (Source::Rollup5m, 600_000)
        );
        assert_eq!(
            resolution(ago(30.0, 0.0), 3_600_000, NOW, TIERS),
            (Source::Rollup1h, 3_600_000)
        );
        assert_eq!(
            resolution(ago(365.0, 0.0), 43_200_000, NOW, TIERS),
            (Source::Rollup1h, 43_200_000)
        );
    }

    #[test]
    fn resolution_falls_back_to_what_is_still_kept() {
        // An hour 60 days ago: raw samples are gone (30 days), the 5-minute rollup has it.
        assert_eq!(
            resolution(ago(60.0, 60.0 - 1.0 / 24.0), 30_000, NOW, TIERS),
            (Source::Rollup5m, 300_000)
        );
        // 200 days ago: only the 1-hour rollup.
        assert_eq!(
            resolution(ago(200.0, 199.0), 600_000, NOW, TIERS),
            (Source::Rollup1h, 3_600_000)
        );
        // Older than everything: the longest-kept rollup (it returns nothing rather than failing).
        assert_eq!(
            resolution(ago(500.0, 499.0), 30_000, NOW, TIERS),
            (Source::Rollup1h, 3_600_000)
        );
    }

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
                let sql = dist_query(exporter_table(d), d, stat, "[]", 30, "1").unwrap();
                assert!(
                    sql.contains("OrgId = {oc_org:String}"),
                    "{d:?} {stat} must be org-scoped"
                );
            }
        }
    }
}
