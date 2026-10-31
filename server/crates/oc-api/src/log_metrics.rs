//! Logs-based metrics (Logs → Configuration → Generate Metrics): the indexer matches every log,
//! after pipelines and before index routing (so excluded or unindexed logs count too), against its
//! org's metric definitions and aggregates each request's matches into short buckets per (org,
//! metric, group-by values). The indexer persists these buckets to a bounded outbox before source
//! acknowledgement; its worker writes the metric ingest tables as the Collector would:
//!   count        → delta, monotonic sum (as Datadog `count` series are stored by the intake);
//!   distribution → delta exponential histogram (as Datadog distributions are), percentiles included.
//! Group-by values become tags: `service` is the series' service, the Datadog tags that map to
//! resource attributes (`host`, `env`, `kube_namespace`, …) become those, the rest data point
//! attributes. A log without a group-by value is counted under `N/A`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use oc_query::{resource_alias, Fields};
use serde_json::{json, Map, Value};

use crate::pipelines::dates::to_rfc3339;
use crate::pipelines::Filter;

/// Width of the aggregation buckets.
pub const BUCKET_MS: i64 = 10_000;
/// Distinct group-by value sets kept per metric in an aggregation batch; further series are dropped.
pub const MAX_SERIES_PER_METRIC: usize = 1_000;
/// Logs whose date is further in the past are not counted.
pub const MAX_LATENESS_MS: i64 = 20 * 60_000;
/// The tag value of a log that lacks a group-by field.
pub const MISSING: &str = "N/A";
/// Exponential histogram scale: base 2^(1/64), about 1.1 % per bucket (as converted Datadog sketches).
pub const SCALE: i32 = 6;

/// Modern JSON IDs carry server acceptance time. Use that stable clock for lateness and future
/// clamping so delayed Kafka replay does not age previously eligible logs out of metric generation.
pub fn processing_clock(id: &str, fallback: i64) -> i64 {
    let hex: String = id.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) || hex.as_bytes()[12] != b'7' {
        return fallback;
    }
    i64::from_str_radix(&hex[..12], 16).unwrap_or(fallback)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Aggregation {
    Count,
    /// Of a numeric attribute (`@duration`).
    Distribution {
        path: String,
    },
}

/// One compiled metric definition.
pub struct Definition {
    pub name: String,
    filter: Filter,
    aggregation: Aggregation,
    /// (path read from the log, tag name on the metric).
    group_by: Vec<(String, String)>,
    description: Arc<str>,
}

impl Definition {
    pub fn new(name: &str, query: &str, aggregation: Aggregation, group_by: &[(String, String)]) -> Definition {
        let description = if query.trim().is_empty() { "Generated from all logs".to_string() } else { format!("Generated from logs matching: {}", query.trim()) };
        Definition { name: name.to_string(), filter: Filter::parse(query), aggregation, group_by: group_by.to_vec(), description: description.into() }
    }

    /// A definition as stored in `log_metrics` (checked by the API): `group_by` is
    /// `[{"path": "@http.status_code", "tagName": "http.status_code"}]`; the tag name defaults to the path.
    pub fn stored(name: &str, query: &str, aggregation: &str, path: Option<&str>, group_by: &Value) -> Definition {
        let aggregation = match (aggregation, path) {
            ("distribution", Some(p)) => Aggregation::Distribution { path: p.to_string() },
            _ => Aggregation::Count,
        };
        let group_by: Vec<(String, String)> = group_by
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|g| {
                let path = g.get("path")?.as_str()?;
                let tag = g.get("tagName").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or(path.trim_start_matches('@'));
                Some((path.to_string(), tag.to_string()))
            })
            .collect();
        Definition::new(name, query, aggregation, &group_by)
    }
}

/// Days the org keeps metrics at full resolution and as 5-minute and 1-hour rollups (0 = the install's default).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Retention {
    pub raw: i32,
    pub rollup_5m: i32,
    pub rollup_1h: i32,
}

/// An org's definitions and what its metric rows carry.
pub struct OrgMetrics {
    pub definitions: Vec<Definition>,
    pub retention: Retention,
}

/// A distribution's observations in one bucket.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Histogram {
    pub count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub zero_count: u64,
    pub positive: BTreeMap<i32, u64>,
    pub negative: BTreeMap<i32, u64>,
}

