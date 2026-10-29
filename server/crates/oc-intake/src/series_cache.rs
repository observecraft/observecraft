//! Which series this intake replica already forwarded in an hour (docs/adr/0002-storage-at-saas-scale.md).
//!
//! ClickHouse writes a series row (both label maps) for every point it gets, unless the point's
//! request is marked with `x-oc-series-known: 1`, which the Collector turns into the resource
//! attribute `oc.series_known = 1`. The intake splits the metrics it converts itself (Datadog
//! series, sketches, service checks, processes and containers) into points of series it hasn't
//! forwarded yet in the point's UTC hour, sent as before, and points of known series, sent marked.
//! So the series table gets about one row per series and hour, and its `LastSeen` is a time inside
//! every hour the series reported (readers test "seen since t" as `LastSeen >= toStartOfHour(t)`).
//!
//! The cache is bounded: when full it forgets the oldest entries. Forgetting, a restart or another
//! replica only means an extra series row, which the series table's ReplacingMergeTree absorbs.
//! Entries are added only once their unmarked request was delivered ([`SeriesCache::commit`]), so
//! a failed delivery never leaves a series without its row.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use crate::otlp::{self, any_value, ExportMetricsServiceRequest, KeyValue};

/// Header marking a request whose points all belong to series forwarded earlier in their hour.
pub const SERIES_KNOWN_HEADER: &str = "x-oc-series-known";

const HOUR_NANOS: u64 = 3_600_000_000_000;

/// A forwarded request split by series novelty.
pub struct Split {
    /// Points of series not yet forwarded in their hour (sent as before).
    pub new: Option<ExportMetricsServiceRequest>,
    /// Points of series already forwarded in their hour (sent with [`SERIES_KNOWN_HEADER`]).
    pub known: Option<ExportMetricsServiceRequest>,
    /// Cache entries to add once `new` was delivered.
    pub pending: Vec<u64>,
}

pub struct SeriesCache {
    capacity: usize,
    inner: Mutex<Entries>,
}

#[derive(Default)]
struct Entries {
    set: HashSet<u64>,
    /// Insertion order, oldest first.
    order: VecDeque<u64>,
}

impl SeriesCache {
    /// A cache of at most `capacity` (series, hour) entries; 0 turns marking off.
    pub fn new(capacity: usize) -> Self {
        SeriesCache { capacity, inner: Mutex::default() }
    }

    /// Splits `req` (metrics of `org`) into points of new and of known series.
    pub fn split(&self, org: &str, req: ExportMetricsServiceRequest) -> Split {
        if self.capacity == 0 {
            return Split { new: Some(req).filter(|r| !r.resource_metrics.is_empty()), known: None, pending: Vec::new() };
        }
        let mut pending: HashSet<u64> = HashSet::new();
        let (mut new, mut known) = (Vec::new(), Vec::new());
        let entries = self.inner.lock().unwrap();
        for rm in req.resource_metrics {
            let resource = resource_hash(org, rm.resource.as_ref());
            let (mut new_scopes, mut known_scopes) = (Vec::new(), Vec::new());
            for sm in rm.scope_metrics {
                let (mut new_metrics, mut known_metrics) = (Vec::new(), Vec::new());
                for m in sm.metrics {
                    let (fresh, seen) = split_metric(m, resource, |key| {
                        if pending.contains(&key) || !entries.set.contains(&key) {
                            pending.insert(key);
                            true
                        } else {
                            false
                        }
                    });
                    new_metrics.extend(fresh);
                    known_metrics.extend(seen);
                }
                if !new_metrics.is_empty() {
                    new_scopes.push(otlp::ScopeMetrics { scope: sm.scope.clone(), metrics: new_metrics });
                }
                if !known_metrics.is_empty() {
                    known_scopes.push(otlp::ScopeMetrics { scope: sm.scope, metrics: known_metrics });
                }
            }
            if !new_scopes.is_empty() {
                new.push(otlp::ResourceMetrics { resource: rm.resource.clone(), scope_metrics: new_scopes });
            }
            if !known_scopes.is_empty() {
                known.push(otlp::ResourceMetrics { resource: rm.resource, scope_metrics: known_scopes });
            }
        }
        drop(entries);
        let request = |resource_metrics: Vec<otlp::ResourceMetrics>| Some(ExportMetricsServiceRequest { resource_metrics }).filter(|r| !r.resource_metrics.is_empty());
        Split { new: request(new), known: request(known), pending: pending.into_iter().collect() }
    }

