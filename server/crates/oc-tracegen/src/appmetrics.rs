//! Application metrics the synthetic services would export through an OTel SDK, derived from their spans:
//! - `http.server.request.duration` (s): explicit-bucket histogram, cumulative, per service + route
//! - `messaging.process.duration` (s): exponential histogram, delta, per consumer service
//! - `runtime.gc.pause` (ms): summary (as from a Prometheus client), cumulative, per service
//!
//! They exercise every distribution type the query engine supports (`<name>.p95`, `.avg`, `.count`…).

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::{attrs_json, Rng};

/// OTel SDK default bucket bounds for `http.server.request.duration`.
const BOUNDS: [f64; 14] = [0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0];
/// Quantiles the GC pause summary reports (0 and 1 are min and max).
const QUANTILES: [f64; 6] = [0.0, 0.5, 0.9, 0.95, 0.99, 1.0];
/// Exponential histogram scale: bucket bases grow by 2^(1/8).
const SCALE: i32 = 3;

#[derive(Default)]
struct Hist {
    counts: Vec<u64>,
    count: u64,
    sum: f64,
    min: f64,
    max: f64,
}

#[derive(Default)]
struct Summary {
    count: u64,
    sum: f64,
    window: Vec<f64>,
}

pub struct AppMetrics {
    start_ns: u64,
    /// (service, host, route) → cumulative histogram.
    http: BTreeMap<(&'static str, &'static str, String), Hist>,
    /// (service, host) → values observed since the last export.
    messaging: BTreeMap<(&'static str, &'static str), Vec<f64>>,
    gc: BTreeMap<(&'static str, &'static str), Summary>,
}

impl AppMetrics {
    pub fn new(start_ns: u64) -> Self {
        AppMetrics { start_ns, http: BTreeMap::new(), messaging: BTreeMap::new(), gc: BTreeMap::new() }
    }

    pub fn observe_http(&mut self, service: &'static str, host: &'static str, route: &str, secs: f64) {
        let h = self.http.entry((service, host, route.to_string())).or_insert_with(|| Hist { counts: vec![0; BOUNDS.len() + 1], min: f64::INFINITY, ..Default::default() });
        let i = BOUNDS.iter().position(|b| secs <= *b).unwrap_or(BOUNDS.len());
        h.counts[i] += 1;
        h.count += 1;
        h.sum += secs;
        h.min = h.min.min(secs);
        h.max = h.max.max(secs);
    }

    pub fn observe_messaging(&mut self, service: &'static str, host: &'static str, secs: f64) {
        self.messaging.entry((service, host)).or_default().push(secs);
    }

    /// GC pauses: a few per interval per service.
    fn simulate_gc(&mut self, rng: &mut Rng) {
        let keys: Vec<_> = self.http.keys().map(|(s, h, _)| (*s, *h)).collect();
        for key in keys {
            let s = self.gc.entry(key).or_default();
            for _ in 0..rng.range(2, 8) {
                let pause = 0.5 + rng.f() * rng.f() * 40.0;
                s.count += 1;
                s.sum += pause;
                s.window.push(pause);
            }
            let excess = s.window.len().saturating_sub(500);
            s.window.drain(..excess);
        }
    }

    pub fn payload(&mut self, rng: &mut Rng, now_ns: u64) -> Value {
        self.simulate_gc(rng);
        let t = now_ns.to_string();
        let start = self.start_ns.to_string();
        let mut by_service: BTreeMap<(&'static str, &'static str), Vec<Value>> = BTreeMap::new();

        for ((svc, host, route), h) in &self.http {
            by_service.entry((svc, host)).or_default().push(json!({
                "name": "http.server.request.duration", "unit": "s", "description": "Duration of HTTP server requests",
                "histogram": { "aggregationTemporality": 2, "dataPoints": [{
                    "startTimeUnixNano": start, "timeUnixNano": t,
                    "count": h.count.to_string(), "sum": h.sum, "min": h.min, "max": h.max,
                    "bucketCounts": h.counts.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
                    "explicitBounds": BOUNDS,
                    "attributes": attrs_json(&[("http.route".into(), json!(route)), ("http.request.method".into(), json!(route.split(' ').next().unwrap_or("GET")))]),
                }]},
            }));
        }

        let base = 2f64.powf(2f64.powi(-SCALE));
        for ((svc, host), values) in std::mem::take(&mut self.messaging) {
            if values.is_empty() {
                continue;
            }
            // Bucket index i covers (base^i, base^(i+1)].
            let idx: Vec<i32> = values.iter().map(|v| (v.max(1e-9).ln() / base.ln()).ceil() as i32 - 1).collect();
            let offset = *idx.iter().min().unwrap();
            let mut counts = vec![0u64; (*idx.iter().max().unwrap() - offset + 1) as usize];
            for i in &idx {
                counts[(i - offset) as usize] += 1;
            }
            let sum: f64 = values.iter().sum();
            by_service.entry((svc, host)).or_default().push(json!({
                "name": "messaging.process.duration", "unit": "s", "description": "Duration of processing a message",
                "exponentialHistogram": { "aggregationTemporality": 1, "dataPoints": [{
                    "startTimeUnixNano": t, "timeUnixNano": t,
                    "count": values.len().to_string(), "sum": sum,
                    "min": values.iter().copied().fold(f64::INFINITY, f64::min), "max": values.iter().copied().fold(0.0, f64::max),
                    "scale": SCALE, "zeroCount": "0",
                    "positive": { "offset": offset, "bucketCounts": counts.iter().map(|c| c.to_string()).collect::<Vec<_>>() },
                }]},
            }));
        }

        for ((svc, host), s) in &self.gc {
            let mut sorted = s.window.clone();
            sorted.sort_by(f64::total_cmp);
            let at = |q: f64| sorted.get(((sorted.len() - 1) as f64 * q).round() as usize).copied().unwrap_or(0.0);
            by_service.entry((svc, host)).or_default().push(json!({
                "name": "runtime.gc.pause", "unit": "ms", "description": "Garbage collection pause time",
                "summary": { "dataPoints": [{
                    "startTimeUnixNano": start, "timeUnixNano": t,
                    "count": s.count.to_string(), "sum": s.sum,
                    "quantileValues": QUANTILES.iter().map(|q| json!({ "quantile": q, "value": at(*q) })).collect::<Vec<_>>(),
                }]},
            }));
        }

        let resource_metrics: Vec<Value> = by_service
            .into_iter()
            .map(|((svc, host), metrics)| {
                json!({
                    "resource": { "attributes": attrs_json(&[("service.name".into(), json!(svc)), ("host.name".into(), json!(host))]) },
                    "scopeMetrics": [{ "scope": { "name": "oc-tracegen" }, "metrics": metrics }],
                })
            })
            .collect();
        json!({ "resourceMetrics": resource_metrics })
    }
}
