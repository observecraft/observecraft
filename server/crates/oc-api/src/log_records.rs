//! Kafka log records into the events `log_normalize` takes, shaped as the path before it shaped them: OTLP
//! (`oc.otlp.logs` protobuf, `oc.otlp-json.logs`) as the Collector's `resource/org` processor and Vector's
//! `opentelemetry` source, JSON logs (`oc.logs.json`) as Vector's `kafka` source with its JSON codec.
//!
//! OTLP resources carrying another signal (`oc.signal`: Kubernetes objects, Datadog events, network
//! connections, RUM, replay) are the Collector's and are skipped here. The organization is the record's
//! `x-oc-org` header, never the sender's `oc.org_id`.

use base64::Engine;
use chrono::{DateTime, Utc};
use rskafka::record::Record;
use serde_json::{Map, Value};

use crate::log_normalize::LogEvent;

/// The topic a record came from, i.e. its encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    /// `oc.otlp.logs`: OTLP `ExportLogsServiceRequest` protobuf.
    Otlp,
    /// `oc.otlp-json.logs`: the same as OTLP/JSON.
    OtlpJson,
    /// `oc.logs.json`: a JSON log or an array of them.
    Json,
}

impl Topic {
    pub const ALL: [(Topic, &'static str); 3] = [(Topic::Otlp, "oc.otlp.logs"), (Topic::OtlpJson, "oc.otlp-json.logs"), (Topic::Json, "oc.logs.json")];
}

/// `oc.signal` values the Collector routes to their own tables (`deploy/otel/collector.yaml`).
const OTHER_SIGNALS: &[&str] = &["k8s.objects", "dd.events", "dd.connections", "dd.rum", "dd.replay"];

// ---------------------------------------------------------------- OTLP logs protobuf
// Field numbers from OpenTelemetry's logs/v1/logs.proto and common/v1/common.proto (Apache-2.0).

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
pub struct Resource {
    #[prost(message, repeated, tag = "1")]
    pub attributes: Vec<KeyValue>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ScopeLogs {
    #[prost(message, optional, tag = "1")]
    pub scope: Option<InstrumentationScope>,
    #[prost(message, repeated, tag = "2")]
    pub log_records: Vec<LogRecord>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct InstrumentationScope {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub version: String,
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
    #[prost(fixed32, tag = "8")]
    pub flags: u32,
    #[prost(bytes = "vec", tag = "9")]
    pub trace_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "10")]
    pub span_id: Vec<u8>,
    #[prost(fixed64, tag = "11")]
    pub observed_time_unix_nano: u64,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct KeyValue {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(message, optional, tag = "2")]
    pub value: Option<AnyValue>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct AnyValue {
    #[prost(oneof = "any_value::Value", tags = "1, 2, 3, 4, 5, 6, 7")]
    pub value: Option<any_value::Value>,
}
pub mod any_value {
    // OTLP's own names (`string_value`, …).
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
        #[prost(message, tag = "5")]
        ArrayValue(super::ArrayValue),
        #[prost(message, tag = "6")]
        KvlistValue(super::KeyValueList),
        #[prost(bytes = "vec", tag = "7")]
        BytesValue(Vec<u8>),
    }
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ArrayValue {
    #[prost(message, repeated, tag = "1")]
    pub values: Vec<AnyValue>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct KeyValueList {
    #[prost(message, repeated, tag = "1")]
    pub values: Vec<KeyValue>,
}

/// The events of one Kafka record.
pub fn events(topic: Topic, record: &Record) -> Result<Vec<LogEvent>, String> {
    let value = record.value.as_deref().unwrap_or_default();
    match topic {
        Topic::Otlp => {
            let req = <ExportLogsServiceRequest as prost::Message>::decode(value).map_err(|e| format!("OTLP logs: {e}"))?;
            Ok(otlp_events(&req, record))
        }
        Topic::OtlpJson => {
            let body: Value = serde_json::from_slice(value).map_err(|e| format!("OTLP JSON logs: {e}"))?;
            Ok(otlp_events(&otlp_from_json(&body)?, record))
        }
        Topic::Json => json_events(value, record),
    }
}

fn header(record: &Record, name: &str) -> Option<String> {
    record.headers.get(name).map(|v| String::from_utf8_lossy(v).into_owned())
}

/// Vector's `kafka` source with the `json` codec: each object of the record is an event, its `timestamp` the
/// record's, its headers in `_oc_kafka_headers`.
fn json_events(value: &[u8], record: &Record) -> Result<Vec<LogEvent>, String> {
    let body: Value = serde_json::from_slice(value).map_err(|e| format!("JSON logs: {e}"))?;
    let logs = match body {
        Value::Array(items) => items,
        other => vec![other],
    };
    let headers: Map<String, Value> = record.headers.iter().map(|(k, v)| (k.clone(), Value::String(String::from_utf8_lossy(v).into_owned()))).collect();
    Ok(logs
        .into_iter()
        .filter_map(|log| match log {
            Value::Object(mut fields) => {
                fields.remove("timestamp");
                fields.insert("source_type".into(), "kafka".into());
                fields.insert("_oc_kafka_headers".into(), Value::Object(headers.clone()));
                Some(LogEvent { fields, timestamp: Some(record.timestamp) })
            }
            _ => None,
        })
        .collect())
}

fn any_value(v: &AnyValue) -> Value {
    use any_value::Value as V;
    match &v.value {
        Some(V::StringValue(s)) => Value::String(s.clone()),
        Some(V::BoolValue(b)) => Value::Bool(*b),
        Some(V::IntValue(i)) => Value::from(*i),
        Some(V::DoubleValue(d)) => serde_json::Number::from_f64(*d).map_or(Value::Null, Value::Number),
        Some(V::ArrayValue(a)) => Value::Array(a.values.iter().map(any_value).collect()),
        Some(V::KvlistValue(kv)) => Value::Object(key_values(&kv.values)),
        Some(V::BytesValue(b)) => Value::String(String::from_utf8_lossy(b).into_owned()),
        None => Value::Null,
    }
}

fn key_values(kvs: &[KeyValue]) -> Map<String, Value> {
    kvs.iter().map(|kv| (kv.key.clone(), kv.value.as_ref().map_or(Value::Null, any_value))).collect()
}

fn nanos(n: u64) -> Option<DateTime<Utc>> {
    (n > 0).then(|| DateTime::from_timestamp_nanos(n as i64))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The Collector's `resource/org` processor, then Vector's `opentelemetry` source: one event per log record.
fn otlp_events(req: &ExportLogsServiceRequest, record: &Record) -> Vec<LogEvent> {
    let org = header(record, "x-oc-org");
    let mut out = Vec::new();
    for rl in &req.resource_logs {
        let mut resources = rl.resource.as_ref().map(|r| key_values(&r.attributes)).unwrap_or_default();
        if resources.get("oc.signal").and_then(Value::as_str).is_some_and(|s| OTHER_SIGNALS.contains(&s)) {
            continue;
        }
        resources.remove("oc.org_id");
        if let Some(org) = &org {
            resources.insert("oc.org_id".into(), Value::String(org.clone()));
        }
        for sl in &rl.scope_logs {
            for log in &sl.log_records {
                let mut fields = Map::new();
                fields.insert("source_type".into(), "opentelemetry".into());
                if !resources.is_empty() {
                    fields.insert("resources".into(), Value::Object(resources.clone()));
                }
                if !log.attributes.is_empty() {
                    fields.insert("attributes".into(), Value::Object(key_values(&log.attributes)));
                }
                if let Some(body) = log.body.as_ref().filter(|b| b.value.is_some()) {
                    fields.insert("message".into(), any_value(body));
                }
                if !log.severity_text.is_empty() {
                    fields.insert("severity_text".into(), log.severity_text.clone().into());
                }
                fields.insert("severity_number".into(), log.severity_number.into());
                if !log.trace_id.is_empty() {
                    fields.insert("trace_id".into(), hex(&log.trace_id).into());
                }
                if !log.span_id.is_empty() {
                    fields.insert("span_id".into(), hex(&log.span_id).into());
                }
                if log.flags > 0 {
                    fields.insert("flags".into(), log.flags.into());
                }
                if let Some(scope) = sl.scope.as_ref().filter(|s| !s.name.is_empty() || !s.version.is_empty()) {
                    let mut m = Map::new();
                    if !scope.name.is_empty() {
                        m.insert("name".into(), scope.name.clone().into());
                    }
                    if !scope.version.is_empty() {
                        m.insert("version".into(), scope.version.clone().into());
                    }
                    fields.insert("scope".into(), Value::Object(m));
                }
                let timestamp = nanos(log.time_unix_nano).or_else(|| nanos(log.observed_time_unix_nano)).unwrap_or(record.timestamp);
                out.push(LogEvent { fields, timestamp: Some(timestamp) });
            }
        }
    }
    out
}

// ---------------------------------------------------------------- OTLP/JSON
// The protobuf JSON mapping: lowerCamelCase or the proto's names, 64-bit integers as strings or numbers,
// enums as numbers or names, trace and span ids as hex.

fn field<'a>(v: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    v.get(camel).or_else(|| v.get(snake)).filter(|v| !v.is_null())
}

fn list<'a>(v: &'a Value, camel: &str, snake: &str) -> &'a [Value] {
    field(v, camel, snake).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| u as i64)),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn uint(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n.as_u64().unwrap_or_default(),
        Some(Value::String(s)) => s.parse().unwrap_or_default(),
        _ => 0,
    }
}

