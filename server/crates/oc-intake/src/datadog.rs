//! Datadog metrics intake: `POST /api/v2/series` (JSON, or the Datadog Agent's protobuf) and
//! `POST /api/v1/series` (JSON), plus `GET /api/v1/validate`. Series are converted to OTLP metrics
//! and sent to the collector, so they land in the same tables as every other metric:
//!   gauge (and unspecified) → OTLP gauge; rate → gauge of the per-second value;
//!   count → OTLP delta, monotonic sum.
//! The `host` resource (v2) or field (v1) becomes `host.name`, the `service:` tag `service.name`,
//! other `key:value` tags become data point attributes.

use std::collections::BTreeMap;
use std::io::Read;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::otlp::{self, KeyValue};
use crate::{admit, send, Rejection, Shared};

/// Most bytes a body may inflate to (compressed payloads are capped by the request limit).
const MAX_INFLATED: u64 = 64 * 1024 * 1024;

// ---------------------------------------------------------------- payloads

/// Datadog's metric types (v2 numbering).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Gauge,
    Rate,
    Count,
}

impl Kind {
    fn from_v2(n: i32) -> Kind {
        match n {
            1 => Kind::Count,
            2 => Kind::Rate,
            _ => Kind::Gauge,
        }
    }
    fn from_v1(s: Option<&str>) -> Kind {
        match s {
            Some("count") => Kind::Count,
            Some("rate") => Kind::Rate,
            _ => Kind::Gauge,
        }
    }
}

/// One series, whichever API version it came from.
#[derive(Debug, PartialEq)]
pub struct Series {
    pub metric: String,
    pub kind: Kind,
    pub unit: String,
    pub host: String,
    pub tags: Vec<String>,
    /// Interval in seconds (count start = timestamp − interval).
    pub interval: i64,
    /// (unix seconds, value)
    pub points: Vec<(i64, f64)>,
}

#[derive(Deserialize)]
struct V2Json {
    series: Vec<V2JsonSeries>,
}

#[derive(Deserialize)]
struct V2JsonSeries {
    metric: String,
    #[serde(default, rename = "type")]
    kind: i32,
    #[serde(default)]
    points: Vec<V2JsonPoint>,
    #[serde(default)]
    resources: Vec<V2Resource>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    unit: String,
    #[serde(default)]
    interval: i64,
}

#[derive(Deserialize)]
struct V2JsonPoint {
    timestamp: i64,
    #[serde(default)]
    value: f64,
}