    /// Remembers the series of a delivered `new` request.
    pub fn commit(&self, pending: &[u64]) {
        if self.capacity == 0 {
            return;
        }
        let mut entries = self.inner.lock().unwrap();
        for &key in pending {
            if !entries.set.insert(key) {
                continue;
            }
            entries.order.push_back(key);
            while entries.order.len() > self.capacity {
                if let Some(oldest) = entries.order.pop_front() {
                    entries.set.remove(&oldest);
                }
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.lock().unwrap().set.len()
    }
}

fn hash_value(v: &Option<otlp::AnyValue>, h: &mut DefaultHasher) {
    match v.as_ref().and_then(|v| v.value.as_ref()) {
        Some(any_value::Value::StringValue(s)) => (0u8, s).hash(h),
        Some(any_value::Value::BoolValue(b)) => (1u8, b).hash(h),
        Some(any_value::Value::IntValue(i)) => (2u8, i).hash(h),
        Some(any_value::Value::DoubleValue(d)) => (3u8, d.to_bits()).hash(h),
        None => 4u8.hash(h),
    }
}

/// Hashes attributes in key order, skipping the org's own `oc.*` stamps (not labels, as in ClickHouse).
fn hash_attributes(attributes: &[KeyValue], h: &mut DefaultHasher) {
    let mut sorted: Vec<&KeyValue> = attributes.iter().filter(|a| !a.key.starts_with("oc.")).collect();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));
    sorted.len().hash(h);
    for a in sorted {
        a.key.hash(h);
        hash_value(&a.value, h);
    }
}

fn resource_hash(org: &str, resource: Option<&otlp::Resource>) -> u64 {
    let mut h = DefaultHasher::new();
    org.hash(&mut h);
    hash_attributes(resource.map_or(&[][..], |r| &r.attributes), &mut h);
    h.finish()
}

/// The cache key of one point: its series (org, resource, metric name and type, attributes) and hour.
fn point_key(resource: u64, name: &str, kind: u8, attributes: &[KeyValue], time_unix_nano: u64) -> u64 {
    let mut h = DefaultHasher::new();
    (resource, name, kind).hash(&mut h);
    hash_attributes(attributes, &mut h);
    (time_unix_nano / HOUR_NANOS).hash(&mut h);
    h.finish()
}