impl Histogram {
    pub fn add(&mut self, v: f64) {
        if !v.is_finite() {
            return;
        }
        (self.min, self.max) = if self.count == 0 { (v, v) } else { (self.min.min(v), self.max.max(v)) };
        self.count += 1;
        self.sum += v;
        match v.partial_cmp(&0.0) {
            Some(std::cmp::Ordering::Greater) => *self.positive.entry(bucket_index(v)).or_insert(0) += 1,
            Some(std::cmp::Ordering::Less) => *self.negative.entry(bucket_index(-v)).or_insert(0) += 1,
            _ => self.zero_count += 1,
        }
    }
}

/// The exponential histogram bucket of |v| > 0 at [`SCALE`]: bucket i holds (base^i, base^(i+1)].
pub fn bucket_index(v: f64) -> i32 {
    (v.log2() * f64::from(1 << SCALE)).ceil() as i32 - 1
}

#[derive(Clone, Debug, PartialEq)]
pub enum PointValue {
    Count(u64),
    Distribution(Histogram),
}

/// An aggregated bucket of one series, ready to be written.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub org: String,
    pub metric: String,
    pub description: Arc<str>,
    pub retention: Retention,
    /// (tag, value), in the definition's group-by order.
    pub tags: Vec<(String, String)>,
    /// Bucket start (epoch ms).
    pub time_ms: i64,
    pub value: PointValue,
}

/// (org, metric name).
pub type MetricKey = (String, String);
/// (org, metric name, group-by values, bucket start).
type SeriesKey = (String, String, Vec<String>, i64);

struct Pending {
    description: Arc<str>,
    retention: Retention,
    tag_names: Vec<String>,
    value: PointValue,
}

/// The buckets not flushed yet.
#[derive(Default)]
pub struct Aggregator {
    pending: HashMap<SeriesKey, Pending>,
    /// Group-by value sets seen per (org, metric) since the last flush (the series cap).
    series: HashMap<MetricKey, HashSet<Vec<String>>>,
    /// Logs not counted per (org, metric) because the metric had too many series.
    dropped: HashMap<MetricKey, u64>,
}

impl Aggregator {
    /// Counts one log (dated `ts_ms`) in every definition of its org that it matches.
    /// A log dated in the future counts now; one older than [`MAX_LATENESS_MS`] doesn't count.
    pub fn record(&mut self, org: &str, metrics: &OrgMetrics, f: &Fields<'_>, ts_ms: i64, now_ms: i64) {
        let ts = ts_ms.min(now_ms);
        if ts < now_ms - MAX_LATENESS_MS {
            return;
        }
        let bucket = ts.div_euclid(BUCKET_MS) * BUCKET_MS;
        for d in &metrics.definitions {
            if !d.filter.matches(f) {
                continue;
            }
            let observed = match &d.aggregation {
                Aggregation::Count => None,
                // A log without a numeric value isn't an observation.
                Aggregation::Distribution { path } => match numeric(f, path) {
                    Some(v) => Some(v),
                    None => continue,
                },
            };
            let values: Vec<String> = d.group_by.iter().map(|(path, _)| field(f, path).unwrap_or_else(|| MISSING.to_string())).collect();
            let metric = (org.to_string(), d.name.clone());
            let seen = self.series.entry(metric.clone()).or_default();
            if !seen.contains(&values) {
                if seen.len() >= MAX_SERIES_PER_METRIC {
                    *self.dropped.entry(metric).or_insert(0) += 1;
                    continue;
                }
                seen.insert(values.clone());
            }
            let pending = self.pending.entry((metric.0, metric.1, values, bucket)).or_insert_with(|| Pending {
                description: d.description.clone(),
                retention: metrics.retention,
                tag_names: d.group_by.iter().map(|(_, tag)| tag.clone()).collect(),
                value: if observed.is_some() { PointValue::Distribution(Histogram::default()) } else { PointValue::Count(0) },
            });
            match (&mut pending.value, observed) {
                (PointValue::Count(n), _) => *n += 1,
                (PointValue::Distribution(h), Some(v)) => h.add(v),
                (PointValue::Distribution(_), None) => {}
            }
        }
    }

