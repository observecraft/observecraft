//! Stamp OTLP logs before Kafka without decoding/re-encoding the sender's full schema.
//! Wire field numbers follow OpenTelemetry's logs/v1/logs.proto (Apache-2.0).
//! Untouched protobuf fields, including future extensions, survive byte-for-byte.

use chrono::{DateTime, Utc};
use prost::{
    encoding::{self, DecodeContext, WireType},
    Message,
};
use serde_json::Value;

const ID: &str = "oc.ingest_id";
const RECEIVED: &str = "oc.ingest_received_at";
const INVALID: &str = "invalid OTLP logs body";
pub const TOO_LARGE: &str = "OTLP logs exceed body limit after adding ingestion metadata; send smaller batches";

#[cfg(test)]
pub fn prepare(body: &[u8], json: bool) -> Result<Vec<u8>, &'static str> {
    prepare_limited(body, json, 16 * 1024 * 1024)
}

pub fn prepare_limited(body: &[u8], json: bool, limit: usize) -> Result<Vec<u8>, &'static str> {
    let at = Utc::now();
    let mut budget = limit.checked_sub(body.len()).ok_or(TOO_LARGE)?;
    let result = if json {
        prepare_json(body, at, &mut budget)
    } else {
        rewrite_budgeted(body, 0, at, &mut budget).map_err(|e| match e {
            Error::Invalid => INVALID,
            Error::TooLarge => TOO_LARGE,
        })
    }?;
    if result.len() > limit {
        return Err(TOO_LARGE);
    }
    Ok(result)
}

#[derive(Debug)]
enum Error {
    Invalid,
    TooLarge,
}
impl From<prost::DecodeError> for Error {
    fn from(_: prost::DecodeError) -> Self {
        Self::Invalid
    }
}

// A tiny protobuf event can expand greatly after stamping. Bound metadata growth while traversing,
// before allocating the output. 160 bytes covers the envelope and ancestor length-prefix growth.
fn reserve(budget: &mut usize) -> Result<(), Error> {
    *budget = budget.checked_sub(160).ok_or(Error::TooLarge)?;
    Ok(())
}

fn array<'a>(value: &'a mut Value, key: &str) -> Result<&'a mut Vec<Value>, &'static str> {
    // Accept protobuf's original names as well as JSON names, but reject ambiguous duplicates.
    let alias = match key {
        "resourceLogs" => "resource_logs",
        "scopeLogs" => "scope_logs",
        "logRecords" => "log_records",
        _ => key,
    };
    canonicalize(value, key, alias)?;
    let obj = value.as_object_mut().ok_or(INVALID)?;
    let entry = obj.entry(key).or_insert_with(|| Value::Array(Vec::new()));
    if entry.is_null() {
        *entry = Value::Array(Vec::new());
    }
    entry.as_array_mut().ok_or(INVALID)
}

fn canonicalize(value: &mut Value, key: &str, alias: &str) -> Result<(), &'static str> {
    let obj = value.as_object_mut().ok_or(INVALID)?;
    if alias != key && obj.contains_key(alias) {
        if obj.contains_key(key) {
            return Err(INVALID);
        }
        let value = obj.remove(alias).unwrap();
        obj.insert(key.into(), value);
    }
    Ok(())
}

fn prepare_json(body: &[u8], at: DateTime<Utc>, budget: &mut usize) -> Result<Vec<u8>, &'static str> {
    let mut value: Value = serde_json::from_slice(body).map_err(|_| INVALID)?;
    for resource in array(&mut value, "resourceLogs")? {
        for scope in array(resource, "scopeLogs")? {
            for record in array(scope, "logRecords")? {
                reserve(budget).map_err(|_| TOO_LARGE)?;
                canonicalize(record, "timeUnixNano", "time_unix_nano")?;
                canonicalize(record, "observedTimeUnixNano", "observed_time_unix_nano")?;
                let attrs = array(record, "attributes")?;
                attrs.retain(|a| !matches!(a.get("key").and_then(Value::as_str), Some(ID | RECEIVED)));
                for (key, text) in [(ID, crate::log_identity::new_id(at)), (RECEIVED, at.to_rfc3339())] {
                    attrs.push(serde_json::json!({"key": key, "value": {"stringValue": text}}));
                }
                let nanos = |v: Option<&Value>| -> Result<u64, &'static str> {
                    match v {
                        None | Some(Value::Null) => Ok(0),
                        Some(Value::String(s)) => s.parse().map_err(|_| INVALID),
                        Some(Value::Number(n)) => n.as_u64().ok_or(INVALID),
                        _ => Err(INVALID),
                    }
                };
                if nanos(record.get("timeUnixNano"))? == 0 {
                    let observed = nanos(record.get("observedTimeUnixNano"))?;
                    record["timeUnixNano"] = fallback(observed, at).to_string().into();
                }
            }
        }
    }
    serde_json::to_vec(&value).map_err(|_| INVALID)
}