/// Splits a metric's points by `is_new(key)`: (the metric with new points, with known points).
fn split_metric(m: otlp::Metric, resource: u64, mut is_new: impl FnMut(u64) -> bool) -> (Option<otlp::Metric>, Option<otlp::Metric>) {
    use otlp::metric::Data;
    fn part<P>(points: Vec<P>, mut is_new: impl FnMut(&P) -> bool) -> (Vec<P>, Vec<P>) {
        points.into_iter().partition(|p| is_new(p))
    }
    let name = m.name.clone();
    let with = |data: Data| otlp::Metric { name: m.name.clone(), unit: m.unit.clone(), data: Some(data) };
    let (fresh, seen) = match m.data {
        Some(Data::Gauge(g)) => {
            let (a, b) = part(g.data_points, |p| is_new(point_key(resource, &name, 1, &p.attributes, p.time_unix_nano)));
            ((!a.is_empty()).then(|| with(Data::Gauge(otlp::Gauge { data_points: a }))), (!b.is_empty()).then(|| with(Data::Gauge(otlp::Gauge { data_points: b }))))
        }
        Some(Data::Sum(s)) => {
            let (a, b) = part(s.data_points, |p| is_new(point_key(resource, &name, 2, &p.attributes, p.time_unix_nano)));
            let sum = |data_points| Data::Sum(otlp::Sum { data_points, aggregation_temporality: s.aggregation_temporality, is_monotonic: s.is_monotonic });
            ((!a.is_empty()).then(|| with(sum(a))), (!b.is_empty()).then(|| with(sum(b))))
        }
        Some(Data::ExponentialHistogram(e)) => {
            let (a, b) = part(e.data_points, |p| is_new(point_key(resource, &name, 4, &p.attributes, p.time_unix_nano)));
            let hist = |data_points| Data::ExponentialHistogram(otlp::ExponentialHistogram { data_points, aggregation_temporality: e.aggregation_temporality });
            ((!a.is_empty()).then(|| with(hist(a))), (!b.is_empty()).then(|| with(hist(b))))
        }
        None => (None, None),
    };
    (fresh, seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otlp::{kv, metric, Gauge, Metric, NumberDataPoint, Resource, ResourceMetrics, ScopeMetrics, Sum};

    const T: u64 = 1_793_095_200 * 1_000_000_000; // 2026-10-27 10:00:00 UTC

    fn point(t: u64, attrs: &[(&str, &str)]) -> NumberDataPoint {
        NumberDataPoint { start_time_unix_nano: 0, time_unix_nano: t, as_double: 1.0, attributes: attrs.iter().map(|(k, v)| kv(*k, *v)).collect() }
    }

    fn gauge(name: &str, points: Vec<NumberDataPoint>) -> Metric {
        Metric { name: name.into(), unit: String::new(), data: Some(metric::Data::Gauge(Gauge { data_points: points })) }
    }

    fn request(resource: &[(&str, &str)], metrics: Vec<Metric>) -> ExportMetricsServiceRequest {
        ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource { attributes: resource.iter().map(|(k, v)| kv(*k, *v)).collect() }),
                scope_metrics: vec![ScopeMetrics { scope: otlp::scope("datadog"), metrics }],
            }],
        }
    }

    /// Number of data points in a request.
    fn points(req: &Option<ExportMetricsServiceRequest>) -> usize {
        req.iter()
            .flat_map(|r| &r.resource_metrics)
            .flat_map(|r| &r.scope_metrics)
            .flat_map(|s| &s.metrics)
            .map(|m| match &m.data {
                Some(metric::Data::Gauge(g)) => g.data_points.len(),
                Some(metric::Data::Sum(s)) => s.data_points.len(),
                Some(metric::Data::ExponentialHistogram(h)) => h.data_points.len(),
                None => 0,
            })
            .sum()
    }

    /// Splits and delivers (commits) like the intake does; returns (new points, known points).
    fn send(cache: &SeriesCache, org: &str, req: ExportMetricsServiceRequest) -> (usize, usize) {
        let s = cache.split(org, req);
        cache.commit(&s.pending);
        (points(&s.new), points(&s.known))
    }

    fn pod(name: &str) -> ExportMetricsServiceRequest {
        request(&[("host.name", "i-0a1b2c3d4e5f60001"), ("k8s.pod.name", name)], vec![gauge("kubernetes.cpu.usage.total", vec![point(T + 10_000_000_000, &[("container", "app")])])])
    }

    #[test]
    fn a_series_seen_in_its_hour_is_marked_known() {
        let cache = SeriesCache::new(100);
        assert_eq!(send(&cache, "acme", pod("storefront-1")), (1, 0));
        assert_eq!(send(&cache, "acme", pod("storefront-1")), (0, 1));
        // Another pod is another series.
        assert_eq!(send(&cache, "acme", pod("storefront-2")), (1, 0));
    }

    #[test]
    fn label_order_and_org_stamps_dont_change_a_series() {
        let cache = SeriesCache::new(100);
        let first = request(&[("host.name", "i-0a1b2c3d4e5f60001"), ("k8s.pod.name", "storefront-1")], vec![gauge("m", vec![point(T, &[("a", "1"), ("b", "2")])])]);
        let reordered = request(&[("oc.org_id", "acme"), ("k8s.pod.name", "storefront-1"), ("host.name", "i-0a1b2c3d4e5f60001")], vec![gauge("m", vec![point(T + 1, &[("b", "2"), ("a", "1")])])]);
        assert_eq!(send(&cache, "acme", first), (1, 0));
        assert_eq!(send(&cache, "acme", reordered), (0, 1));
    }

    #[test]
    fn a_gauge_and_a_sum_of_one_name_are_two_series() {
        let cache = SeriesCache::new(100);
        let sum = Metric { name: "m".into(), unit: String::new(), data: Some(metric::Data::Sum(Sum { data_points: vec![point(T, &[])], aggregation_temporality: otlp::DELTA, is_monotonic: true })) };
        assert_eq!(send(&cache, "acme", request(&[], vec![gauge("m", vec![point(T, &[])])])), (1, 0));
        assert_eq!(send(&cache, "acme", request(&[], vec![sum])), (1, 0));
    }

    #[test]
    fn the_next_hour_writes_the_series_again() {
        let cache = SeriesCache::new(100);
        let at = |t: u64| request(&[("host.name", "i-0a1b2c3d4e5f60001")], vec![gauge("m", vec![point(t, &[])])]);
        assert_eq!(send(&cache, "acme", at(T + 59 * 60 * 1_000_000_000)), (1, 0));
        assert_eq!(send(&cache, "acme", at(T + 3_599 * 1_000_000_000)), (0, 1));
        // 11:00: a new hour.
        assert_eq!(send(&cache, "acme", at(T + HOUR_NANOS)), (1, 0));
        // A late point of 10:xx is still known.
        assert_eq!(send(&cache, "acme", at(T + 30 * 1_000_000_000)), (0, 1));
        // One request with points in two hours: one new (11:xx), one known (10:xx).
        let both = request(&[("host.name", "i-0a1b2c3d4e5f60001")], vec![gauge("m", vec![point(T + 10, &[]), point(T + 2 * HOUR_NANOS, &[])])]);
        assert_eq!(send(&cache, "acme", both), (1, 1));
    }

    #[test]
    fn orgs_dont_share_series() {
        let cache = SeriesCache::new(100);
        assert_eq!(send(&cache, "acme", pod("storefront-1")), (1, 0));
        assert_eq!(send(&cache, "globex", pod("storefront-1")), (1, 0));
        assert_eq!(send(&cache, "globex", pod("storefront-1")), (0, 1));
    }

    #[test]
    fn the_oldest_series_are_forgotten_when_full() {
        let cache = SeriesCache::new(2);
        send(&cache, "acme", pod("p1"));
        send(&cache, "acme", pod("p2"));
        send(&cache, "acme", pod("p3"));
        assert_eq!(cache.len(), 2);
        // p1 was evicted: written again (an extra series row, never a missing one).
        assert_eq!(send(&cache, "acme", pod("p1")), (1, 0));
        assert_eq!(send(&cache, "acme", pod("p3")), (0, 1));
    }

    #[test]
    fn nothing_is_remembered_until_delivered() {
        let cache = SeriesCache::new(100);
        let s = cache.split("acme", pod("p1"));
        assert_eq!((points(&s.new), points(&s.known)), (1, 0));
        // The new request failed: not committed, so the retry is new again.
        assert_eq!(send(&cache, "acme", pod("p1")), (1, 0));
    }

    #[test]
    fn a_series_twice_in_one_request_is_new_once() {
        let cache = SeriesCache::new(100);
        let req = request(&[], vec![gauge("m", vec![point(T, &[]), point(T + 10, &[])])]);
        // Both points go with the unmarked request: its series row covers them.
        assert_eq!(send(&cache, "acme", req), (2, 0));
    }

    #[test]
    fn a_zero_capacity_cache_marks_nothing() {
        let cache = SeriesCache::new(0);
        assert_eq!(send(&cache, "acme", pod("p1")), (1, 0));
        assert_eq!(send(&cache, "acme", pod("p1")), (1, 0));
    }

    #[test]
    fn resources_and_metrics_keep_their_shape_in_both_halves() {
        let cache = SeriesCache::new(100);
        send(&cache, "acme", request(&[("host.name", "h1")], vec![gauge("a", vec![point(T, &[])])]));
        let s = cache.split("acme", request(&[("host.name", "h1")], vec![gauge("a", vec![point(T + 1, &[])]), gauge("b", vec![point(T + 1, &[])])]));
        let new = s.new.unwrap();
        let known = s.known.unwrap();
        assert_eq!(new.resource_metrics[0].scope_metrics[0].metrics.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["b"]);
        assert_eq!(known.resource_metrics[0].scope_metrics[0].metrics.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["a"]);
        assert_eq!(known.resource_metrics[0].resource, new.resource_metrics[0].resource);
    }
}
