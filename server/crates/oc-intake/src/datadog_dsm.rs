//! Datadog Data Streams Monitoring: the pathway checkpoints the tracers (dd-trace-go, dd-trace-py, …) aggregate
//! every 10 seconds and the Agent's trace-agent forwards unchanged (`POST /api/v0.1/pipeline_stats`, gzip'd
//! msgpack). Each stats point (a service's checkpoint on a pathway: its edge tags, the pathway hash and its
//! parent's) becomes an OTLP log record with resource `oc.signal = dd.pathways`, which the collector routes to
//! `otel.otel_pathways`; the Data Streams map reads them (`oc-api` `routes/data_streams.rs`).
//!
//! The payload (dd-trace-go `internal/datastreams/payload.go`, Apache-2.0; field names as msgpack keys):
//! `{Env, Service, Version, Lang, TracerVersion, Hostname, Stats: [{Start, Duration, Stats: [{EdgeTags, Hash,
//! ParentHash, PathwayLatency, EdgeLatency, PayloadSize, TimestampType}], Backlogs: [{Tags, Value}]}]}`. Latencies
//! and sizes are DDSketches in their protobuf form (sketches-go `ddsketch.proto`, Apache-2.0), latencies in seconds.
//!
//! Record attributes: `kind` (`point` or `backlog`); for points the edge tags as they are (`direction`, `type`,
//! `topic`, `group`, …, all joined in `edge_tags`), `hash`, `parent_hash` (decimal), `count` (checkpoints in the
//! bucket), `bucket_s`, and each sketch as its bins, `<value>:<count>` joined by commas (`pathway_latency` and
//! `edge_latency` in seconds, `payload_size` in bytes); for backlogs the tags and `value` (an offset). Only the
//! points bucketed by checkpoint time (`TimestampType` current) are kept: the tracers send each point twice.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::datadog::inflate;
use crate::otlp;
use crate::{admit, send, Rejection, Shared};

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct StatsPayload {
    #[serde(rename = "Env")]
    pub env: String,
    #[serde(rename = "Service")]
    pub service: String,
    #[serde(rename = "Version")]
    pub version: String,
    #[serde(rename = "Lang")]
    pub lang: String,
    #[serde(rename = "TracerVersion")]
    pub tracer_version: String,
    #[serde(rename = "Hostname")]
    pub hostname: String,
    #[serde(rename = "Stats")]
    pub stats: Vec<StatsBucket>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct StatsBucket {
    /// Unix nanoseconds.
    #[serde(rename = "Start")]
    pub start: u64,
    #[serde(rename = "Duration")]
    pub duration: u64,
    #[serde(rename = "Stats")]
    pub stats: Vec<StatsPoint>,
    #[serde(rename = "Backlogs")]
    pub backlogs: Vec<Backlog>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct StatsPoint {
    #[serde(rename = "EdgeTags")]
    pub edge_tags: Vec<String>,
    #[serde(rename = "Hash")]
    pub hash: u64,
    #[serde(rename = "ParentHash")]
    pub parent_hash: u64,
    #[serde(rename = "PathwayLatency")]
    pub pathway_latency: serde_bytes::ByteBuf,
    #[serde(rename = "EdgeLatency")]
    pub edge_latency: serde_bytes::ByteBuf,
    #[serde(rename = "PayloadSize")]
    pub payload_size: serde_bytes::ByteBuf,
    /// `current` (bucketed by checkpoint time) or `origin`; dd-trace-py leaves it out (current).
    #[serde(rename = "TimestampType")]
    pub timestamp_type: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Backlog {
    #[serde(rename = "Tags")]
    pub tags: Vec<String>,
    #[serde(rename = "Value")]
    pub value: i64,
}

/// sketches-go `ddsketch/pb/ddsketch.proto`.
pub mod pb {
    use std::collections::HashMap;

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DdSketch {
        #[prost(message, optional, tag = "1")]
        pub mapping: Option<IndexMapping>,
        #[prost(message, optional, tag = "2")]
        pub positive_values: Option<Store>,
        #[prost(message, optional, tag = "3")]
        pub negative_values: Option<Store>,
        #[prost(double, tag = "4")]
        pub zero_count: f64,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct IndexMapping {
        #[prost(double, tag = "1")]
        pub gamma: f64,
        #[prost(double, tag = "2")]
        pub index_offset: f64,
        /// NONE (0) is the logarithmic mapping the tracers use.
        #[prost(int32, tag = "3")]
        pub interpolation: i32,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Store {
        #[prost(map = "sint32, double", tag = "1")]
        pub bin_counts: HashMap<i32, f64>,
        #[prost(double, repeated, tag = "2")]
        pub contiguous_bin_counts: Vec<f64>,
        #[prost(sint32, tag = "3")]
        pub contiguous_bin_index_offset: i32,
    }
}

/// A sketch's bins as (value, count), values ascending: each bin's representative value, as sketches-go's
/// logarithmic mapping gives it (`gamma^(index - indexOffset) · (1 + relative accuracy)`), zeros as 0.
pub fn sketch_bins(data: &[u8]) -> Result<Vec<(f64, f64)>, String> {
    use prost::Message;
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let s = pb::DdSketch::decode(data).map_err(|e| format!("invalid sketch: {e}"))?;
    let m = s.mapping.unwrap_or_default();
    if m.gamma <= 1.0 {
        return Err("invalid sketch: gamma must be above 1".into());
    }
    if m.interpolation != 0 {
        return Err("unsupported sketch: interpolated index mapping".into());
    }
    let value = |i: i32| m.gamma.powf(f64::from(i) - m.index_offset) * (1.0 + (1.0 - 2.0 / (1.0 + m.gamma)));
    let bins = |store: &Option<pb::Store>, sign: f64| -> Vec<(f64, f64)> {
        let Some(st) = store else { return Vec::new() };
        let contiguous = st.contiguous_bin_counts.iter().enumerate().map(|(k, c)| (st.contiguous_bin_index_offset + k as i32, *c));
        contiguous.chain(st.bin_counts.iter().map(|(i, c)| (*i, *c))).filter(|(_, c)| *c > 0.0).map(|(i, c)| (sign * value(i), c)).collect()
    };
    let mut out = bins(&s.negative_values, -1.0);
    if s.zero_count > 0.0 {
        out.push((0.0, s.zero_count));
    }
    out.extend(bins(&s.positive_values, 1.0));
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(out)
}

/// Bins as `value:count` pairs joined by commas (values with 6 significant digits, so equal bins stay equal).
fn bins_attr(bins: &[(f64, f64)]) -> String {
    bins.iter().map(|(v, c)| format!("{v:.5e}:{c}")).collect::<Vec<_>>().join(",")
}

/// `key:value` edge tags as attributes (a bare tag is a key without value).
fn tag_attrs(tags: &[String], out: &mut Vec<otlp::KeyValue>) {
    for t in tags {
        let (k, v) = t.split_once(':').unwrap_or((t, ""));
        if !k.is_empty() && !matches!(k, "kind" | "hash" | "parent_hash" | "count" | "value" | "edge_tags" | "bucket_s") {
            out.push(otlp::kv(k, v));
        }
    }
}

/// The payload as OTLP log records, with the record count; `host` from the Agent's additional tags.
pub fn to_otlp(data: &[u8], host: Option<&str>) -> Result<(otlp::ExportLogsServiceRequest, usize), String> {
    let p: StatsPayload = rmp_serde::from_slice(data).map_err(|e| format!("invalid pipeline stats: {e}"))?;
    let mut records = Vec::new();
    for b in &p.stats {
        let bucket_s = (b.duration as f64 / 1e9).max(1.0).to_string();
        for s in b.stats.iter().filter(|s| s.timestamp_type.is_empty() || s.timestamp_type == "current") {
            let edge = sketch_bins(&s.edge_latency)?;
            let count: f64 = edge.iter().map(|(_, c)| c).sum();
            let mut attributes = vec![
                otlp::kv("kind", "point"),
                otlp::kv("hash", s.hash.to_string()),
                otlp::kv("parent_hash", s.parent_hash.to_string()),
                otlp::kv("edge_tags", s.edge_tags.join(",")),
                otlp::kv("count", count.to_string()),
                otlp::kv("bucket_s", bucket_s.as_str()),
                otlp::kv("edge_latency", bins_attr(&edge)),
                otlp::kv("pathway_latency", bins_attr(&sketch_bins(&s.pathway_latency)?)),
                otlp::kv("payload_size", bins_attr(&sketch_bins(&s.payload_size)?)),
            ];
            tag_attrs(&s.edge_tags, &mut attributes);
            records.push(otlp::LogRecord { time_unix_nano: b.start, observed_time_unix_nano: b.start, attributes, ..Default::default() });
        }
        for l in &b.backlogs {
            let mut attributes = vec![otlp::kv("kind", "backlog"), otlp::kv("value", l.value.to_string()), otlp::kv("edge_tags", l.tags.join(","))];
            tag_attrs(&l.tags, &mut attributes);
            records.push(otlp::LogRecord { time_unix_nano: b.start, observed_time_unix_nano: b.start, attributes, ..Default::default() });
        }
    }
    let n = records.len();
    let mut resource = vec![otlp::kv("oc.signal", "dd.pathways"), otlp::kv("service.name", p.service.as_str())];
    for (key, value) in [("deployment.environment", p.env.as_str()), ("service.version", p.version.as_str()), ("telemetry.sdk.language", p.lang.as_str()), ("telemetry.sdk.version", p.tracer_version.as_str())] {
        if !value.is_empty() {
            resource.push(otlp::kv(key, value));
        }
    }
    if let Some(h) = host.filter(|h| !h.is_empty()).or((!p.hostname.is_empty()).then_some(p.hostname.as_str())) {
        resource.push(otlp::kv("host.name", h));
    }
    let req = otlp::ExportLogsServiceRequest {
        resource_logs: vec![otlp::ResourceLogs { resource: Some(otlp::Resource { attributes: resource }), scope_logs: vec![otlp::ScopeLogs { scope: otlp::scope("datadog.data_streams"), log_records: records }] }],
    };
    Ok((req, n))
}

/// `host:` of the trace-agent's `X-Datadog-Additional-Tags`.
fn agent_host(headers: &HeaderMap) -> Option<&str> {
    headers.get("x-datadog-additional-tags").and_then(|v| v.to_str().ok())?.split(',').find_map(|t| t.trim().strip_prefix("host:"))
}

/// `POST /api/v0.1/pipeline_stats`.
pub async fn pipeline_stats(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let converted = inflate(&headers, &body).and_then(|d| to_otlp(&d, agent_host(&headers)));
    drop(body);
    match converted {
        Ok((req, n)) if n > 0 => match send(&st, &org, crate::kafka::Sink::Signals, Some(&HeaderValue::from_static("application/x-protobuf")), None, otlp::encode(&req)).await {
            Ok(up) if up.status.is_success() => StatusCode::ACCEPTED.into_response(),
            Ok(up) => {
                tracing::warn!(status = %up.status, "collector refused converted pipeline stats");
                Rejection { status: StatusCode::SERVICE_UNAVAILABLE, message: "pipeline stats were not accepted" }.into_response()
            }
            Err(r) => r.into_response(),
        },
        Ok(_) => StatusCode::ACCEPTED.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "errors": [e] }))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// One checkpoint of 0.5 s (dd-trace-go's mapping: gamma 1.015625, index offset 1.876…), as the tracer encodes it.
    const HALF_SECOND: &str = "0a1209000000000040f03f1100d88afe9e04fe3f120c1208000000000000f03f1855";

    #[test]
    fn sketches_decode_to_their_bins_values() {
        let bins = sketch_bins(&unhex(HALF_SECOND)).unwrap();
        assert_eq!(bins.len(), 1);
        assert!((bins[0].0 - 0.50256).abs() < 1e-4, "{bins:?}");
        assert_eq!(bins[0].1, 1.0);
        let size = sketch_bins(&unhex("0a1209000000000040f03f1100d88afe9e04fe3f120d1208000000000000f03f18d404")).unwrap();
        assert!((size[0].0 - 100.0).abs() / 100.0 < 0.008, "within the sketch's 1/129 relative accuracy: {size:?}");
        assert_eq!(sketch_bins(&[]).unwrap(), vec![]);
    }

    /// A producer's checkpoint and its Kafka produce offset, as dd-trace-go sends them (msgpack, before gzip).
    const PAYLOAD: &str = "85a3456e76a470726f64a753657276696365ad6f726465722d73657276696365a553746174739184a55374617274cf1876a7120529c000a84475726174696f6ecf00000002540be400a553746174739187a8456467655461677393ad646972656374696f6e3a6f7574ac746f7069633a6f7264657273aa747970653a6b61666b61a448617368cf1122334455667788aa506172656e744861736800ae506174687761794c6174656e6379c4220a1209000000000040f03f1100d88afe9e04fe3f120c1208000000000000f03f1855ab456467654c6174656e6379c4220a1209000000000040f03f1100d88afe9e04fe3f120c1208000000000000f03f1855ab5061796c6f616453697a65c4230a1209000000000040f03f1100d88afe9e04fe3f120d1208000000000000f03f18d404ad54696d657374616d7054797065a763757272656e74a84261636b6c6f67739182a45461677393ab706172746974696f6e3a30ac746f7069633a6f7264657273b2747970653a6b61666b615f70726f64756365a556616c75652aad54726163657256657273696f6ea676322e302e30a44c616e67a2676f";

    fn attrs(r: &otlp::LogRecord) -> HashMap<String, String> {
        r.attributes
            .iter()
            .map(|kv| {
                let v = match kv.value.as_ref().and_then(|v| v.value.as_ref()) {
                    Some(otlp::any_value::Value::StringValue(s)) => s.clone(),
                    other => format!("{other:?}"),
                };
                (kv.key.clone(), v)
            })
            .collect()
    }

    #[test]
    fn checkpoints_and_backlogs_become_records() {
        let (req, n) = to_otlp(&unhex(PAYLOAD), Some("i-0a1b2c3d4e5f60001")).unwrap();
        assert_eq!(n, 2);
        let rl = &req.resource_logs[0];
        let resource: HashMap<String, String> = rl.resource.as_ref().unwrap().attributes.iter().map(|kv| (kv.key.clone(), format!("{:?}", kv.value.as_ref().unwrap().value.as_ref().unwrap()))).collect();
        assert!(resource["oc.signal"].contains("dd.pathways"));
        assert!(resource["service.name"].contains("order-service"));
        assert!(resource["deployment.environment"].contains("prod"));
        assert!(resource["telemetry.sdk.language"].contains("go"));
        assert!(resource["host.name"].contains("i-0a1b2c3d4e5f60001"));
        let records = &rl.scope_logs[0].log_records;
        let point = attrs(&records[0]);
        assert_eq!(records[0].time_unix_nano, 1_762_780_000_000_000_000);
        assert_eq!((point["kind"].as_str(), point["direction"].as_str(), point["topic"].as_str(), point["type"].as_str()), ("point", "out", "orders", "kafka"));
        assert_eq!((point["hash"].as_str(), point["parent_hash"].as_str(), point["count"].as_str(), point["bucket_s"].as_str()), ("1234605616436508552", "0", "1", "10"));
        assert_eq!(point["edge_tags"], "direction:out,topic:orders,type:kafka");
        assert!(point["edge_latency"].starts_with("5.025") && point["edge_latency"].ends_with(":1"), "{}", point["edge_latency"]);
        assert_eq!(point["pathway_latency"], point["edge_latency"]);
        assert!(point["payload_size"].starts_with("1.00") || point["payload_size"].starts_with("9.9"), "{}", point["payload_size"]);
        let backlog = attrs(&records[1]);
        assert_eq!((backlog["kind"].as_str(), backlog["type"].as_str(), backlog["partition"].as_str(), backlog["value"].as_str()), ("backlog", "kafka_produce", "0", "42"));
    }

    #[test]
    fn origin_points_are_not_counted_twice() {
        use serde::Serialize;
        #[derive(Serialize)]
        struct P {
            #[serde(rename = "Service")]
            service: &'static str,
            #[serde(rename = "Stats")]
            stats: Vec<B>,
        }
        #[derive(Serialize)]
        struct B {
            #[serde(rename = "Start")]
            start: u64,
            #[serde(rename = "Duration")]
            duration: u64,
            #[serde(rename = "Stats")]
            stats: Vec<S>,
        }
        #[derive(Serialize)]
        struct S {
            #[serde(rename = "EdgeTags")]
            edge_tags: Vec<&'static str>,
            #[serde(rename = "Hash")]
            hash: u64,
            #[serde(rename = "EdgeLatency")]
            edge: serde_bytes::ByteBuf,
            #[serde(rename = "TimestampType")]
            ts: &'static str,
        }
        let point = |ts| S { edge_tags: vec!["direction:in", "group:billing-consumer", "topic:orders", "type:kafka"], hash: 7, edge: serde_bytes::ByteBuf::from(unhex(HALF_SECOND)), ts };
        let body = rmp_serde::to_vec_named(&P { service: "billing", stats: vec![B { start: 1, duration: 10_000_000_000, stats: vec![point("current"), point("origin"), point("")] }] }).unwrap();
        let (req, n) = to_otlp(&body, None).unwrap();
        assert_eq!(n, 2, "current, and dd-trace-py's points without a timestamp type");
        assert_eq!(attrs(&req.resource_logs[0].scope_logs[0].log_records[0])["group"], "billing-consumer");
        assert!(to_otlp(b"\x91\x01", None).is_err(), "not a payload");
    }
}
