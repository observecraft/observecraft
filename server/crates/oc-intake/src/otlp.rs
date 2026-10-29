//! The OTLP protobuf messages the Datadog conversions build (opentelemetry-proto, the fields we
//! use). Converted payloads are sent to the collector as protobuf: compact, and without an
//! intermediate JSON tree (which multiplied a payload's size several hundred times in memory).

use prost::Message;

#[derive(Clone, PartialEq, prost::Message)]
pub struct AnyValue {
    #[prost(oneof = "any_value::Value", tags = "1, 2, 3, 4")]
    pub value: Option<any_value::Value>,
}
pub mod any_value {
    // Variant names follow the OTLP proto.
    #[allow(clippy::enum_variant_names)]
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Value {
        #[prost(string, tag = "1")]
        StringValue(String),
        #[prost(bool, tag = "2")]
        BoolValue(bool),
        #[prost(int64, tag = "3")]
        IntValue(i64),
        #[prost(double, tag = "4")]
        DoubleValue(f64),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct KeyValue {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(message, optional, tag = "2")]
    pub value: Option<AnyValue>,
}

/// A string attribute.
pub fn kv(key: impl Into<String>, value: impl Into<String>) -> KeyValue {
    KeyValue { key: key.into(), value: Some(AnyValue { value: Some(any_value::Value::StringValue(value.into())) }) }
}

/// A double attribute.
pub fn kv_f64(key: impl Into<String>, value: f64) -> KeyValue {
    KeyValue { key: key.into(), value: Some(AnyValue { value: Some(any_value::Value::DoubleValue(value)) }) }
}

/// The string value of an attribute ("" for other types).
#[cfg(test)]
pub fn text(kv: &KeyValue) -> &str {
    match kv.value.as_ref().and_then(|v| v.value.as_ref()) {
        Some(any_value::Value::StringValue(s)) => s,
        _ => "",
    }
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct InstrumentationScope {
    #[prost(string, tag = "1")]
    pub name: String,
}

pub fn scope(name: &str) -> Option<InstrumentationScope> {
    Some(InstrumentationScope { name: name.into() })
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Resource {
    #[prost(message, repeated, tag = "1")]
    pub attributes: Vec<KeyValue>,
}

// ---------------------------------------------------------------- metrics

#[derive(Clone, PartialEq, prost::Message)]
pub struct ExportMetricsServiceRequest {
    #[prost(message, repeated, tag = "1")]
    pub resource_metrics: Vec<ResourceMetrics>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ResourceMetrics {
    #[prost(message, optional, tag = "1")]
    pub resource: Option<Resource>,
    #[prost(message, repeated, tag = "2")]
    pub scope_metrics: Vec<ScopeMetrics>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ScopeMetrics {
    #[prost(message, optional, tag = "1")]
    pub scope: Option<InstrumentationScope>,
    #[prost(message, repeated, tag = "2")]
    pub metrics: Vec<Metric>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Metric {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub unit: String,
    #[prost(oneof = "metric::Data", tags = "5, 7, 10")]
    pub data: Option<metric::Data>,
}
pub mod metric {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Data {
        #[prost(message, tag = "5")]
        Gauge(super::Gauge),
        #[prost(message, tag = "7")]
        Sum(super::Sum),
        #[prost(message, tag = "10")]
        ExponentialHistogram(super::ExponentialHistogram),
    }
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Gauge {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<NumberDataPoint>,
}
/// Aggregation temporality: delta.
pub const DELTA: i32 = 1;
#[derive(Clone, PartialEq, prost::Message)]
pub struct Sum {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<NumberDataPoint>,
    #[prost(int32, tag = "2")]
    pub aggregation_temporality: i32,
    #[prost(bool, tag = "3")]
    pub is_monotonic: bool,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct NumberDataPoint {
    #[prost(fixed64, tag = "2")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "3")]
    pub time_unix_nano: u64,
    #[prost(double, tag = "4")]
    pub as_double: f64,
    #[prost(message, repeated, tag = "7")]
    pub attributes: Vec<KeyValue>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExponentialHistogram {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<ExponentialHistogramDataPoint>,
    #[prost(int32, tag = "2")]
    pub aggregation_temporality: i32,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExponentialHistogramDataPoint {
    #[prost(message, repeated, tag = "1")]
    pub attributes: Vec<KeyValue>,
    #[prost(fixed64, tag = "2")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "3")]
    pub time_unix_nano: u64,
    #[prost(fixed64, tag = "4")]
    pub count: u64,
    #[prost(double, optional, tag = "5")]
    pub sum: Option<f64>,
    #[prost(sint32, tag = "6")]
    pub scale: i32,
    #[prost(fixed64, tag = "7")]
    pub zero_count: u64,
    #[prost(message, optional, tag = "8")]
    pub positive: Option<Buckets>,
    #[prost(message, optional, tag = "9")]
    pub negative: Option<Buckets>,
    #[prost(double, optional, tag = "12")]
    pub min: Option<f64>,
    #[prost(double, optional, tag = "13")]
    pub max: Option<f64>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Buckets {
    #[prost(sint32, tag = "1")]
    pub offset: i32,
    #[prost(uint64, repeated, tag = "2")]
    pub bucket_counts: Vec<u64>,
}

// ---------------------------------------------------------------- traces

#[derive(Clone, PartialEq, prost::Message)]
pub struct ExportTraceServiceRequest {
    #[prost(message, repeated, tag = "1")]
    pub resource_spans: Vec<ResourceSpans>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ResourceSpans {
    #[prost(message, optional, tag = "1")]
    pub resource: Option<Resource>,
    #[prost(message, repeated, tag = "2")]
    pub scope_spans: Vec<ScopeSpans>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ScopeSpans {
    #[prost(message, optional, tag = "1")]
    pub scope: Option<InstrumentationScope>,
    #[prost(message, repeated, tag = "2")]
    pub spans: Vec<Span>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Span {
    #[prost(bytes = "vec", tag = "1")]
    pub trace_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub span_id: Vec<u8>,
    #[prost(string, tag = "3")]
    pub trace_state: String,
    #[prost(bytes = "vec", tag = "4")]
    pub parent_span_id: Vec<u8>,
    #[prost(string, tag = "5")]
    pub name: String,
    #[prost(int32, tag = "6")]
    pub kind: i32,
    #[prost(fixed64, tag = "7")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "8")]
    pub end_time_unix_nano: u64,
    #[prost(message, repeated, tag = "9")]
    pub attributes: Vec<KeyValue>,
    #[prost(message, repeated, tag = "11")]
    pub events: Vec<SpanEvent>,
    #[prost(message, repeated, tag = "13")]
    pub links: Vec<SpanLink>,
    #[prost(message, optional, tag = "15")]
    pub status: Option<Status>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct SpanEvent {
    #[prost(fixed64, tag = "1")]
    pub time_unix_nano: u64,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(message, repeated, tag = "3")]
    pub attributes: Vec<KeyValue>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct SpanLink {
    #[prost(bytes = "vec", tag = "1")]
    pub trace_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub span_id: Vec<u8>,
    #[prost(string, tag = "3")]
    pub trace_state: String,
    #[prost(message, repeated, tag = "4")]
    pub attributes: Vec<KeyValue>,
}
/// Status code: error.
pub const STATUS_ERROR: i32 = 2;
#[derive(Clone, PartialEq, prost::Message)]
pub struct Status {
    #[prost(string, tag = "2")]
    pub message: String,
    #[prost(int32, tag = "3")]
    pub code: i32,
}

// ---------------------------------------------------------------- logs

#[derive(Clone, PartialEq, prost::Message)]
pub struct ExportLogsServiceRequest {
    #[prost(message, repeated, tag = "1")]
    pub resource_logs: Vec<ResourceLogs>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ResourceLogs {
    #[prost(message, optional, tag = "1")]
    pub resource: Option<Resource>,
    #[prost(message, repeated, tag = "2")]
    pub scope_logs: Vec<ScopeLogs>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ScopeLogs {
    #[prost(message, optional, tag = "1")]
    pub scope: Option<InstrumentationScope>,
    #[prost(message, repeated, tag = "2")]
    pub log_records: Vec<LogRecord>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct LogRecord {
    #[prost(fixed64, tag = "1")]
    pub time_unix_nano: u64,
    #[prost(int32, tag = "2")]
    pub severity_number: i32,
    #[prost(string, tag = "3")]
    pub severity_text: String,
    #[prost(message, optional, tag = "5")]
    pub body: Option<AnyValue>,
    #[prost(message, repeated, tag = "6")]
    pub attributes: Vec<KeyValue>,
    #[prost(fixed64, tag = "11")]
    pub observed_time_unix_nano: u64,
}

/// Encodes any OTLP request for the collector's OTLP/HTTP protobuf endpoint.
pub fn encode(m: &impl Message) -> bytes::Bytes {
    bytes::Bytes::from(m.encode_to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{Hash, Hasher};
    use std::time::{Duration, Instant};

    const POINTS: usize = 100_000;
    const LABELS: usize = 20;
    const METRICS_PER_RESOURCE: usize = 25;

    /// 100k gauge points with ~20 labels: on the points (as the Datadog conversion builds them) or
    /// on one resource per 25 points (a Kubernetes pod, as OpenTelemetry collectors send them).
    fn request(labels_on_points: bool) -> ExportMetricsServiceRequest {
        let labels = |pod: usize| -> Vec<KeyValue> {
            (0..LABELS).map(|l| kv(format!("kube_label_key_{l}"), if l == 0 { format!("order-service-7d4f9c-{pod:05}") } else { format!("value-{}-{}", l, pod % 40) })).collect()
        };
        let resource_metrics = (0..POINTS / METRICS_PER_RESOURCE)
            .map(|pod| {
                let (resource_attrs, point_attrs) = if labels_on_points { (vec![kv("host.name", format!("i-0a1b2c3d4e5f{pod:05x}"))], labels(pod)) } else { (labels(pod), Vec::new()) };
                let metrics = (0..METRICS_PER_RESOURCE)
                    .map(|m| Metric {
                        name: format!("kubernetes.metric_{m}"),
                        unit: String::new(),
                        data: Some(metric::Data::Gauge(Gauge {
                            data_points: vec![NumberDataPoint { start_time_unix_nano: 0, time_unix_nano: 1_800_000_000_000_000_000, as_double: (pod * m) as f64, attributes: point_attrs.clone() }],
                        })),
                    })
                    .collect();
                ResourceMetrics { resource: Some(Resource { attributes: resource_attrs }), scope_metrics: vec![ScopeMetrics { scope: scope("datadog"), metrics }] }
            })
            .collect();
        ExportMetricsServiceRequest { resource_metrics }
    }

    /// The series identity of every point (metric name, type and sorted resource + point labels),
    /// as an intake cache of known series would compute it (docs/adr/0002, stage 2).
    fn series_hashes(req: &ExportMetricsServiceRequest) -> Vec<u64> {
        let mut out = Vec::new();
        for rm in &req.resource_metrics {
            let resource = rm.resource.as_ref().map(|r| r.attributes.as_slice()).unwrap_or_default();
            for sm in &rm.scope_metrics {
                for m in &sm.metrics {
                    let Some(metric::Data::Gauge(g)) = &m.data else { continue };
                    for p in &g.data_points {
                        let mut labels: Vec<(&str, &str)> = resource.iter().chain(&p.attributes).map(|a| (a.key.as_str(), text(a))).collect();
                        labels.sort_unstable();
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        (m.name.as_str(), "gauge", labels).hash(&mut h);
                        out.push(h.finish());
                    }
                }
            }
        }
        out
    }

    fn best_of<T>(runs: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
        let mut best = Duration::MAX;
        let mut last = None;
        for _ in 0..runs {
            let start = Instant::now();
            let out = f();
            best = best.min(start.elapsed());
            last = Some(out);
        }
        (best, last.expect("at least one run"))
    }

    /// What marking OTLP series in the intake would cost: decoding the body, hashing each point's
    /// series and re-encoding it. One thread, best of 5 runs:
    /// `cargo test --release -p oc-intake otlp_metrics_decode_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing benchmark, run by hand in release mode"]
    fn otlp_metrics_decode_cost() {
        for labels_on_points in [true, false] {
            let body = encode(&request(labels_on_points));
            let (decode, req) = best_of(5, || ExportMetricsServiceRequest::decode(body.as_ref()).expect("valid OTLP"));
            let (hash, hashes) = best_of(5, || series_hashes(&req));
            let (reencode, out) = best_of(5, || encode(&req));
            assert_eq!(hashes.len(), POINTS);
            assert_eq!(out, body, "re-encoding these fields is lossless");
            let ms = |d: Duration| d.as_secs_f64() * 1e3;
            println!(
                "labels on {}: body {:.1} MiB; per 100k points: decode {:.1} ms, series hashes {:.1} ms, re-encode {:.1} ms, total {:.1} ms",
                if labels_on_points { "points" } else { "resources" },
                body.len() as f64 / 1_048_576.0,
                ms(decode),
                ms(hash),
                ms(reencode),
                ms(decode + hash + reencode)
            );
        }
    }
}