    /// Takes every pending bucket, and the logs dropped per (org, metric) by the series cap.
    /// A bucket still filling is taken too: its next flush adds another delta point for it.
    pub fn drain(&mut self) -> (Vec<Point>, Vec<(MetricKey, u64)>) {
        let points = self
            .pending
            .drain()
            .map(|((org, metric, values, time_ms), p)| Point {
                org,
                metric,
                description: p.description,
                retention: p.retention,
                tags: p.tag_names.into_iter().zip(values).collect(),
                time_ms,
                value: p.value,
            })
            .collect();
        self.series.clear();
        (points, self.dropped.drain().collect())
    }
}

/// A log's value for a group-by path: a reserved field, `@attribute` or tag.
fn field(f: &Fields<'_>, path: &str) -> Option<String> {
    let v = match path {
        "service" => f.service,
        "status" => f.status,
        "host" => f.host,
        "source" => f.source,
        p if p.starts_with('@') => f.attr.get(&p[1..])?.as_str(),
        p => f.tags.get(p)?.as_str(),
    };
    (!v.is_empty()).then(|| v.to_string())
}

/// A log's numeric value of `@path`.
fn numeric(f: &Fields<'_>, path: &str) -> Option<f64> {
    f.attr.get(path.trim_start_matches('@')).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite())
}

/// A point's tags as the Collector stores them: (service name, resource attributes, data point attributes).
fn placed(p: &Point) -> (String, Map<String, Value>, Map<String, Value>) {
    let mut service = String::new();
    let mut resource = Map::new();
    let mut attrs = Map::new();
    resource.insert("oc.org_id".into(), Value::from(p.org.as_str()));
    let r = p.retention;
    for (k, days) in [("oc.metrics_retention_days", r.raw), ("oc.metrics_retention_5m_days", r.rollup_5m), ("oc.metrics_retention_1h_days", r.rollup_1h)] {
        resource.insert(k.into(), Value::from(days.to_string()));
    }
    for (tag, v) in &p.tags {
        if tag == "service" {
            service = v.clone();
            resource.insert("service.name".into(), Value::from(v.as_str()));
        } else if let Some(key) = resource_alias(tag) {
            resource.insert(key.into(), Value::from(v.as_str()));
        } else {
            attrs.insert(tag.clone(), Value::from(v.as_str()));
        }
    }
    (service, resource, attrs)
}

/// Contiguous counts from the lowest bucket: (offset, counts).
fn dense(buckets: &BTreeMap<i32, u64>) -> (i32, Vec<u64>) {
    match (buckets.first_key_value(), buckets.last_key_value()) {
        (Some((&lo, _)), Some((&hi, _))) => (lo, (lo..=hi).map(|i| buckets.get(&i).copied().unwrap_or(0)).collect()),
        _ => (0, Vec::new()),
    }
}

fn row(p: &Point, scope: &str) -> Value {
    let (service, resource, attrs) = placed(p);
    let time = to_rfc3339(p.time_ms);
    let mut row = json!({
        "ResourceAttributes": resource,
        "ServiceName": service,
        "ScopeName": scope,
        "MetricName": p.metric,
        "MetricDescription": &*p.description,
        "MetricUnit": "",
        "Attributes": attrs,
        "StartTimeUnix": time,
        "TimeUnix": time,
        "AggregationTemporality": 1,
    });
    let fields = match &p.value {
        PointValue::Count(n) => json!({"Value": *n as f64, "IsMonotonic": true}),
        PointValue::Distribution(h) => {
            let ((pos_offset, pos), (neg_offset, neg)) = (dense(&h.positive), dense(&h.negative));
            json!({
                "Count": h.count, "Sum": h.sum, "Min": h.min, "Max": h.max, "Scale": SCALE, "ZeroCount": h.zero_count,
                "PositiveOffset": pos_offset, "PositiveBucketCounts": pos, "NegativeOffset": neg_offset, "NegativeBucketCounts": neg,
            })
        }
    };
    if let (Some(o), Value::Object(extra)) = (row.as_object_mut(), fields) {
        o.extend(extra);
    }
    row
}