fn severity(v: Option<&Value>) -> i32 {
    const NAMES: [&str; 6] = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL"];
    if let Some(n) = int(v) {
        return n as i32;
    }
    let Some(name) = v.and_then(Value::as_str).and_then(|s| s.strip_prefix("SEVERITY_NUMBER_")) else { return 0 };
    let (base, step) = match name.find(|c: char| c.is_ascii_digit()) {
        Some(i) => (&name[..i], name[i..].parse::<i32>().unwrap_or(1)),
        None => (name, 1),
    };
    NAMES.iter().position(|n| *n == base).map_or(0, |i| i as i32 * 4 + step)
}

fn ids(v: Option<&Value>) -> Vec<u8> {
    let s = v.and_then(Value::as_str).unwrap_or_default();
    if !s.is_ascii() || !s.len().is_multiple_of(2) {
        return Vec::new();
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16)).collect::<Result<_, _>>().unwrap_or_default()
}

fn any_from_json(v: &Value) -> AnyValue {
    use any_value::Value as V;
    let value = if let Some(s) = field(v, "stringValue", "string_value").and_then(Value::as_str) {
        Some(V::StringValue(s.to_string()))
    } else if let Some(b) = field(v, "boolValue", "bool_value").and_then(Value::as_bool) {
        Some(V::BoolValue(b))
    } else if let Some(i) = int(field(v, "intValue", "int_value")) {
        Some(V::IntValue(i))
    } else if let Some(d) = field(v, "doubleValue", "double_value").and_then(|d| d.as_f64().or_else(|| d.as_str()?.parse().ok())) {
        Some(V::DoubleValue(d))
    } else if let Some(a) = field(v, "arrayValue", "array_value") {
        Some(V::ArrayValue(ArrayValue { values: list(a, "values", "values").iter().map(any_from_json).collect() }))
    } else if let Some(kv) = field(v, "kvlistValue", "kvlist_value") {
        Some(V::KvlistValue(KeyValueList { values: kvs_from_json(list(kv, "values", "values")) }))
    } else {
        field(v, "bytesValue", "bytes_value").and_then(Value::as_str).and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok()).map(V::BytesValue)
    };
    AnyValue { value }
}