#[derive(Deserialize)]
struct V2Resource {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct V1Json {
    series: Vec<V1JsonSeries>,
}

#[derive(Deserialize)]
struct V1JsonSeries {
    metric: String,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    points: Vec<(f64, Option<f64>)>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    interval: Option<i64>,
}

/// The Datadog Agent's protobuf (DataDog/agent-payload, proto/metrics/agent_payload.proto).
pub mod pb {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct MetricPayload {
        #[prost(message, repeated, tag = "1")]
        pub series: Vec<MetricSeries>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct MetricSeries {
        #[prost(message, repeated, tag = "1")]
        pub resources: Vec<Resource>,
        #[prost(string, tag = "2")]
        pub metric: String,
        #[prost(string, repeated, tag = "3")]
        pub tags: Vec<String>,
        #[prost(message, repeated, tag = "4")]
        pub points: Vec<MetricPoint>,
        #[prost(int32, tag = "5")]
        pub r#type: i32,
        #[prost(string, tag = "6")]
        pub unit: String,
        #[prost(string, tag = "7")]
        pub source_type_name: String,
        #[prost(int64, tag = "8")]
        pub interval: i64,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Resource {
        #[prost(string, tag = "1")]
        pub r#type: String,
        #[prost(string, tag = "2")]
        pub name: String,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SketchPayload {
        #[prost(message, repeated, tag = "1")]
        pub sketches: Vec<Sketch>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Sketch {
        #[prost(string, tag = "1")]
        pub metric: String,
        #[prost(string, tag = "2")]
        pub host: String,
        #[prost(string, repeated, tag = "4")]
        pub tags: Vec<String>,
        #[prost(message, repeated, tag = "7")]
        pub dogsketches: Vec<Dogsketch>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Dogsketch {
        #[prost(int64, tag = "1")]
        pub ts: i64,
        #[prost(int64, tag = "2")]
        pub cnt: i64,
        #[prost(double, tag = "3")]
        pub min: f64,
        #[prost(double, tag = "4")]
        pub max: f64,
        #[prost(double, tag = "5")]
        pub avg: f64,
        #[prost(double, tag = "6")]
        pub sum: f64,
        #[prost(sint32, repeated, tag = "7")]
        pub k: Vec<i32>,
        #[prost(uint32, repeated, tag = "8")]
        pub n: Vec<u32>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct MetricPoint {
        #[prost(double, tag = "1")]
        pub value: f64,
        #[prost(int64, tag = "2")]
        pub timestamp: i64,
    }
}

fn host_of(resources: impl IntoIterator<Item = (String, String)>) -> String {
    resources.into_iter().find(|(kind, _)| kind == "host").map(|(_, name)| name).unwrap_or_default()
}

pub fn parse_v2(body: &[u8], protobuf: bool) -> Result<Vec<Series>, String> {
    if protobuf {
        let p = <pb::MetricPayload as prost::Message>::decode(body).map_err(|e| format!("invalid protobuf payload: {e}"))?;
        return Ok(p
            .series
            .into_iter()
            .map(|s| Series {
                host: host_of(s.resources.into_iter().map(|r| (r.r#type, r.name))),
                metric: s.metric,
                kind: Kind::from_v2(s.r#type),
                unit: s.unit,
                tags: s.tags,
                interval: s.interval,
                points: s.points.into_iter().map(|p| (p.timestamp, p.value)).collect(),
            })
            .collect());
    }
    let p: V2Json = serde_json::from_slice(body).map_err(|e| format!("invalid JSON payload: {e}"))?;
    Ok(p.series
        .into_iter()
        .map(|s| Series {
            host: host_of(s.resources.into_iter().map(|r| (r.kind, r.name))),
            metric: s.metric,
            kind: Kind::from_v2(s.kind),
            unit: s.unit,
            tags: s.tags,
            interval: s.interval,
            points: s.points.into_iter().map(|p| (p.timestamp, p.value)).collect(),
        })
        .collect())
}

#[derive(Deserialize)]
struct CheckRun {
    check: String,
    #[serde(default)]
    host_name: String,
    #[serde(default)]
    timestamp: Option<f64>,
    #[serde(default)]
    status: u8,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

/// Service checks (`/api/v1/check_run`) as the gauge `service_check.status` (0 OK, 1 WARNING,
/// 2 CRITICAL, 3 UNKNOWN) tagged `check:<name>`, so monitors and dashboards can use them like metrics.
pub fn parse_check_runs(body: &[u8]) -> Result<Vec<Series>, String> {
    let runs: Vec<CheckRun> = serde_json::from_slice(body).map_err(|e| format!("invalid JSON payload: {e}"))?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    Ok(runs
        .into_iter()
        .map(|r| {
            let mut tags = r.tags.unwrap_or_default();
            tags.push(format!("check:{}", r.check));
            Series {
                metric: "service_check.status".into(),
                kind: Kind::Gauge,
                unit: String::new(),
                host: r.host_name,
                tags,
                interval: 0,
                points: vec![(r.timestamp.map_or(now, |t| t as i64), f64::from(r.status.min(3)))],
            }
        })
        .collect())
}

pub fn parse_v1(body: &[u8]) -> Result<Vec<Series>, String> {
    let p: V1Json = serde_json::from_slice(body).map_err(|e| format!("invalid JSON payload: {e}"))?;
    Ok(p.series
        .into_iter()
        .map(|s| Series {
            kind: Kind::from_v1(s.kind.as_deref()),
            metric: s.metric,
            unit: String::new(),
            host: s.host.unwrap_or_default(),
            tags: s.tags.unwrap_or_default(),
            interval: s.interval.unwrap_or(0),
            points: s.points.into_iter().filter_map(|(t, v)| Some((t as i64, v?))).collect(),
        })
        .collect())
}

// ---------------------------------------------------------------- OTLP

fn nanos(secs: i64) -> u64 {
    secs.max(0) as u64 * 1_000_000_000
}

/// Datadog container / orchestrator tags → OTel resource attributes. Metrics, spans and logs then
/// carry the same resource as OpenTelemetry senders' (the Kubernetes and host pages read those),
/// and Datadog-style queries still work: `kube_namespace:shop` resolves to `k8s.namespace.name`.
pub(crate) const TAG_ATTRS: &[(&str, &str)] = &[
    ("kube_cluster_name", "k8s.cluster.name"),
    ("kube_namespace", "k8s.namespace.name"),
    ("pod_name", "k8s.pod.name"),
    ("kube_deployment", "k8s.deployment.name"),
    ("kube_replica_set", "k8s.replicaset.name"),
    ("kube_stateful_set", "k8s.statefulset.name"),
    ("kube_daemon_set", "k8s.daemonset.name"),
    ("kube_job", "k8s.job.name"),
    ("kube_cronjob", "k8s.cronjob.name"),
    ("kube_container_name", "k8s.container.name"),
    ("kube_node", "k8s.node.name"),
    ("container_id", "container.id"),
    ("container_name", "container.name"),
    ("image_name", "container.image.name"),
    ("image_tag", "container.image.tag"),
    ("ecs_cluster_name", "aws.ecs.cluster.name"),
    ("task_family", "aws.ecs.task.family"),
    ("task_arn", "aws.ecs.task.arn"),
    ("env", "deployment.environment"),
    ("version", "service.version"),
];

/// A resource: host.name, service.name and the promoted tags (sorted, so equal resources group).
type ResourceKey = Vec<(String, String)>;

/// A series' tags split into its resource (host, `service:` and the TAG_ATTRS tags) and the data
/// point attributes.
fn split_tags(tags: &[String], host: &str) -> (ResourceKey, Vec<KeyValue>) {
    let mut resource: BTreeMap<String, String> = BTreeMap::new();
    if !host.is_empty() {
        resource.insert("host.name".into(), host.to_string());
    }
    let mut attrs = Vec::new();
    for tag in tags {
        let (k, v) = tag.split_once(':').unwrap_or((tag.as_str(), ""));
        if k == "service" {
            resource.entry("service.name".into()).or_insert_with(|| v.to_string());
        } else if k == "host" && v == host {
            // Already the resource's host.
        } else if let Some((_, otel)) = TAG_ATTRS.iter().find(|(dd, _)| *dd == k) {
            resource.entry((*otel).to_string()).or_insert_with(|| v.to_string());
        } else {
            attrs.push(otlp::kv(k, v));
        }
    }
    (resource.into_iter().collect(), attrs)
}

/// Metrics grouped by resource → an OTLP request. `collector` marks what sent them (the host list
/// shows it).
fn resource_metrics(resources: BTreeMap<ResourceKey, Vec<otlp::Metric>>, collector: Option<&str>) -> otlp::ExportMetricsServiceRequest {
    otlp::ExportMetricsServiceRequest {
        resource_metrics: resources
            .into_iter()
            .map(|(resource, metrics)| {
                let mut attributes: Vec<KeyValue> = resource.into_iter().map(|(k, v)| otlp::kv(k, v)).collect();
                if let Some(c) = collector {
                    attributes.push(otlp::kv("observecraft.collector", c));
                }
                otlp::ResourceMetrics { resource: Some(otlp::Resource { attributes }), scope_metrics: vec![otlp::ScopeMetrics { scope: otlp::scope("datadog"), metrics }] }
            })
            .collect(),
    }
}

/// OTLP metrics for the series: one resource per distinct resource attribute set.
pub fn to_otlp(series: &[Series], collector: Option<&str>) -> otlp::ExportMetricsServiceRequest {
    let mut resources: BTreeMap<ResourceKey, Vec<otlp::Metric>> = BTreeMap::new();
    for s in series {
        let (resource, attrs) = split_tags(&s.tags, &s.host);
        let data_points: Vec<otlp::NumberDataPoint> = s
            .points
            .iter()
            .map(|&(t, v)| otlp::NumberDataPoint {
                start_time_unix_nano: if s.kind == Kind::Count { nanos(t - s.interval.max(1)) } else { 0 },
                time_unix_nano: nanos(t),
                as_double: v,
                attributes: attrs.clone(),
            })
            .collect();
        let data = match s.kind {
            Kind::Count => otlp::metric::Data::Sum(otlp::Sum { data_points, aggregation_temporality: otlp::DELTA, is_monotonic: true }),
            Kind::Gauge | Kind::Rate => otlp::metric::Data::Gauge(otlp::Gauge { data_points }),
        };
        resources.entry(resource).or_default().push(otlp::Metric { name: s.metric.clone(), unit: s.unit.clone(), data: Some(data) });
    }
    resource_metrics(resources, collector)
}

// ---------------------------------------------------------------- distributions (sketches)

/// The Agent's DDSketch key mapping (datadog-agent pkg/util/quantile): relative accuracy 1/128,
/// γ = 1 + 2/128, key(v) = round(log_γ v) + bias, smallest value 1e-9.
struct SketchKeys {
    gamma_ln: f64,
    bias: i32,
}

impl SketchKeys {
    fn agent() -> SketchKeys {
        let eps = 2.0 / 128.0;
        let gamma_ln = f64::ln_1p(eps);
        let emin = (1e-9f64.ln() / gamma_ln).floor() as i32;
        SketchKeys { gamma_ln, bias: -emin + 1 }
    }
    /// The value a key stands for (γ^(k − bias)); negative keys are negative values.
    fn value(&self, k: i32) -> f64 {
        match k {
            0 => 0.0,
            k if k < 0 => -self.value(-k),
            k => ((k - self.bias) as f64 * self.gamma_ln).exp(),
        }
    }
}

/// OTLP exponential histogram scale for converted sketches: base 2^(1/64), ~1.1 % per bucket
/// (the Agent's sketches are ~1.6 %).
const SKETCH_SCALE: i32 = 6;

fn exp_index(v: f64) -> i32 {
    (v.log2() * f64::from(1 << SKETCH_SCALE)).ceil() as i32 - 1
}

fn exp_buckets(counts: &BTreeMap<i32, u64>) -> otlp::Buckets {
    let (Some((&lo, _)), Some((&hi, _))) = (counts.first_key_value(), counts.last_key_value()) else {
        return otlp::Buckets::default();
    };
    otlp::Buckets { offset: lo, bucket_counts: (lo..=hi).map(|i| counts.get(&i).copied().unwrap_or(0)).collect() }
}

/// OTLP metrics for the Agent's sketches (distribution metrics, DogStatsD histograms and
/// distributions): delta exponential histograms with count, sum, min and max.
pub fn sketches_to_otlp(sketches: &[pb::Sketch], collector: Option<&str>) -> otlp::ExportMetricsServiceRequest {
    let keys = SketchKeys::agent();
    let mut resources: BTreeMap<ResourceKey, Vec<otlp::Metric>> = BTreeMap::new();
    for s in sketches {
        let (resource, attrs) = split_tags(&s.tags, &s.host);
        let data_points: Vec<otlp::ExponentialHistogramDataPoint> = s
            .dogsketches
            .iter()
            .map(|d| {
                let (mut pos, mut neg, mut zero) = (BTreeMap::new(), BTreeMap::new(), 0u64);
                for (&k, &n) in d.k.iter().zip(&d.n) {
                    let v = keys.value(k);
                    match v.partial_cmp(&0.0) {
                        Some(std::cmp::Ordering::Greater) => *pos.entry(exp_index(v)).or_insert(0) += u64::from(n),
                        Some(std::cmp::Ordering::Less) => *neg.entry(exp_index(-v)).or_insert(0) += u64::from(n),
                        _ => zero += u64::from(n),
                    }
                }
                otlp::ExponentialHistogramDataPoint {
                    attributes: attrs.clone(),
                    start_time_unix_nano: nanos(d.ts - 10),
                    time_unix_nano: nanos(d.ts),
                    count: d.cnt.max(0) as u64,
                    sum: Some(d.sum),
                    scale: SKETCH_SCALE,
                    zero_count: zero,
                    positive: Some(exp_buckets(&pos)),
                    negative: Some(exp_buckets(&neg)),
                    min: Some(d.min),
                    max: Some(d.max),
                }
            })
            .collect();
        if data_points.is_empty() {
            continue;
        }
        let data = otlp::metric::Data::ExponentialHistogram(otlp::ExponentialHistogram { data_points, aggregation_temporality: otlp::DELTA });
        resources.entry(resource).or_default().push(otlp::Metric { name: s.metric.clone(), unit: String::new(), data: Some(data) });
    }
    resource_metrics(resources, collector)
}

// ---------------------------------------------------------------- handlers

/// The body with its Content-Encoding removed (the Agent sends zstd or zlib, others gzip).
/// `datadog-agent` when the Datadog Agent sent the request (it adds DD-Agent-Version).
pub(crate) fn sender(headers: &HeaderMap) -> Option<&'static str> {
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    (headers.contains_key("dd-agent-version") || ua.starts_with("datadog-agent")).then_some("datadog-agent")
}

pub(crate) fn inflate(headers: &HeaderMap, body: &[u8]) -> Result<Vec<u8>, String> {
    let encoding = headers.get(header::CONTENT_ENCODING).and_then(|v| v.to_str().ok()).unwrap_or("").trim().to_ascii_lowercase();
    let mut out = Vec::new();
    let read = match encoding.as_str() {
        "" | "identity" => return Ok(body.to_vec()),
        "gzip" => flate2::read::MultiGzDecoder::new(body).take(MAX_INFLATED).read_to_end(&mut out),
        // HTTP's "deflate" is zlib-wrapped; some clients send raw deflate.
        "deflate" => match flate2::read::ZlibDecoder::new(body).take(MAX_INFLATED).read_to_end(&mut out) {
            Ok(n) => Ok(n),
            Err(_) => {
                out.clear();
                flate2::read::DeflateDecoder::new(body).take(MAX_INFLATED).read_to_end(&mut out)
            }
        },
        "zstd" => zstd::stream::read::Decoder::new(body).and_then(|d| d.take(MAX_INFLATED).read_to_end(&mut out)),
        other => return Err(format!("unsupported Content-Encoding: {other}")),
    };
    read.map_err(|e| format!("cannot decompress body: {e}"))?;
    Ok(out)
}

fn bad_request(msg: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "errors": [msg] }))).into_response()
}

/// Series converted and sent per this many, so a large payload never becomes one huge request.
const CHUNK: usize = 2000;

async fn ingest(st: Shared, headers: HeaderMap, body: Bytes, parse: impl FnOnce(&[u8]) -> Result<Vec<Series>, String>, ok: Value) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let series = match inflate(&headers, &body).and_then(|raw| parse(&raw)) {
        Ok(series) => series,
        Err(e) => return bad_request(e),
    };
    drop(body);
    let collector = sender(&headers);
    for chunk in series.chunks(CHUNK) {
        if let Err(r) = deliver_metrics(&st, &org, otlp::encode(&to_otlp(chunk, collector))).await {
            return r.into_response();
        }
    }
    (StatusCode::ACCEPTED, Json(ok)).into_response()
}

/// Sends converted OTLP metrics (protobuf) to the collector.
async fn deliver_metrics(st: &Shared, org: &crate::KeyOrg, body: Bytes) -> Result<(), Rejection> {
    let target = format!("{}/v1/metrics", st.cfg.collector_url);
    let up = send(st, org, &target, Some(&HeaderValue::from_static("application/x-protobuf")), None, body).await?;
    if up.status.is_success() {
        return Ok(());
    }
    tracing::warn!(status = %up.status, "collector refused converted Datadog metrics");
    Err(Rejection { status: if up.status.is_client_error() { StatusCode::BAD_REQUEST } else { StatusCode::SERVICE_UNAVAILABLE }, message: "metrics were not accepted" })
}

pub async fn series_v2(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let protobuf = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|ct| ct.contains("protobuf"));
    ingest(st, headers, body, |raw| parse_v2(raw, protobuf), json!({ "errors": [] })).await
}

pub async fn series_v1(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    ingest(st, headers, body, parse_v1, json!({ "status": "ok" })).await
}

/// `POST /api/v1/check_run`: the Agent's service checks.
pub async fn check_run(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    ingest(st, headers, body, parse_check_runs, json!({ "status": "ok" })).await
}

/// `POST /api/beta/sketches`: the Agent's distribution metrics (SketchPayload protobuf).
pub async fn sketches(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let payload = match inflate(&headers, &body).and_then(|raw| <pb::SketchPayload as prost::Message>::decode(raw.as_slice()).map_err(|e| format!("invalid sketch payload: {e}"))) {
        Ok(p) => p,
        Err(e) => return bad_request(e),
    };
    drop(body);
    let collector = sender(&headers);
    for chunk in payload.sketches.chunks(CHUNK) {
        if let Err(r) = deliver_metrics(&st, &org, otlp::encode(&sketches_to_otlp(chunk, collector))).await {
            return r.into_response();
        }
    }
    StatusCode::ACCEPTED.into_response()
}

/// What the Agent's `/intake/` host metadata says about the host.
#[derive(Debug, Default, PartialEq)]
pub struct HostMeta {
    pub host: String,
    pub agent_version: String,
    pub os: String,
    pub platform: String,
    pub kernel: String,
    pub arch: String,
    pub cpu_cores: Option<i32>,
    pub memory_bytes: Option<i64>,
    pub cloud: String,
    pub instance_id: String,
    pub tags: Vec<String>,
}

/// Host metadata from an `/intake/` payload (None for the payloads that carry only events or
/// process summaries).
pub fn host_meta(v: &Value) -> Option<HostMeta> {
    let stats = v.get("systemStats")?;
    let s = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").to_string();
    let meta = v.get("meta");
    let host = [s(v.get("internalHostname")), s(meta.and_then(|m| m.get("hostname")))].into_iter().find(|h| !h.is_empty())?;
    // gohai arrives as a JSON string (or, from some versions, an object).
    let gohai: Value = match v.get("gohai") {
        Some(Value::String(g)) => serde_json::from_str(g).unwrap_or(Value::Null),
        Some(g) => g.clone(),
        None => Value::Null,
    };
    let nix: Vec<String> = stats.get("nixV").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str()).filter(|x| !x.is_empty()).map(str::to_string).collect()).unwrap_or_default();
    let platform = if nix.is_empty() { s(stats.get("platform")) } else { nix.join(" ") };
    let num = |x: Option<&Value>| x.and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.trim_end_matches("kB").parse().ok())));
    let host_tags = v.get("host-tags").and_then(Value::as_object);
    let mut tags: Vec<String> = host_tags.map(|t| t.values().filter_map(Value::as_array).flatten().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    tags.sort();
    tags.dedup();
    let instance_id = s(meta.and_then(|m| m.get("instance-id")));
    let cloud = if host_tags.is_some_and(|t| t.keys().any(|k| k.contains("google"))) {
        "gcp"
    } else if host_tags.is_some_and(|t| t.keys().any(|k| k.contains("azure"))) {
        "azure"
    } else if !s(meta.and_then(|m| m.get("ec2-hostname"))).is_empty() || instance_id.starts_with("i-") {
        "aws"
    } else {
        ""
    };
    Some(HostMeta {
        host,
        agent_version: s(v.get("agentVersion")),
        os: s(v.get("os")),
        platform,
        kernel: s(gohai.pointer("/platform/kernel_release")),
        arch: [s(stats.get("machine")), s(gohai.pointer("/platform/machine"))].into_iter().find(|x| !x.is_empty()).unwrap_or_default(),
        cpu_cores: num(gohai.pointer("/cpu/cpu_logical_processors")).or_else(|| num(stats.get("cpuCores"))).map(|n| n as i32),
        memory_bytes: num(gohai.pointer("/memory/total")),
        cloud: cloud.into(),
        instance_id,
        tags,
    })
}

/// `POST /intake/` (and `/api/v2/host_metadata`): the Agent's host metadata is stored (the host list
/// and host page show it) and its events go to the events table; process summaries are acknowledged.
pub async fn intake(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let (org, retention) = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let payload: Value = match inflate(&headers, &body).and_then(|raw| serde_json::from_slice(&raw).map_err(|e| format!("invalid JSON payload: {e}"))) {
        Ok(v) => v,
        Err(e) => return bad_request(e),
    };
    if let Some(m) = host_meta(&payload) {
        let stored = sqlx::query(
            "INSERT INTO datadog_hosts (org_id, host, agent_version, os, platform, kernel, arch, cpu_cores, memory_bytes, cloud, instance_id, host_tags, payload, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, now()) \
             ON CONFLICT (org_id, host) DO UPDATE SET agent_version = EXCLUDED.agent_version, os = EXCLUDED.os, platform = EXCLUDED.platform, \
             kernel = EXCLUDED.kernel, arch = EXCLUDED.arch, cpu_cores = EXCLUDED.cpu_cores, memory_bytes = EXCLUDED.memory_bytes, cloud = EXCLUDED.cloud, \
             instance_id = EXCLUDED.instance_id, host_tags = EXCLUDED.host_tags, payload = EXCLUDED.payload, updated_at = now()",
        )
        .bind(&org)
        .bind(&m.host)
        .bind(&m.agent_version)
        .bind(&m.os)
        .bind(&m.platform)
        .bind(&m.kernel)
        .bind(&m.arch)
        .bind(m.cpu_cores)
        .bind(m.memory_bytes)
        .bind(&m.cloud)
        .bind(&m.instance_id)
        .bind(&m.tags)
        .bind(sqlx::types::Json(&payload))
        .execute(&st.pg)
        .await;
        if let Err(e) = stored {
            tracing::warn!(error = %e, "storing Datadog host metadata failed");
            return Rejection { status: StatusCode::SERVICE_UNAVAILABLE, message: "temporarily unavailable" }.into_response();
        }
    }
    if let Err(r) = crate::datadog_events::deliver(&st, &(org, retention), &crate::datadog_events::intake_events(&payload)).await {
        return r.into_response();
    }
    (StatusCode::ACCEPTED, Json(json!({}))).into_response()
}

/// The Datadog Agent's host metadata (`/intake/`, `/api/v1/metadata`) and orchestrator summaries
/// (`/api/v2/orch`): authenticated and acknowledged so the Agent doesn't retry them forever, but
/// not stored (host inventory comes from metrics, Kubernetes objects from `/api/v2/orchmanif`).
pub async fn acknowledge(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    match admit(&st, &headers, body.len()).await {
        Ok(_) => (StatusCode::ACCEPTED, Json(json!({ "status": "ok" }))).into_response(),
        Err(r) => r.into_response(),
    }
}

/// `GET /api/v1/validate`: whether the API key is valid (the Datadog Agent checks it at startup).
pub async fn validate(State(st): State<Shared>, headers: HeaderMap) -> Response {
    match admit(&st, &headers, 0).await {
        Ok(_) => Json(json!({ "valid": true })).into_response(),
        Err(r) if r.status == StatusCode::UNAUTHORIZED => (StatusCode::FORBIDDEN, Json(json!({ "errors": ["Forbidden"] }))).into_response(),
        Err(r) => r.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn v2_json_and_protobuf_agree() {
        let json = br#"{"series":[{"metric":"system.load.1","type":3,"points":[{"timestamp":1700000000,"value":0.7}],
            "resources":[{"name":"web-1","type":"host"}],"tags":["env:prod","service:storefront-api"],"unit":"","interval":10}]}"#;
        let from_json = parse_v2(json, false).unwrap();
        let pb = pb::MetricPayload {
            series: vec![pb::MetricSeries {
                resources: vec![pb::Resource { r#type: "host".into(), name: "web-1".into() }],
                metric: "system.load.1".into(),
                tags: vec!["env:prod".into(), "service:storefront-api".into()],
                points: vec![pb::MetricPoint { value: 0.7, timestamp: 1_700_000_000 }],
                r#type: 3,
                unit: String::new(),
                source_type_name: String::new(),
                interval: 10,
            }],
        };
        let from_pb = parse_v2(&pb.encode_to_vec(), true).unwrap();
        assert_eq!(from_json, from_pb);
        assert_eq!(from_json[0].host, "web-1");
        assert_eq!(from_json[0].kind, Kind::Gauge);
    }

    #[test]
    fn v1_points_and_types() {
        let s = parse_v1(br#"{"series":[{"metric":"orders.placed","type":"count","interval":20,"points":[[1700000000,3],[1700000020,null]],"host":"web-2","tags":["env:prod"]}]}"#).unwrap();
        assert_eq!(s[0].kind, Kind::Count);
        assert_eq!(s[0].points, vec![(1_700_000_000, 3.0)]);
        assert_eq!(s[0].host, "web-2");
    }

    fn attrs(r: &otlp::Resource) -> Vec<(String, String)> {
        r.attributes.iter().map(|a| (a.key.clone(), otlp::text(a).to_string())).collect()
    }

    #[test]
    fn otlp_mapping() {
        let series = vec![
            Series {
                metric: "orders.placed".into(),
                kind: Kind::Count,
                unit: "".into(),
                host: "web-1".into(),
                tags: vec!["service:order-service".into(), "env:prod".into(), "canary".into()],
                interval: 10,
                points: vec![(100, 3.0)],
            },
            Series {
                metric: "queue.depth".into(),
                kind: Kind::Gauge,
                unit: "".into(),
                host: "web-1".into(),
                tags: vec!["service:order-service".into(), "env:prod".into()],
                interval: 0,
                points: vec![(100, 7.0)],
            },
        ];
        let req = to_otlp(&series, None);
        assert_eq!(req.resource_metrics.len(), 1, "same host, service and env share a resource");
        let rm = &req.resource_metrics[0];
        let pairs = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        assert_eq!(attrs(rm.resource.as_ref().unwrap()), pairs(&[("deployment.environment", "prod"), ("host.name", "web-1"), ("service.name", "order-service")]));
        let metrics = &rm.scope_metrics[0].metrics;
        let Some(otlp::metric::Data::Sum(sum)) = &metrics[0].data else { panic!("count is a sum") };
        assert_eq!(sum.aggregation_temporality, otlp::DELTA);
        assert!(sum.is_monotonic);
        let p = &sum.data_points[0];
        assert_eq!(p.time_unix_nano, 100_000_000_000);
        assert_eq!(p.start_time_unix_nano, 90_000_000_000);
        assert_eq!(p.attributes, vec![otlp::kv("canary", "")]);
        assert!(matches!(metrics[1].data, Some(otlp::metric::Data::Gauge(_))));
    }

    #[test]
    fn kubernetes_tags_become_resource_attributes() {
        let tags: Vec<String> = ["kube_namespace:shop", "pod_name:storefront-7d9", "kube_container_name:app", "kube_cluster_name:prod-eu", "team:web"].iter().map(|t| t.to_string()).collect();
        let (resource, attrs) = split_tags(&tags, "node-1");
        let r: std::collections::HashMap<_, _> = resource.into_iter().collect();
        assert_eq!(r["k8s.namespace.name"], "shop");
        assert_eq!(r["k8s.pod.name"], "storefront-7d9");
        assert_eq!(r["k8s.container.name"], "app");
        assert_eq!(r["k8s.cluster.name"], "prod-eu");
        assert_eq!(r["host.name"], "node-1");
        assert_eq!(attrs, vec![otlp::kv("team", "web")]);
    }

    #[test]
    fn host_metadata() {
        let v: Value = serde_json::from_str(
            r#"{"agentVersion":"7.84.6","internalHostname":"web-1","os":"linux",
            "systemStats":{"cpuCores":2,"machine":"arm64","platform":"linux","nixV":["ubuntu","24.04",""]},
            "meta":{"hostname":"web-1","ec2-hostname":"ip-10-0-0-1.ec2.internal","instance-id":"i-0abc"},
            "host-tags":{"system":["env:prod","team:web"]},
            "gohai":"{\"cpu\":{\"cpu_logical_processors\":\"4\"},\"memory\":{\"total\":\"16817168384\"},\"platform\":{\"kernel_release\":\"6.8.0\"}}"}"#,
        )
        .unwrap();
        let m = host_meta(&v).unwrap();
        assert_eq!(m.host, "web-1");
        assert_eq!(m.platform, "ubuntu 24.04");
        assert_eq!(m.cpu_cores, Some(4));
        assert_eq!(m.memory_bytes, Some(16_817_168_384));
        assert_eq!(m.kernel, "6.8.0");
        assert_eq!(m.arch, "arm64");
        assert_eq!(m.cloud, "aws");
        assert_eq!(m.tags, vec!["env:prod", "team:web"]);
        assert!(host_meta(&serde_json::json!({"events": {}})).is_none());
    }

    #[test]
    fn service_checks_become_a_gauge() {
        let s = parse_check_runs(br#"[{"check":"ntp.in_sync","host_name":"web-1","timestamp":1700000000,"status":2,"message":"offset 3s","tags":["env:prod"]}]"#).unwrap();
        assert_eq!(s[0].metric, "service_check.status");
        assert_eq!(s[0].points, vec![(1_700_000_000, 2.0)]);
        assert!(s[0].tags.contains(&"check:ntp.in_sync".to_string()));
        assert_eq!(s[0].host, "web-1");
    }

    #[test]
    fn sketch_keys_match_the_agent() {
        let keys = SketchKeys::agent();
        // key(v) = round(log_γ v) + bias, so value(key(v)) is within the Agent's 1/128 accuracy.
        for v in [0.001, 1.0, 42.0, 1234.5, 9.9e6] {
            let k = ((v as f64).ln() / keys.gamma_ln).round() as i32 + keys.bias;
            let back = keys.value(k);
            assert!((back - v).abs() / v < 1.0 / 64.0, "{v} → {k} → {back}");
            assert_eq!(keys.value(-k), -back);
        }
        assert_eq!(keys.value(0), 0.0);
    }

    #[test]
    fn sketches_become_exponential_histograms() {
        let keys = SketchKeys::agent();
        let key = |v: f64| (v.ln() / keys.gamma_ln).round() as i32 + keys.bias;
        let s = pb::Sketch {
            metric: "checkout.latency".into(),
            host: "web-1".into(),
            tags: vec!["service:checkout".into(), "env:prod".into()],
            dogsketches: vec![pb::Dogsketch { ts: 100, cnt: 6, min: 0.0, max: 200.0, avg: 0.0, sum: 410.0, k: vec![0, key(10.0), key(200.0)], n: vec![1, 3, 2] }],
        };
        let req = sketches_to_otlp(&[s], Some("datadog-agent"));
        let rm = &req.resource_metrics[0];
        let m = &rm.scope_metrics[0].metrics[0];
        assert_eq!(m.name, "checkout.latency");
        let Some(otlp::metric::Data::ExponentialHistogram(h)) = &m.data else { panic!("exponential histogram") };
        assert_eq!(h.aggregation_temporality, otlp::DELTA);
        let p = &h.data_points[0];
        assert_eq!(p.count, 6);
        assert_eq!(p.zero_count, 1);
        assert_eq!(p.scale, 6);
        let pos = p.positive.as_ref().unwrap();
        assert_eq!(pos.bucket_counts.iter().sum::<u64>(), 5);
        // Bucket i covers (2^((offset+i)/64), 2^((offset+i+1)/64)]: 10 lands in the first one.
        let base = 2f64.powf(1.0 / 64.0);
        assert!(base.powi(pos.offset) < 10.0 * 1.02 && 10.0 / 1.02 <= base.powi(pos.offset + 1));
        assert_eq!(pos.bucket_counts[0], 3);
        assert_eq!(*pos.bucket_counts.last().unwrap(), 2);
        let res = attrs(rm.resource.as_ref().unwrap());
        assert_eq!(res[2], ("service.name".to_string(), "checkout".to_string()));
        assert_eq!(res[3], ("observecraft.collector".to_string(), "datadog-agent".to_string()));
    }

    #[test]
    fn inflates_agent_encodings() {
        use std::io::Write;
        let raw = b"{\"series\":[]}".to_vec();
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&raw).unwrap();
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("deflate"));
        assert_eq!(inflate(&h, &z.finish().unwrap()).unwrap(), raw);
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("zstd"));
        assert_eq!(inflate(&h, &zstd::encode_all(&raw[..], 3).unwrap()).unwrap(), raw);
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
        assert!(inflate(&h, &raw).is_err());
    }
}