/// The JSON rows for `otel.metrics_ingest_sum` and `otel.metrics_ingest_exponential_histogram`
/// (`INSERT … FORMAT JSONEachRow`), shaped as the Collector's exporter writes them.
pub fn rows(points: &[Point]) -> (Vec<u8>, Vec<u8>) {
    rows_in_scope(points, "observecraft.log_metrics")
}

/// [`rows`] for metrics the server writes for another feature (`ScopeName` = `scope`).
pub fn rows_in_scope(points: &[Point], scope: &str) -> (Vec<u8>, Vec<u8>) {
    let (mut sums, mut histograms) = (Vec::new(), Vec::new());
    for p in points {
        let out = if matches!(p.value, PointValue::Count(_)) { &mut sums } else { &mut histograms };
        if serde_json::to_writer(&mut *out, &row(p, scope)).is_ok() {
            out.push(b'\n');
        }
    }
    (sums, histograms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_792_933_205_000;

    #[test]
    fn acceptance_clock_is_stable_across_replay_and_legacy_ids_keep_the_fallback() {
        let hex = format!("{NOW:012x}");
        let id = format!("{}-{}-7000-8000-000000000001", &hex[..8], &hex[8..]);
        assert_eq!(processing_clock(&id, NOW + 72 * 3_600_000), NOW);
        assert_eq!(processing_clock("00000000-0000-4000-8000-000000000001", NOW), NOW);
        assert_eq!(processing_clock("not-an-id", NOW), NOW);
    }

    struct Log {
        service: &'static str,
        status: &'static str,
        tags: HashMap<String, String>,
        attr: HashMap<String, String>,
    }

    fn log(service: &'static str, status: &'static str, attr: &[(&str, &str)]) -> Log {
        Log { service, status, tags: HashMap::from([("env".to_string(), "prod".to_string())]), attr: attr.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn fields(l: &Log) -> Fields<'_> {
        Fields { service: l.service, status: l.status, host: "i-0a1b2c3d4e5f60001", source: "nginx", message: "GET /cart", tags: &l.tags, attr: &l.attr, index: None }
    }

    fn group(paths: &[&str]) -> Vec<(String, String)> {
        paths.iter().map(|p| (p.to_string(), p.trim_start_matches('@').to_string())).collect()
    }

    fn org(definitions: Vec<Definition>) -> OrgMetrics {
        OrgMetrics { definitions, retention: Retention { raw: 15, rollup_5m: 90, rollup_1h: 395 } }
    }

    /// (metric, tags, count) of the count points.
    type Counted = (String, Vec<(String, String)>, u64);

    fn counts(points: &[Point]) -> Vec<Counted> {
        let mut out: Vec<_> = points
            .iter()
            .filter_map(|p| match &p.value {
                PointValue::Count(n) => Some((p.metric.clone(), p.tags.clone(), *n)),
                PointValue::Distribution(_) => None,
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn counts_matching_logs_per_group() {
        let m = org(vec![
            Definition::new("logs.checkout.errors", "service:(order-service OR billing) status:error", Aggregation::Count, &group(&["service", "@http.status_code"])),
            Definition::new("logs.all", "", Aggregation::Count, &[]),
        ]);
        let mut agg = Aggregator::default();
        let logs = [
            log("order-service", "error", &[("http.status_code", "502")]),
            log("order-service", "error", &[("http.status_code", "502")]),
            log("billing", "error", &[]),
            log("order-service", "info", &[("http.status_code", "200")]),
            log("api-gateway", "error", &[("http.status_code", "500")]),
        ];
        for l in &logs {
            agg.record("acme", &m, &fields(l), NOW - 1_000, NOW);
        }
        let (points, dropped) = agg.drain();
        assert!(dropped.is_empty());
        let tags = |s: &str, c: &str| vec![("service".to_string(), s.to_string()), ("http.status_code".to_string(), c.to_string())];
        assert_eq!(
            counts(&points),
            vec![("logs.all".into(), vec![], 5), ("logs.checkout.errors".into(), tags("billing", MISSING), 1), ("logs.checkout.errors".into(), tags("order-service", "502"), 2)]
        );
        assert!(points.iter().all(|p| p.org == "acme" && p.retention.raw == 15));
        // Drained: the next flush starts empty.
        assert!(agg.drain().0.is_empty());
    }

    #[test]
    fn stored_definitions() {
        let d = Definition::stored("logs.latency", "service:billing", "distribution", Some("@duration"), &json!([{"path": "@http.status_code", "tagName": "code"}, {"path": "service"}]));
        assert_eq!(d.aggregation, Aggregation::Distribution { path: "@duration".into() });
        assert_eq!(d.group_by, vec![("@http.status_code".to_string(), "code".to_string()), ("service".to_string(), "service".to_string())]);
        assert_eq!(&*d.description, "Generated from logs matching: service:billing");
        let c = Definition::stored("logs.all", "", "count", None, &json!(null));
        assert_eq!((c.aggregation, c.group_by.len(), &*c.description), (Aggregation::Count, 0, "Generated from all logs"));
    }

    #[test]
    fn logs_are_bucketed_by_their_date() {
        let m = org(vec![Definition::new("logs.all", "", Aggregation::Count, &[])]);
        let mut agg = Aggregator::default();
        let l = log("billing", "info", &[]);
        let bucket = NOW.div_euclid(BUCKET_MS) * BUCKET_MS;
        for ts in [bucket, bucket + 9_999, bucket - 1, NOW - MAX_LATENESS_MS - 1, NOW + 60_000] {
            agg.record("acme", &m, &fields(&l), ts, NOW);
        }
        let mut points = agg.drain().0;
        points.sort_by_key(|p| p.time_ms);
        let got: Vec<_> = points.iter().map(|p| (p.time_ms, p.value.clone())).collect();
        // Too old: not counted; dated in the future: counted now.
        assert_eq!(got, vec![(bucket - BUCKET_MS, PointValue::Count(1)), (bucket, PointValue::Count(3))]);
    }

    #[test]
    fn distributions_of_a_numeric_attribute() {
        let m = org(vec![Definition::new("logs.checkout.latency", "service:order-service", Aggregation::Distribution { path: "@duration".into() }, &group(&["status"]))]);
        let mut agg = Aggregator::default();
        for d in ["120", "80", "0", "-4", "not a number"] {
            agg.record("acme", &m, &fields(&log("order-service", "info", &[("duration", d)])), NOW, NOW);
        }
        // No value: not observed.
        agg.record("acme", &m, &fields(&log("order-service", "info", &[])), NOW, NOW);
        let points = agg.drain().0;
        assert_eq!(points.len(), 1);
        let PointValue::Distribution(h) = &points[0].value else { panic!("a distribution") };
        assert_eq!((h.count, h.sum, h.min, h.max, h.zero_count), (4, 196.0, -4.0, 120.0, 1));
        assert_eq!(h.positive, BTreeMap::from([(bucket_index(80.0), 1), (bucket_index(120.0), 1)]));
        assert_eq!(h.negative, BTreeMap::from([(bucket_index(4.0), 1)]));
        assert_eq!(points[0].tags, vec![("status".to_string(), "info".to_string())]);
    }

    #[test]
    fn bucket_indexes_bound_their_values() {
        let base = 2f64.powf(2f64.powi(-SCALE));
        for v in [1e-6, 0.5, 1.0, 1.5, 80.0, 123_456.0] {
            let i = bucket_index(v);
            assert!(base.powi(i) < v * (1.0 + 1e-12) && v <= base.powi(i + 1) * (1.0 + 1e-12), "{v} in bucket {i}");
        }
        assert_eq!(bucket_index(1.0), -1, "1 is the upper bound of bucket -1");
    }

    #[test]
    fn series_per_metric_are_capped() {
        let m = org(vec![Definition::new("logs.by_user", "", Aggregation::Count, &group(&["@usr.id"])), Definition::new("logs.all", "", Aggregation::Count, &[])]);
        let mut agg = Aggregator::default();
        for i in 0..MAX_SERIES_PER_METRIC + 5 {
            let id = i.to_string();
            agg.record("acme", &m, &fields(&log("storefront-api", "info", &[("usr.id", &id)])), NOW, NOW);
        }
        // A series already seen still counts.
        agg.record("acme", &m, &fields(&log("storefront-api", "info", &[("usr.id", "0")])), NOW, NOW);
        let (points, dropped) = agg.drain();
        let by_user: Vec<_> = points.iter().filter(|p| p.metric == "logs.by_user").collect();
        assert_eq!(by_user.len(), MAX_SERIES_PER_METRIC);
        assert_eq!(dropped, vec![(("acme".to_string(), "logs.by_user".to_string()), 5)]);
        assert_eq!(counts(&points).iter().find(|c| c.0 == "logs.all").map(|c| c.2), Some(MAX_SERIES_PER_METRIC as u64 + 6));
        // The cap is per flush.
        agg.record("acme", &m, &fields(&log("storefront-api", "info", &[("usr.id", "new")])), NOW, NOW);
        assert_eq!(agg.drain().0.iter().filter(|p| p.metric == "logs.by_user").count(), 1);
    }

    fn parse(ndjson: &[u8]) -> Vec<Value> {
        ndjson.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| serde_json::from_slice(l).unwrap()).collect()
    }

    #[test]
    fn rows_are_shaped_like_the_collectors() {
        let retention = Retention { raw: 15, rollup_5m: 90, rollup_1h: 395 };
        let tags = vec![("service".to_string(), "order-service".to_string()), ("env".to_string(), "prod".to_string()), ("http.status_code".to_string(), "502".to_string())];
        let mut h = Histogram::default();
        for v in [80.0, 120.0] {
            h.add(v);
        }
        let points = [
            Point {
                org: "acme".into(),
                metric: "logs.checkout.errors".into(),
                description: "Generated from logs matching: status:error".into(),
                retention,
                tags: tags.clone(),
                time_ms: 1_792_933_200_000,
                value: PointValue::Count(3),
            },
            Point {
                org: "acme".into(),
                metric: "logs.checkout.latency".into(),
                description: "d".into(),
                retention: Retention::default(),
                tags: vec![],
                time_ms: 1_792_933_200_000,
                value: PointValue::Distribution(h),
            },
        ];
        let (sums, hists) = rows(&points);
        let sums = parse(&sums);
        assert_eq!(sums.len(), 1);
        assert_eq!(
            sums[0],
            json!({
                "ResourceAttributes": {"oc.org_id": "acme", "oc.metrics_retention_days": "15", "oc.metrics_retention_5m_days": "90", "oc.metrics_retention_1h_days": "395",
                                       "service.name": "order-service", "deployment.environment": "prod"},
                "ServiceName": "order-service",
                "ScopeName": "observecraft.log_metrics",
                "MetricName": "logs.checkout.errors",
                "MetricDescription": "Generated from logs matching: status:error",
                "MetricUnit": "",
                "Attributes": {"http.status_code": "502"},
                "StartTimeUnix": "2026-10-25T13:00:00.000Z",
                "TimeUnix": "2026-10-25T13:00:00.000Z",
                "Value": 3.0,
                "AggregationTemporality": 1,
                "IsMonotonic": true
            })
        );
        let hists = parse(&hists);
        assert_eq!(hists.len(), 1);
        let r = &hists[0];
        assert_eq!(r["ResourceAttributes"], json!({"oc.org_id": "acme", "oc.metrics_retention_days": "0", "oc.metrics_retention_5m_days": "0", "oc.metrics_retention_1h_days": "0"}));
        assert_eq!((r["ServiceName"].as_str(), r["MetricName"].as_str()), (Some(""), Some("logs.checkout.latency")));
        assert_eq!((r["Count"].as_u64(), r["Sum"].as_f64(), r["Min"].as_f64(), r["Max"].as_f64()), (Some(2), Some(200.0), Some(80.0), Some(120.0)));
        assert_eq!((r["Scale"].as_i64(), r["ZeroCount"].as_u64(), r["AggregationTemporality"].as_i64()), (Some(SCALE as i64), Some(0), Some(1)));
        let (lo, hi) = (bucket_index(80.0), bucket_index(120.0));
        assert_eq!(r["PositiveOffset"].as_i64(), Some(lo as i64));
        let counts: Vec<u64> = serde_json::from_value(r["PositiveBucketCounts"].clone()).unwrap();
        assert_eq!(counts.len() as i32, hi - lo + 1);
        assert_eq!((counts[0], counts[counts.len() - 1], counts.iter().sum::<u64>()), (1, 1, 2));
        assert_eq!((r["NegativeOffset"].as_i64(), r["NegativeBucketCounts"].as_array().map(Vec::len)), (Some(0), Some(0)));
    }
}
