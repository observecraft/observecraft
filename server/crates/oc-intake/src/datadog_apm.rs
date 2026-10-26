//! Datadog APM intake: what the Datadog Agent's trace-agent sends (`apm_config.apm_dd_url`), so
//! applications instrumented with Datadog's tracing libraries (dd-trace-*) report to ObserveCraft
//! unchanged.
//!
//! - `POST /api/v0.2/traces`  AgentPayload protobuf (gzip / zstd) → OTLP spans → Collector
//! - `POST /api/v0.2/stats`   pre-aggregated APM stats: acknowledged; ObserveCraft computes its own
//!   from the spans
//!
//! Span mapping (the reverse of Datadog's OTLP ingestion):
//!   resource → span name (what ObserveCraft shows as the resource), name → `operation.name`,
//!   type → `span.type`, meta / metrics → attributes, error → status ERROR plus an `exception`
//!   event (error.type / error.message / error.stack), `span.kind` meta (or the type) → kind,
//!   64-bit ids → hex (the upper trace id half from `_dd.p.tid`).
//!   service / env / version / host / language / container (and the Agent's container tags) → resource.

use std::collections::BTreeMap;
use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::datadog::{inflate, TAG_ATTRS};
use crate::otlp;
use crate::{admit, send, Rejection, Shared};