fn kvs_from_json(kvs: &[Value]) -> Vec<KeyValue> {
    kvs.iter().map(|kv| KeyValue { key: kv.get("key").and_then(Value::as_str).unwrap_or_default().to_string(), value: kv.get("value").map(any_from_json) }).collect()
}

fn otlp_from_json(body: &Value) -> Result<ExportLogsServiceRequest, String> {
    if !body.is_object() {
        return Err("OTLP JSON logs: not an object".into());
    }
    let resource_logs = list(body, "resourceLogs", "resource_logs")
        .iter()
        .map(|rl| ResourceLogs {
            resource: field(rl, "resource", "resource").map(|r| Resource { attributes: kvs_from_json(list(r, "attributes", "attributes")) }),
            scope_logs: list(rl, "scopeLogs", "scope_logs")
                .iter()
                .map(|sl| ScopeLogs {
                    scope: field(sl, "scope", "scope").map(|s| InstrumentationScope {
                        name: s.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                        version: s.get("version").and_then(Value::as_str).unwrap_or_default().to_string(),
                    }),
                    log_records: list(sl, "logRecords", "log_records")
                        .iter()
                        .map(|l| LogRecord {
                            time_unix_nano: uint(field(l, "timeUnixNano", "time_unix_nano")),
                            observed_time_unix_nano: uint(field(l, "observedTimeUnixNano", "observed_time_unix_nano")),
                            severity_number: severity(field(l, "severityNumber", "severity_number")),
                            severity_text: field(l, "severityText", "severity_text").and_then(Value::as_str).unwrap_or_default().to_string(),
                            body: field(l, "body", "body").map(any_from_json),
                            attributes: kvs_from_json(list(l, "attributes", "attributes")),
                            flags: uint(field(l, "flags", "flags")) as u32,
                            trace_id: ids(field(l, "traceId", "trace_id")),
                            span_id: ids(field(l, "spanId", "span_id")),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
    Ok(ExportLogsServiceRequest { resource_logs })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    use serde_json::json;

    fn record(value: Vec<u8>, org: Option<&str>) -> Record {
        let headers = org.map(|o| [("x-oc-org".to_string(), o.as_bytes().to_vec()), ("x-oc-retention-days".to_string(), b"15".to_vec())].into()).unwrap_or_default();
        Record { key: org.map(|o| o.as_bytes().to_vec()), value: Some(value), headers, timestamp: chrono::DateTime::parse_from_rfc3339("2026-11-05T10:00:00Z").unwrap().to_utc() }
    }

    fn kv(key: &str, value: any_value::Value) -> KeyValue {
        KeyValue { key: key.into(), value: Some(AnyValue { value: Some(value) }) }
    }

    fn s(v: &str) -> any_value::Value {
        any_value::Value::StringValue(v.into())
    }

    fn request() -> ExportLogsServiceRequest {
        let log = LogRecord {
            time_unix_nano: 1_793_872_800_123_000_000,
            severity_number: 17,
            severity_text: "ERROR".into(),
            body: Some(AnyValue { value: Some(s("payment declined")) }),
            attributes: vec![
                kv("oc.ingest_id", s("019a4000-0000-7000-8000-000000000001")),
                kv("order.id", any_value::Value::IntValue(42)),
                kv("retry", any_value::Value::BoolValue(true)),
                kv("ratio", any_value::Value::DoubleValue(0.5)),
                kv("items", any_value::Value::ArrayValue(ArrayValue { values: vec![AnyValue { value: Some(s("a")) }] })),
                kv("user", any_value::Value::KvlistValue(KeyValueList { values: vec![kv("id", s("u-1"))] })),
            ],
            flags: 1,
            trace_id: vec![0xab; 16],
            span_id: vec![0x0c; 8],
            observed_time_unix_nano: 0,
        };
        let resource = |attrs: Vec<KeyValue>| Some(Resource { attributes: attrs });
        ExportLogsServiceRequest {
            resource_logs: vec![
                ResourceLogs {
                    resource: resource(vec![kv("service.name", s("billing")), kv("oc.org_id", s("someone-else"))]),
                    scope_logs: vec![ScopeLogs { scope: Some(InstrumentationScope { name: "app.logger".into(), version: "1.0".into() }), log_records: vec![log] }],
                },
                ResourceLogs {
                    resource: resource(vec![kv("oc.signal", s("dd.events"))]),
                    scope_logs: vec![ScopeLogs { scope: None, log_records: vec![LogRecord::default()] }],
                },
            ],
        }
    }

    fn expected() -> Value {
        json!({
            "source_type": "opentelemetry",
            "resources": { "service.name": "billing", "oc.org_id": "acme" },
            "attributes": { "oc.ingest_id": "019a4000-0000-7000-8000-000000000001", "order.id": 42, "retry": true, "ratio": 0.5, "items": ["a"], "user": { "id": "u-1" } },
            "message": "payment declined",
            "severity_text": "ERROR",
            "severity_number": 17,
            "trace_id": "abababababababababababababababab",
            "span_id": "0c0c0c0c0c0c0c0c",
            "flags": 1,
            "scope": { "name": "app.logger", "version": "1.0" }
        })
    }

    #[test]
    fn otlp_records_become_vectors_events_with_the_headers_org() {
        let got = events(Topic::Otlp, &record(request().encode_to_vec(), Some("acme"))).unwrap();
        assert_eq!(got.len(), 1, "the dd.events resource is the Collector's");
        assert_eq!(Value::Object(got[0].fields.clone()), expected());
        assert_eq!(got[0].timestamp.unwrap().to_rfc3339(), "2026-11-05T10:00:00.123+00:00");
        let row = crate::log_normalize::normalize(got[0].clone(), chrono::Utc::now()).unwrap();
        assert_eq!((row.org_id.as_str(), row.service.as_str(), row.status.as_str(), row.id.as_str()), ("acme", "billing", "error", "019a4000-0000-7000-8000-000000000001"));
    }

    #[test]
    fn otlp_json_records_are_the_same_events() {
        let body = json!({ "resourceLogs": [
            { "resource": { "attributes": [ { "key": "service.name", "value": { "stringValue": "billing" } }, { "key": "oc.org_id", "value": { "stringValue": "someone-else" } } ] },
              "scopeLogs": [ { "scope": { "name": "app.logger", "version": "1.0" }, "logRecords": [ {
                "timeUnixNano": "1793872800123000000", "severityNumber": "SEVERITY_NUMBER_ERROR", "severityText": "ERROR",
                "body": { "stringValue": "payment declined" }, "flags": 1,
                "traceId": "ABABABABABABABABABABABABABABABAB", "spanId": "0c0c0c0c0c0c0c0c",
                "attributes": [
                  { "key": "oc.ingest_id", "value": { "stringValue": "019a4000-0000-7000-8000-000000000001" } },
                  { "key": "order.id", "value": { "intValue": "42" } },
                  { "key": "retry", "value": { "boolValue": true } },
                  { "key": "ratio", "value": { "doubleValue": 0.5 } },
                  { "key": "items", "value": { "arrayValue": { "values": [ { "stringValue": "a" } ] } } },
                  { "key": "user", "value": { "kvlistValue": { "values": [ { "key": "id", "value": { "stringValue": "u-1" } } ] } } }
                ] } ] } ] },
            { "resource": { "attributes": [ { "key": "oc.signal", "value": { "stringValue": "dd.events" } } ] }, "scopeLogs": [ { "logRecords": [ {} ] } ] }
        ] });
        let got = events(Topic::OtlpJson, &record(body.to_string().into_bytes(), Some("acme"))).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(Value::Object(got[0].fields.clone()), expected());
        assert_eq!(got[0].timestamp.unwrap().to_rfc3339(), "2026-11-05T10:00:00.123+00:00");
    }

    #[test]
    fn otlp_without_an_org_header_has_no_org() {
        let got = events(Topic::Otlp, &record(request().encode_to_vec(), None)).unwrap();
        assert_eq!(got[0].fields["resources"], json!({ "service.name": "billing" }));
        assert!(crate::log_normalize::normalize(got[0].clone(), chrono::Utc::now()).is_none());
    }

    #[test]
    fn otlp_logs_without_a_time_take_the_observed_time_then_the_records() {
        let mut req = ExportLogsServiceRequest { resource_logs: vec![ResourceLogs { resource: None, scope_logs: vec![ScopeLogs { scope: None, log_records: vec![LogRecord::default(), LogRecord::default()] }] }] };
        req.resource_logs[0].scope_logs[0].log_records[0].observed_time_unix_nano = 1_793_871_000_000_000_000;
        let got = events(Topic::Otlp, &record(req.encode_to_vec(), Some("acme"))).unwrap();
        assert_eq!(got[0].timestamp.unwrap().to_rfc3339(), "2026-11-05T09:30:00+00:00");
        assert_eq!(got[1].timestamp.unwrap().to_rfc3339(), "2026-11-05T10:00:00+00:00");
        assert_eq!(Value::Object(got[1].fields.clone()), json!({ "source_type": "opentelemetry", "resources": { "oc.org_id": "acme" }, "severity_number": 0 }));
    }

    #[test]
    fn malformed_otlp_json_ids_are_dropped() {
        assert_eq!(ids(Some(&json!("ééééééééééééééé"))), Vec::<u8>::new());
        assert_eq!(ids(Some(&json!("abc"))), Vec::<u8>::new());
        assert_eq!(ids(Some(&json!("zz"))), Vec::<u8>::new());
        assert_eq!(ids(Some(&json!("0aFF"))), vec![0x0a, 0xff]);
    }

    #[test]
    fn json_records_are_one_event_per_log_with_the_kafka_headers() {
        let body = json!([{ "message": "a", "timestamp": "2020-01-01T00:00:00Z", "_oc_ingest": { "id": "x" } }, { "message": "b" }, 7]);
        let got = events(Topic::Json, &record(body.to_string().into_bytes(), Some("acme"))).unwrap();
        assert_eq!(got.len(), 2, "only objects are logs");
        assert_eq!(
            Value::Object(got[0].fields.clone()),
            json!({ "message": "a", "_oc_ingest": { "id": "x" }, "source_type": "kafka", "_oc_kafka_headers": { "x-oc-org": "acme", "x-oc-retention-days": "15" } })
        );
        assert_eq!(got[0].timestamp.unwrap().to_rfc3339(), "2026-11-05T10:00:00+00:00", "the record's time replaces the log's, as Vector's kafka source does");
        let one = events(Topic::Json, &record(br#"{"message":"c"}"#.to_vec(), Some("acme"))).unwrap();
        assert_eq!(one[0].fields["message"], "c");
        assert!(events(Topic::Json, &record(b"not json".to_vec(), Some("acme"))).is_err());
    }
}