fn fallback(observed: u64, at: DateTime<Utc>) -> u64 {
    if observed != 0 {
        observed
    } else {
        at.timestamp_nanos_opt().expect("current time fits nanoseconds") as u64
    }
}

fn put_message(tag: u32, payload: &[u8], output: &mut Vec<u8>) {
    encoding::encode_key(tag, WireType::LengthDelimited, output);
    encoding::encode_varint(payload.len() as u64, output);
    output.extend_from_slice(payload);
}

// Only decode attribute keys. Values may contain arrays/maps unsupported by our conversion model.
#[derive(Clone, PartialEq, Message)]
struct AttributeKey {
    #[prost(string, tag = "1")]
    key: String,
}

#[cfg(test)]
fn rewrite(input: &[u8], level: u8, at: DateTime<Utc>) -> Result<Vec<u8>, Error> {
    rewrite_budgeted(input, level, at, &mut (16 * 1024 * 1024))
}

fn rewrite_budgeted(mut input: &[u8], level: u8, at: DateTime<Utc>, budget: &mut usize) -> Result<Vec<u8>, Error> {
    if level == 3 {
        reserve(budget)?;
    }
    let mut output = Vec::with_capacity(input.len());
    let (mut time, mut observed) = (0, 0);
    while !input.is_empty() {
        let raw = input;
        let (tag, wire) = encoding::decode_key(&mut input)?;
        let data = input;
        encoding::skip_field(wire, tag, &mut input, DecodeContext::default())?;
        let raw = &raw[..raw.len() - input.len()];
        let child = (level == 0 && tag == 1) || ((level == 1 || level == 2) && tag == 2);
        if child || (level == 3 && tag == 6) {
            if wire != WireType::LengthDelimited {
                return Err(Error::Invalid);
            }
            let mut payload = data;
            let len = encoding::decode_varint(&mut payload)? as usize;
            let payload = payload.get(..len).ok_or_else(|| prost::DecodeError::new("truncated message"))?;
            if child {
                put_message(tag, &rewrite_budgeted(payload, level + 1, at, budget)?, &mut output);
                continue;
            }
            if matches!(AttributeKey::decode(payload)?.key.as_str(), ID | RECEIVED) {
                continue;
            }
        }
        if level == 3 && (tag == 1 || tag == 11) {
            if wire != WireType::SixtyFourBit {
                return Err(Error::Invalid);
            }
            let value = u64::from_le_bytes(data[..8].try_into().unwrap());
            if tag == 1 {
                time = value;
            } else {
                observed = value;
            }
        }
        output.extend_from_slice(raw);
    }
    if level == 3 {
        if time == 0 {
            encoding::encode_key(1, WireType::SixtyFourBit, &mut output);
            output.extend_from_slice(&fallback(observed, at).to_le_bytes());
        }
        for (key, text) in [(ID, crate::log_identity::new_id(at)), (RECEIVED, at.to_rfc3339())] {
            put_message(6, &crate::otlp::kv(key, text).encode_to_vec(), &mut output);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otlp::*;

    fn request(records: Vec<LogRecord>) -> Vec<u8> {
        ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs { scope: None, log_records: records }],
            }],
        }
        .encode_to_vec()
    }

    fn records(body: &[u8]) -> Vec<LogRecord> {
        ExportLogsServiceRequest::decode(body).unwrap().resource_logs.remove(0).scope_logs.remove(0).log_records
    }

    #[test]
    fn protobuf_equal_events_get_distinct_server_ids_and_stable_fallback_times() {
        let record = LogRecord {
            attributes: vec![kv(ID, "forged"), kv(ID, "also forged"), kv(RECEIVED, "forged"), kv("order.id", "42")],
            ..Default::default()
        };
        let body = request(vec![record.clone(), record]);
        let prepared = prepare(&body, false).unwrap();
        let events = records(&prepared);
        let id = |r: &LogRecord| text(r.attributes.iter().find(|a| a.key == ID).unwrap()).to_owned();
        assert_ne!(id(&events[0]), id(&events[1]));
        assert_ne!(id(&events[0]), "forged");
        assert_eq!(events[0].attributes.iter().filter(|a| a.key == ID).count(), 1);
        assert_eq!(events[0].time_unix_nano, events[1].time_unix_nano);
        assert_ne!(events[0].time_unix_nano, 0);
        assert_eq!(text(events[0].attributes.iter().find(|a| a.key == "order.id").unwrap()), "42");
        // The serialized request is Kafka's replay unit, never stamped again by a consumer.
        assert_eq!(records(&prepared), events);
        assert_ne!(id(&records(&prepare(&body, false).unwrap())[0]), id(&events[0]));
    }

    #[test]
    fn protobuf_preserves_fields_outside_the_intakes_partial_otlp_model() {
        let at = Utc::now();
        let mut log = LogRecord {
            time_unix_nano: 123,
            observed_time_unix_nano: 456,
            ..Default::default()
        }
        .encode_to_vec();
        put_message(9, &[3; 16], &mut log); // trace_id, missing from our partial conversion model
        put_message(10, &[4; 8], &mut log); // span_id
        put_message(12, b"order.created", &mut log); // event_name
        put_message(101, b"future extension", &mut log);
        // An array-valued attribute, also outside our conversion model.
        let mut attr = Vec::new();
        put_message(1, b"order.items", &mut attr);
        let mut array_value = Vec::new();
        put_message(5, &[10, 2, 16, 1], &mut array_value);
        put_message(2, &array_value, &mut attr);
        put_message(6, &attr, &mut log);
        let mut scope = Vec::new();
        put_message(3, b"https://example.com/scope-schema", &mut scope);
        let scope_prefix = scope.clone();
        put_message(2, &log, &mut scope);
        let mut resource = Vec::new();
        put_message(3, b"https://example.com/resource-schema", &mut resource);
        let resource_prefix = resource.clone();
        put_message(2, &scope, &mut resource);
        let mut body = Vec::new();
        put_message(100, b"future request extension", &mut body);
        let prefix = body.clone();
        put_message(1, &resource, &mut body);
        let rewritten_log = rewrite(&log, 3, at).unwrap();
        assert!(rewritten_log.starts_with(&log));
        assert_eq!(records(&rewrite(&body, 0, at).unwrap())[0].time_unix_nano, 123);
        let result = rewrite(&body, 0, at).unwrap();
        for preserved in [prefix, scope_prefix, resource_prefix, log] {
            assert!(result.windows(preserved.len()).any(|w| w == preserved));
        }
    }

    #[test]
    fn missing_event_time_uses_observed_time_before_acceptance_time() {
        let body = request(vec![LogRecord {
            observed_time_unix_nano: 123456,
            ..Default::default()
        }]);
        let record = records(&prepare(&body, false).unwrap()).remove(0);
        assert_eq!(record.time_unix_nano, 123456);
        assert_eq!(record.observed_time_unix_nano, 123456);
    }

    #[test]
    fn json_stamps_each_record_replaces_reserved_attributes_and_preserves_otel_fields() {
        let input = serde_json::json!({"resourceLogs": [{"schemaUrl": "resource-schema", "scopeLogs": [{"schemaUrl": "scope-schema", "logRecords": [
            {"traceId": "0123456789abcdef0123456789abcdef", "spanId": "0123456789abcdef", "flags": 1, "timeUnixNano": "123", "body": {"kvlistValue": {"values": []}}, "attributes": [{"key": ID, "value": {"stringValue": "forged"}}]},
            {"observedTimeUnixNano": "456"}, {}, {}
        ]}]}]});
        let result: Value = serde_json::from_slice(&prepare(&serde_json::to_vec(&input).unwrap(), true).unwrap()).unwrap();
        let scope = &result["resourceLogs"][0]["scopeLogs"][0];
        let logs = scope["logRecords"].as_array().unwrap();
        assert_eq!(scope["schemaUrl"], "scope-schema");
        for key in ["traceId", "spanId", "flags", "timeUnixNano", "body"] {
            assert_eq!(logs[0][key], input["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0][key]);
        }
        assert_ne!(logs[0]["attributes"][0]["value"]["stringValue"], "forged");
        assert_eq!(logs[1]["timeUnixNano"], "456");
        assert_eq!(logs[2]["timeUnixNano"], logs[3]["timeUnixNano"]);
        assert_ne!(logs[2]["attributes"][0], logs[3]["attributes"][0]);
    }

    #[test]
    fn rejects_truncation_and_invalid_container_shapes_before_kafka() {
        for input in [&[10, 5, 1][..], &[10, 2, 18, 5], &[0], &[15]] {
            assert!(prepare(input, false).is_err());
        }
        for input in ["null", "[]", "{\"resourceLogs\":false}", "{\"resourceLogs\":[{\"scopeLogs\":[{\"logRecords\":[null]}]}]}"] {
            assert!(prepare(input.as_bytes(), true).is_err());
        }
        assert!(prepare(b"{}", true).is_ok());
        assert!(prepare(b"", false).is_ok());
    }

    #[test]
    fn tiny_records_cannot_expand_past_the_intake_body_budget() {
        let body = request(vec![LogRecord::default(); 10]);
        assert_eq!(prepare_limited(&body, false, 1000).unwrap_err(), TOO_LARGE);
        assert!(prepare_limited(&body, false, 4096).unwrap().len() <= 4096);
        let json = br#"{"resourceLogs":[{"scopeLogs":[{"logRecords":[{},{},{},{},{},{},{},{},{},{}]}]}]}"#;
        assert_eq!(prepare_limited(json, true, 1000).unwrap_err(), TOO_LARGE);
        assert!(prepare_limited(json, true, 4096).unwrap().len() <= 4096);
    }

    #[test]
    fn protobuf_json_field_aliases_cannot_bypass_stamping_or_make_timestamps_ambiguous() {
        let input = br#"{"resource_logs":[{"scope_logs":[{"log_records":[{"observed_time_unix_nano":"456"}]}]}]}"#;
        let result: Value = serde_json::from_slice(&prepare(input, true).unwrap()).unwrap();
        let record = &result["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0];
        assert_eq!(record["timeUnixNano"], "456");
        assert_eq!(record["observedTimeUnixNano"], "456");
        assert_eq!(record["attributes"].as_array().unwrap().len(), 2);
        assert!(prepare(br#"{"resourceLogs":null}"#, true).is_ok());
        assert!(prepare(br#"{"resourceLogs":[{"scopeLogs":[{"logRecords":[{"attributes":null}]}]}]}"#, true).is_ok());
        assert!(prepare(br#"{"resourceLogs":[],"resource_logs":[]}"#, true).is_err());
        assert!(prepare(br#"{"resourceLogs":[{"scopeLogs":[{"logRecords":[{"timeUnixNano":"1","time_unix_nano":"2"}]}]}]}"#, true).is_err());
    }

    #[tokio::test]
    async fn http_logs_are_stamped_after_decompression_for_json_and_protobuf() {
        use axum::{
            body::Bytes,
            http::{header, HeaderMap, HeaderValue, StatusCode},
            routing::post,
            Router,
        };
        use std::io::Write;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = crate::Config::from_env();
        cfg.collector_url = format!("http://{}", listener.local_addr().unwrap());
        cfg.max_body = 1024;
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/v1/logs",
                    post(|headers: HeaderMap, body: Bytes| async move {
                        assert!(!headers.contains_key(header::CONTENT_ENCODING));
                        assert_eq!(headers["x-oc-org"], "example-org");
                        body
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let st = crate::test_state(cfg);
        let org = crate::KeyOrg {
            org_id: "example-org".into(),
            span_retention_days: 15,
            metrics_retention_days: 30,
            metrics_5m_days: 90,
            metrics_1h_days: 395,
            flag_environment_id: None,
        };
        for json in [false, true] {
            let input = if json {
                br#"{"resourceLogs":[{"scopeLogs":[{"logRecords":[{},{}]}]}]}"#.to_vec()
            } else {
                request(vec![LogRecord::default(), LogRecord::default()])
            };
            let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gzip.write_all(&input).unwrap();
            for (encoding, bytes) in [
                (None, input.clone()),
                (Some(HeaderValue::from_static("gzip")), gzip.finish().unwrap()),
                (Some(HeaderValue::from_static("zstd")), zstd::stream::encode_all(&input[..], 1).unwrap()),
            ] {
                let content = HeaderValue::from_static(if json { "application/json" } else { "application/x-protobuf" });
                let result = crate::send(&st, &org, crate::Sink::Logs, Some(&content), encoding.as_ref(), bytes.into())
                    .await
                    .unwrap_or_else(|r| panic!("{}", r.message));
                assert_eq!(result.status, StatusCode::OK);
                if json {
                    let logs: Value = serde_json::from_slice(&result.body).unwrap();
                    assert_ne!(
                        logs["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["attributes"][0],
                        logs["resourceLogs"][0]["scopeLogs"][0]["logRecords"][1]["attributes"][0]
                    );
                } else {
                    let events = records(&result.body);
                    assert_ne!(events[0].attributes[0], events[1].attributes[0]);
                    assert_ne!(events[0].time_unix_nano, 0);
                }
            }
        }
        let oversized = crate::send(&st, &org, crate::Sink::Logs, None, None, request(vec![LogRecord::default(); 10]).into()).await;
        assert!(matches!(
            oversized,
            Err(crate::Rejection {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                ..
            })
        ));
        let invalid = crate::send(&st, &org, crate::Sink::Logs, None, None, Bytes::from_static(&[10, 5, 1])).await;
        assert!(matches!(invalid, Err(crate::Rejection { status: StatusCode::BAD_REQUEST, .. })));
        task.abort();
    }
}