/// The trace-agent's protobuf (datadog-agent, pkg/proto/datadog/trace).
pub mod pb {
    use std::collections::HashMap;

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct AgentPayload {
        #[prost(string, tag = "1")]
        pub host_name: String,
        #[prost(string, tag = "2")]
        pub env: String,
        #[prost(message, repeated, tag = "5")]
        pub tracer_payloads: Vec<TracerPayload>,
        #[prost(map = "string, string", tag = "6")]
        pub tags: HashMap<String, String>,
        #[prost(string, tag = "7")]
        pub agent_version: String,
        #[prost(message, repeated, tag = "11")]
        pub idx_tracer_payloads: Vec<super::idx::TracerPayload>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TracerPayload {
        #[prost(string, tag = "1")]
        pub container_id: String,
        #[prost(string, tag = "2")]
        pub language_name: String,
        #[prost(string, tag = "3")]
        pub language_version: String,
        #[prost(string, tag = "4")]
        pub tracer_version: String,
        #[prost(string, tag = "5")]
        pub runtime_id: String,
        #[prost(message, repeated, tag = "6")]
        pub chunks: Vec<TraceChunk>,
        #[prost(map = "string, string", tag = "7")]
        pub tags: HashMap<String, String>,
        #[prost(string, tag = "8")]
        pub env: String,
        #[prost(string, tag = "9")]
        pub hostname: String,
        #[prost(string, tag = "10")]
        pub app_version: String,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TraceChunk {
        #[prost(int32, tag = "1")]
        pub priority: i32,
        #[prost(string, tag = "2")]
        pub origin: String,
        #[prost(message, repeated, tag = "3")]
        pub spans: Vec<Span>,
        #[prost(map = "string, string", tag = "4")]
        pub tags: HashMap<String, String>,
        #[prost(bool, tag = "5")]
        pub dropped_trace: bool,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Span {
        #[prost(string, tag = "1")]
        pub service: String,
        #[prost(string, tag = "2")]
        pub name: String,
        #[prost(string, tag = "3")]
        pub resource: String,
        #[prost(uint64, tag = "4")]
        pub trace_id: u64,
        #[prost(uint64, tag = "5")]
        pub span_id: u64,
        #[prost(uint64, tag = "6")]
        pub parent_id: u64,
        #[prost(int64, tag = "7")]
        pub start: i64,
        #[prost(int64, tag = "8")]
        pub duration: i64,
        #[prost(int32, tag = "9")]
        pub error: i32,
        #[prost(map = "string, string", tag = "10")]
        pub meta: HashMap<String, String>,
        #[prost(map = "string, double", tag = "11")]
        pub metrics: HashMap<String, f64>,
        #[prost(string, tag = "12")]
        pub r#type: String,
        #[prost(message, repeated, tag = "14")]
        pub span_links: Vec<SpanLink>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SpanLink {
        #[prost(uint64, tag = "1")]
        pub trace_id: u64,
        #[prost(uint64, tag = "2")]
        pub trace_id_high: u64,
        #[prost(uint64, tag = "3")]
        pub span_id: u64,
        #[prost(map = "string, string", tag = "4")]
        pub attributes: HashMap<String, String>,
        #[prost(string, tag = "5")]
        pub tracestate: String,
    }
}

/// The idx ("string table") format newer Agents send (datadog-agent pkg/proto/datadog/trace/idx).
pub mod idx {
    use std::collections::HashMap;

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct AnyValue {
        #[prost(oneof = "any_value::Value", tags = "1, 2, 3, 4, 5, 6, 7")]
        pub value: Option<any_value::Value>,
    }
    pub mod any_value {
        // Variant names follow the agent's proto.
        #[allow(clippy::enum_variant_names)]
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Value {
            #[prost(uint32, tag = "1")]
            StringValueRef(u32),
            #[prost(bool, tag = "2")]
            BoolValue(bool),
            #[prost(double, tag = "3")]
            DoubleValue(f64),
            #[prost(int64, tag = "4")]
            IntValue(i64),
            #[prost(bytes, tag = "5")]
            BytesValue(Vec<u8>),
            #[prost(message, tag = "6")]
            ArrayValue(super::ArrayValue),
            #[prost(message, tag = "7")]
            KeyValueList(super::KeyValueList),
        }
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ArrayValue {
        #[prost(message, repeated, tag = "1")]
        pub values: Vec<AnyValue>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct KeyValue {
        #[prost(uint32, tag = "1")]
        pub key: u32,
        #[prost(message, optional, tag = "2")]
        pub value: Option<AnyValue>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct KeyValueList {
        #[prost(message, repeated, tag = "1")]
        pub key_values: Vec<KeyValue>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SpanLink {
        #[prost(bytes = "vec", tag = "1")]
        pub trace_id: Vec<u8>,
        #[prost(fixed64, tag = "2")]
        pub span_id: u64,
        #[prost(map = "uint32, message", tag = "3")]
        pub attributes: HashMap<u32, AnyValue>,
        #[prost(uint32, tag = "4")]
        pub tracestate_ref: u32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SpanEvent {
        #[prost(fixed64, tag = "1")]
        pub time: u64,
        #[prost(uint32, tag = "2")]
        pub name_ref: u32,
        #[prost(map = "uint32, message", tag = "3")]
        pub attributes: HashMap<u32, AnyValue>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Span {
        #[prost(uint32, tag = "1")]
        pub service_ref: u32,
        #[prost(uint32, tag = "2")]
        pub name_ref: u32,
        #[prost(uint32, tag = "3")]
        pub resource_ref: u32,
        #[prost(fixed64, tag = "4")]
        pub span_id: u64,
        #[prost(uint64, tag = "5")]
        pub parent_id: u64,
        #[prost(fixed64, tag = "6")]
        pub start: u64,
        #[prost(uint64, tag = "7")]
        pub duration: u64,
        #[prost(bool, tag = "8")]
        pub error: bool,
        #[prost(map = "uint32, message", tag = "9")]
        pub attributes: HashMap<u32, AnyValue>,
        #[prost(uint32, tag = "10")]
        pub type_ref: u32,
        #[prost(message, repeated, tag = "11")]
        pub links: Vec<SpanLink>,
        #[prost(message, repeated, tag = "12")]
        pub events: Vec<SpanEvent>,
        #[prost(uint32, tag = "13")]
        pub env_ref: u32,
        #[prost(uint32, tag = "14")]
        pub version_ref: u32,
        #[prost(uint32, tag = "15")]
        pub component_ref: u32,
        #[prost(int32, tag = "16")]
        pub kind: i32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TraceChunk {
        #[prost(int32, tag = "1")]
        pub priority: i32,
        #[prost(uint32, tag = "2")]
        pub origin_ref: u32,
        #[prost(map = "uint32, message", tag = "3")]
        pub attributes: HashMap<u32, AnyValue>,
        #[prost(message, repeated, tag = "4")]
        pub spans: Vec<Span>,
        #[prost(bool, tag = "5")]
        pub dropped_trace: bool,
        #[prost(bytes = "vec", tag = "6")]
        pub trace_id: Vec<u8>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TracerPayload {
        #[prost(string, repeated, tag = "1")]
        pub strings: Vec<String>,
        #[prost(uint32, tag = "2")]
        pub container_id_ref: u32,
        #[prost(uint32, tag = "3")]
        pub language_name_ref: u32,
        #[prost(uint32, tag = "4")]
        pub language_version_ref: u32,
        #[prost(uint32, tag = "5")]
        pub tracer_version_ref: u32,
        #[prost(uint32, tag = "6")]
        pub runtime_id_ref: u32,
        #[prost(uint32, tag = "7")]
        pub env_ref: u32,
        #[prost(uint32, tag = "8")]
        pub hostname_ref: u32,
        #[prost(uint32, tag = "9")]
        pub app_version_ref: u32,
        #[prost(map = "uint32, message", tag = "10")]
        pub attributes: HashMap<u32, AnyValue>,
        #[prost(message, repeated, tag = "11")]
        pub chunks: Vec<TraceChunk>,
    }
}

// ---------------------------------------------------------------- one model for both formats

/// A tracer (process) the spans came from.
#[derive(Default)]
pub struct Tracer {
    pub container_id: String,
    pub language: String,
    pub language_version: String,
    pub tracer_version: String,
    pub env: String,
    pub hostname: String,
    pub app_version: String,
    pub tags: HashMap<String, String>,
}

/// A span link: ((trace id high, low), span id, attributes, tracestate).
pub type DdLink = ((u64, u64), u64, Vec<(String, String)>, String);
/// A span event: (time ns, name, attributes).
pub type DdEvent = (u64, String, Vec<(String, String)>);

#[derive(Default)]
pub struct DdSpan {
    pub service: String,
    pub name: String,
    pub resource: String,
    pub trace_high: u64,
    pub trace_low: u64,
    pub span_id: u64,
    pub parent_id: u64,
    pub start: u64,
    pub duration: u64,
    pub error: bool,
    pub meta: HashMap<String, String>,
    pub metrics: HashMap<String, f64>,
    pub kind: String,
    pub links: Vec<DdLink>,
    pub events: Vec<DdEvent>,
}

impl DdSpan {
    fn from_legacy(s: &pb::Span, chunk_tags: &HashMap<String, String>) -> DdSpan {
        DdSpan {
            service: s.service.clone(),
            name: s.name.clone(),
            resource: s.resource.clone(),
            trace_high: s.meta.get("_dd.p.tid").or_else(|| chunk_tags.get("_dd.p.tid")).and_then(|h| u64::from_str_radix(h, 16).ok()).unwrap_or(0),
            trace_low: s.trace_id,
            span_id: s.span_id,
            parent_id: s.parent_id,
            start: s.start.max(0) as u64,
            duration: s.duration.max(0) as u64,
            error: s.error != 0,
            meta: s.meta.clone(),
            metrics: s.metrics.clone(),
            kind: s.r#type.clone(),
            links: s.span_links.iter().map(|l| ((l.trace_id_high, l.trace_id), l.span_id, l.attributes.iter().map(|(k, v)| (k.clone(), v.clone())).collect(), l.tracestate.clone())).collect(),
            events: vec![],
        }
    }
}

/// String table lookup (out of range → "").
fn sref(strings: &[String], r: u32) -> String {
    strings.get(r as usize).cloned().unwrap_or_default()
}

/// An idx value as text (strings, bools, bytes, arrays and maps) or as a number.
enum Val {
    Text(String),
    Num(f64),
}

fn idx_value(strings: &[String], v: &idx::AnyValue) -> Option<Val> {
    use idx::any_value::Value;
    Some(match v.value.as_ref()? {
        Value::StringValueRef(r) => Val::Text(sref(strings, *r)),
        Value::BoolValue(b) => Val::Text(b.to_string()),
        Value::DoubleValue(d) => Val::Num(*d),
        Value::IntValue(i) => Val::Num(*i as f64),
        Value::BytesValue(b) => Val::Text(String::from_utf8_lossy(b).into_owned()),
        Value::ArrayValue(a) => Val::Text(idx_json(strings, &Value::ArrayValue(a.clone())).to_string()),
        Value::KeyValueList(l) => Val::Text(idx_json(strings, &Value::KeyValueList(l.clone())).to_string()),
    })
}

fn idx_json(strings: &[String], v: &idx::any_value::Value) -> Value {
    use idx::any_value::Value as V;
    match v {
        V::StringValueRef(r) => json!(sref(strings, *r)),
        V::BoolValue(b) => json!(b),
        V::DoubleValue(d) => json!(d),
        V::IntValue(i) => json!(i),
        V::BytesValue(b) => json!(String::from_utf8_lossy(b)),
        V::ArrayValue(a) => Value::Array(a.values.iter().filter_map(|x| x.value.as_ref()).map(|x| idx_json(strings, x)).collect()),
        V::KeyValueList(l) => Value::Object(l.key_values.iter().filter_map(|kv| Some((sref(strings, kv.key), idx_json(strings, kv.value.as_ref()?.value.as_ref()?)))).collect()),
    }
}

fn idx_text_attrs(strings: &[String], attrs: &HashMap<u32, idx::AnyValue>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = attrs
        .iter()
        .filter_map(|(k, v)| {
            Some((
                sref(strings, *k),
                match idx_value(strings, v)? {
                    Val::Text(t) => t,
                    Val::Num(n) => n.to_string(),
                },
            ))
        })
        .collect();
    out.sort();
    out
}

/// (high, low) halves of a 128-bit big-endian trace id.
fn trace_halves(id: &[u8]) -> (u64, u64) {
    let mut b = [0u8; 16];
    let n = id.len().min(16);
    b[16 - n..].copy_from_slice(&id[id.len() - n..]);
    (u64::from_be_bytes(b[..8].try_into().unwrap()), u64::from_be_bytes(b[8..].try_into().unwrap()))
}

const KINDS: [&str; 6] = ["", "internal", "server", "client", "producer", "consumer"];

fn from_idx(t: &idx::TracerPayload) -> (Tracer, Vec<DdSpan>) {
    let st = &t.strings;
    let tracer = Tracer {
        container_id: sref(st, t.container_id_ref),
        language: sref(st, t.language_name_ref),
        language_version: sref(st, t.language_version_ref),
        tracer_version: sref(st, t.tracer_version_ref),
        env: sref(st, t.env_ref),
        hostname: sref(st, t.hostname_ref),
        app_version: sref(st, t.app_version_ref),
        tags: idx_text_attrs(st, &t.attributes).into_iter().collect(),
    };
    let mut spans = Vec::new();
    for chunk in t.chunks.iter().filter(|c| !c.dropped_trace) {
        let (high, low) = trace_halves(&chunk.trace_id);
        for s in &chunk.spans {
            let mut meta = HashMap::new();
            let mut metrics = HashMap::new();
            for (k, v) in &s.attributes {
                match idx_value(st, v) {
                    Some(Val::Text(x)) => {
                        meta.insert(sref(st, *k), x);
                    }
                    Some(Val::Num(n)) => {
                        metrics.insert(sref(st, *k), n);
                    }
                    None => {}
                }
            }
            for (key, r) in [("env", s.env_ref), ("version", s.version_ref), ("component", s.component_ref)] {
                let v = sref(st, r);
                if !v.is_empty() {
                    meta.insert(key.into(), v);
                }
            }
            if let Some(k) = KINDS.get(s.kind as usize).filter(|k| !k.is_empty()) {
                meta.entry("span.kind".into()).or_insert_with(|| (*k).to_string());
            }
            spans.push(DdSpan {
                service: sref(st, s.service_ref),
                name: sref(st, s.name_ref),
                resource: sref(st, s.resource_ref),
                trace_high: high,
                trace_low: low,
                span_id: s.span_id,
                parent_id: s.parent_id,
                start: s.start,
                duration: s.duration,
                error: s.error,
                meta,
                metrics,
                kind: sref(st, s.type_ref),
                links: s.links.iter().map(|l| (trace_halves(&l.trace_id), l.span_id, idx_text_attrs(st, &l.attributes), sref(st, l.tracestate_ref))).collect(),
                events: s.events.iter().map(|e| (e.time, sref(st, e.name_ref), idx_text_attrs(st, &e.attributes))).collect(),
            });
        }
    }
    (tracer, spans)
}

/// Every tracer payload of an AgentPayload, in either format.
pub fn tracers(p: &pb::AgentPayload) -> Vec<(Tracer, Vec<DdSpan>)> {
    let mut out: Vec<(Tracer, Vec<DdSpan>)> = p
        .tracer_payloads
        .iter()
        .map(|t| {
            let tracer = Tracer {
                container_id: t.container_id.clone(),
                language: t.language_name.clone(),
                language_version: t.language_version.clone(),
                tracer_version: t.tracer_version.clone(),
                env: t.env.clone(),
                hostname: t.hostname.clone(),
                app_version: t.app_version.clone(),
                tags: t.tags.clone(),
            };
            let spans = t.chunks.iter().filter(|c| !c.dropped_trace).flat_map(|c| c.spans.iter().map(|s| DdSpan::from_legacy(s, &c.tags))).collect();
            (tracer, spans)
        })
        .collect();
    out.extend(p.idx_tracer_payloads.iter().map(from_idx));
    out
}

// ---------------------------------------------------------------- OTLP

fn trace_id(high: u64, low: u64) -> Vec<u8> {
    let mut id = high.to_be_bytes().to_vec();
    id.extend_from_slice(&low.to_be_bytes());
    id
}

fn span_kind(meta: &HashMap<String, String>, kind: &str) -> i32 {
    match meta.get("span.kind").map(String::as_str) {
        Some("internal") => 1,
        Some("server") => 2,
        Some("client") => 3,
        Some("producer") => 4,
        Some("consumer") => 5,
        _ => match kind {
            "web" => 2,
            "http" | "grpc" | "db" | "sql" | "cache" | "redis" | "memcached" | "mongodb" | "cassandra" | "elasticsearch" | "queue" => 3,
            _ => 1,
        },
    }
}

/// Resource attributes of one span: service, env, version, host, tracer and container identity.
fn resource_attrs(p: &pb::AgentPayload, t: &Tracer, s: &DdSpan) -> Vec<(String, String)> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let mut set = |k: &str, v: &str| {
        if !v.is_empty() {
            out.insert(k.to_string(), v.to_string());
        }
    };
    set("service.name", &s.service);
    let env = [s.meta.get("env").map(String::as_str), Some(t.env.as_str()), Some(p.env.as_str())].into_iter().flatten().find(|e| !e.is_empty()).unwrap_or("");
    set("deployment.environment", env);
    let version = s.meta.get("version").map(String::as_str).filter(|v| !v.is_empty()).unwrap_or(&t.app_version);
    set("service.version", version);
    set("host.name", if t.hostname.is_empty() { &p.host_name } else { &t.hostname });
    set("telemetry.sdk.language", &t.language);
    set("telemetry.sdk.name", if t.tracer_version.is_empty() { "" } else { "datadog" });
    set("telemetry.sdk.version", &t.tracer_version);
    set("process.runtime.version", &t.language_version);
    set("container.id", &t.container_id);
    // Container / orchestrator tags the Agent attaches ("_dd.tags.container": "kube_namespace:shop,pod_name:…").
    if let Some(tags) = t.tags.get("_dd.tags.container") {
        for tag in tags.split(',') {
            if let Some((k, v)) = tag.split_once(':') {
                let key = TAG_ATTRS.iter().find(|(dd, _)| *dd == k).map(|(_, otel)| *otel).unwrap_or(k);
                set(key, v);
            }
        }
    }
    out.into_iter().collect()
}

fn otlp_span(s: &DdSpan) -> otlp::Span {
    let mut attributes = vec![otlp::kv("operation.name", s.name.as_str()), otlp::kv("resource.name", s.resource.as_str())];
    if !s.kind.is_empty() {
        attributes.push(otlp::kv("span.type", s.kind.as_str()));
    }
    let mut meta: Vec<_> = s.meta.iter().filter(|(k, _)| !matches!(k.as_str(), "_dd.p.tid" | "env" | "version")).collect();
    meta.sort();
    attributes.extend(meta.into_iter().map(|(k, v)| otlp::kv(k.as_str(), v.as_str())));
    let mut metrics: Vec<_> = s.metrics.iter().collect();
    metrics.sort_by(|a, b| a.0.cmp(b.0));
    attributes.extend(metrics.into_iter().map(|(k, v)| otlp::kv_f64(k.as_str(), *v)));
    let end = s.start + s.duration;
    let mut events: Vec<otlp::SpanEvent> =
        s.events.iter().map(|(t, name, a)| otlp::SpanEvent { time_unix_nano: *t, name: name.clone(), attributes: a.iter().map(|(k, v)| otlp::kv(k.as_str(), v.as_str())).collect() }).collect();
    let mut status = None;
    if s.error {
        let message = s.meta.get("error.message").or_else(|| s.meta.get("error.msg")).cloned().unwrap_or_default();
        if !events.iter().any(|e| e.name == "exception") {
            let mut ev = vec![otlp::kv("exception.message", message.as_str())];
            if let Some(t) = s.meta.get("error.type") {
                ev.push(otlp::kv("exception.type", t.as_str()));
            }
            if let Some(st) = s.meta.get("error.stack") {
                ev.push(otlp::kv("exception.stacktrace", st.as_str()));
            }
            events.push(otlp::SpanEvent { time_unix_nano: end, name: "exception".into(), attributes: ev });
        }
        status = Some(otlp::Status { message, code: otlp::STATUS_ERROR });
    }
    otlp::Span {
        trace_id: trace_id(s.trace_high, s.trace_low),
        span_id: s.span_id.to_be_bytes().to_vec(),
        trace_state: String::new(),
        parent_span_id: if s.parent_id == 0 { vec![] } else { s.parent_id.to_be_bytes().to_vec() },
        name: if s.resource.is_empty() { s.name.clone() } else { s.resource.clone() },
        kind: span_kind(&s.meta, &s.kind),
        start_time_unix_nano: s.start,
        end_time_unix_nano: end,
        attributes,
        events,
        links: s
            .links
            .iter()
            .map(|((h, l), span, a, state)| otlp::SpanLink {
                trace_id: trace_id(*h, *l),
                span_id: span.to_be_bytes().to_vec(),
                trace_state: state.clone(),
                attributes: a.iter().map(|(k, v)| otlp::kv(k.as_str(), v.as_str())).collect(),
            })
            .collect(),
        status,
    }
}

/// OTLP spans for an AgentPayload, grouped by resource.
pub fn to_otlp(p: &pb::AgentPayload) -> (otlp::ExportTraceServiceRequest, usize) {
    let mut groups: BTreeMap<Vec<(String, String)>, Vec<otlp::Span>> = BTreeMap::new();
    let mut n = 0;
    for (t, spans) in tracers(p) {
        for s in &spans {
            groups.entry(resource_attrs(p, &t, s)).or_default().push(otlp_span(s));
            n += 1;
        }
    }
    let resource_spans = groups
        .into_iter()
        .map(|(attrs, spans)| otlp::ResourceSpans {
            resource: Some(otlp::Resource { attributes: attrs.into_iter().map(|(k, v)| otlp::kv(k, v)).collect() }),
            scope_spans: vec![otlp::ScopeSpans { scope: otlp::scope("datadog"), spans }],
        })
        .collect();
    (otlp::ExportTraceServiceRequest { resource_spans }, n)
}

pub async fn traces(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let payload = match inflate(&headers, &body).and_then(|raw| <pb::AgentPayload as prost::Message>::decode(raw.as_slice()).map_err(|e| format!("invalid AgentPayload: {e}"))) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "errors": [e] }))).into_response(),
    };
    let (otlp, n) = to_otlp(&payload);
    tracing::debug!(spans = n, agent = %payload.agent_version, "datadog traces");
    if n == 0 {
        return StatusCode::ACCEPTED.into_response();
    }
    drop(body);
    let target = format!("{}/v1/traces", st.cfg.collector_url);
    match send(&st, &org, &target, Some(&HeaderValue::from_static("application/x-protobuf")), None, crate::otlp::encode(&otlp)).await {
        Ok(up) if up.status.is_success() => StatusCode::ACCEPTED.into_response(),
        Ok(up) => {
            tracing::warn!(status = %up.status, "collector refused converted Datadog spans");
            Rejection { status: if up.status.is_client_error() { StatusCode::BAD_REQUEST } else { StatusCode::SERVICE_UNAVAILABLE }, message: "spans were not accepted" }.into_response()
        }
        Err(r) => r.into_response(),
    }
}

/// APM stats: authenticated and acknowledged (ObserveCraft derives its own from the spans).
pub async fn stats(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    match admit(&st, &headers, body.len()).await {
        Ok(_) => StatusCode::ACCEPTED.into_response(),
        Err(r) => r.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn res_map(rs: &otlp::ResourceSpans) -> HashMap<String, String> {
        rs.resource.as_ref().unwrap().attributes.iter().map(|a| (a.key.clone(), otlp::text(a).to_string())).collect()
    }

    fn span(id: u64, parent: u64, error: i32) -> pb::Span {
        let mut meta = HashMap::new();
        meta.insert("env".into(), "prod".into());
        meta.insert("_dd.p.tid".into(), "6553f1f000000000".into());
        meta.insert("http.method".into(), "GET".into());
        if error != 0 {
            meta.insert("error.type".into(), "ValueError".into());
            meta.insert("error.message".into(), "bad cart".into());
        }
        let mut metrics = HashMap::new();
        metrics.insert("_sampling_priority_v1".into(), 1.0);
        pb::Span {
            service: "storefront-api".into(),
            name: "flask.request".into(),
            resource: "GET /cart".into(),
            trace_id: 42,
            span_id: id,
            parent_id: parent,
            start: 1_700_000_000_000_000_000,
            duration: 5_000_000,
            error,
            meta,
            metrics,
            r#type: "web".into(),
            span_links: vec![],
        }
    }

    #[test]
    fn maps_agent_payload() {
        let mut ttags = HashMap::new();
        ttags.insert("_dd.tags.container".into(), "kube_namespace:shop,pod_name:storefront-7d9,kube_deployment:storefront".into());
        let p = pb::AgentPayload {
            host_name: "node-1".into(),
            env: "".into(),
            tracer_payloads: vec![pb::TracerPayload {
                container_id: "abc".into(),
                language_name: "python".into(),
                tracer_version: "2.14.0".into(),
                chunks: vec![pb::TraceChunk { spans: vec![span(1, 0, 0), span(2, 1, 1)], ..Default::default() }],
                tags: ttags,
                app_version: "1.4.2".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let (v, n) = to_otlp(&p);
        assert_eq!(n, 2);
        let rs = &v.resource_spans[0];
        let attrs = res_map(rs);
        assert_eq!(attrs["service.name"], "storefront-api");
        assert_eq!(attrs["deployment.environment"], "prod");
        assert_eq!(attrs["service.version"], "1.4.2");
        assert_eq!(attrs["host.name"], "node-1");
        assert_eq!(attrs["k8s.namespace.name"], "shop");
        assert_eq!(attrs["k8s.deployment.name"], "storefront");
        let spans = &rs.scope_spans[0].spans;
        assert_eq!(hex(&spans[0].trace_id), "6553f1f000000000000000000000002a");
        assert_eq!(spans[0].name, "GET /cart");
        assert_eq!(spans[0].kind, 2);
        assert!(spans[0].parent_span_id.is_empty());
        assert_eq!(hex(&spans[1].parent_span_id), "0000000000000001");
        assert_eq!(spans[1].status.as_ref().unwrap().code, otlp::STATUS_ERROR);
        assert_eq!(spans[1].events[0].name, "exception");
        assert_eq!(spans[0].end_time_unix_nano, 1_700_000_000_005_000_000);
    }

    #[test]
    fn maps_idx_payloads() {
        use idx::any_value::Value as V;
        let strings: Vec<String> =
            ["", "dd-checkout", "flask.request", "POST /checkout", "web", "http.method", "POST", "prod", "1.0.0", "python", "_dd.tags.container", "kube_namespace:shop", "http.status_code"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        let mut attrs = HashMap::new();
        attrs.insert(5, idx::AnyValue { value: Some(V::StringValueRef(6)) });
        attrs.insert(12, idx::AnyValue { value: Some(V::IntValue(200)) });
        let mut tattrs = HashMap::new();
        tattrs.insert(10, idx::AnyValue { value: Some(V::StringValueRef(11)) });
        let mut trace_id = vec![0u8; 16];
        trace_id[7] = 1;
        trace_id[15] = 2;
        let t = idx::TracerPayload {
            strings,
            language_name_ref: 9,
            attributes: tattrs,
            chunks: vec![idx::TraceChunk {
                trace_id,
                spans: vec![idx::Span {
                    service_ref: 1,
                    name_ref: 2,
                    resource_ref: 3,
                    span_id: 9,
                    start: 1_000,
                    duration: 500,
                    error: true,
                    attributes: attrs,
                    type_ref: 4,
                    env_ref: 7,
                    version_ref: 8,
                    kind: 2,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let p = pb::AgentPayload { host_name: "node-1".into(), idx_tracer_payloads: vec![t], ..Default::default() };
        let (v, n) = to_otlp(&p);
        assert_eq!(n, 1);
        let rs = &v.resource_spans[0];
        let res = res_map(rs);
        assert_eq!(res["service.name"], "dd-checkout");
        assert_eq!(res["deployment.environment"], "prod");
        assert_eq!(res["service.version"], "1.0.0");
        assert_eq!(res["k8s.namespace.name"], "shop");
        let sp = &rs.scope_spans[0].spans[0];
        assert_eq!(hex(&sp.trace_id), "00000000000000010000000000000002");
        assert_eq!(sp.name, "POST /checkout");
        assert_eq!(sp.kind, 2);
        assert_eq!(sp.status.as_ref().unwrap().code, otlp::STATUS_ERROR);
        assert!(sp.attributes.contains(&otlp::kv_f64("http.status_code", 200.0)));
        assert!(sp.attributes.contains(&otlp::kv("http.method", "POST")));
    }

    #[test]
    fn decodes_protobuf() {
        use prost::Message;
        let p = pb::AgentPayload {
            host_name: "h".into(),
            tracer_payloads: vec![pb::TracerPayload { chunks: vec![pb::TraceChunk { spans: vec![span(7, 0, 0)], ..Default::default() }], ..Default::default() }],
            ..Default::default()
        };
        let back = pb::AgentPayload::decode(p.encode_to_vec().as_slice()).unwrap();
        assert_eq!(back.tracer_payloads[0].chunks[0].spans[0].span_id, 7);
    }
}
