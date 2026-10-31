//! Session Replay: the recording segments of Datadog's browser SDK (`POST /api/v2/replay`).
//!
//! The SDK posts a multipart form whose `segment` part is a deflate-compressed JSON segment: the
//! application, session and view, its start and end (epoch ms) and its `records` (full snapshots of
//! the DOM, incremental changes, mouse and input events), with the application's client token in the
//! query string (`dd-api-key=pub…`, or inside `?ddforward=` behind the SDK's proxy option), like RUM
//! events ([`crate::datadog_rum`]).
//!
//! Each segment becomes one OTLP log record with resource `oc.signal = dd.replay`, which the collector
//! routes to `otel.otel_rum_replay`: Body = the segment's JSON, LogAttributes = application.id,
//! session.id, view.id, replay.start / replay.end, replay.index_in_view, replay.has_full_snapshot,
//! replay.records_count and replay.source.

use std::io::Read;

use axum::extract::{Multipart, RawQuery, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::datadog_rum::{authenticate, params};
use crate::otlp;
use crate::{send, Rejection, Shared};

/// Largest segment kept, decompressed.
const MAX_SEGMENT: u64 = 32 << 20;

/// A segment's bytes as JSON: zlib or raw deflate (the SDK's `CompressionStream('deflate')`), or plain.
pub fn decode(raw: &[u8]) -> Result<Value, String> {
    let mut out = Vec::new();
    if flate2::read::ZlibDecoder::new(raw).take(MAX_SEGMENT).read_to_end(&mut out).is_err() {
        out.clear();
        if flate2::read::DeflateDecoder::new(raw).take(MAX_SEGMENT).read_to_end(&mut out).is_err() {
            out = raw.to_vec();
        }
    }
    serde_json::from_slice(&out).map_err(|e| format!("invalid replay segment: {e}"))
}

/// The OTLP log record of one segment.
pub fn to_record(seg: &Value) -> Option<otlp::LogRecord> {
    let id = |k: &str| seg.get(k).and_then(|v| v.get("id")).and_then(Value::as_str).map(str::to_string);
    let (app, session, view) = (id("application")?, id("session")?, id("view")?);
    let num = |k: &str| seg.get(k).and_then(Value::as_i64);
    let start = num("start").or_else(|| seg.get("records")?.as_array()?.first()?.get("timestamp")?.as_i64())?;
    let end = num("end").unwrap_or(start);
    let records = seg.get("records").and_then(Value::as_array).map_or(0, Vec::len);
    let full = seg.get("has_full_snapshot").and_then(Value::as_bool).unwrap_or_else(|| seg.get("records").and_then(Value::as_array).is_some_and(|r| r.iter().any(|x| x.get("type").and_then(Value::as_i64) == Some(2))));
    let mut attributes = vec![
        otlp::kv("application.id", app),
        otlp::kv("session.id", session),
        otlp::kv("view.id", view),
        otlp::kv("replay.start", start.to_string()),
        otlp::kv("replay.end", end.to_string()),
        otlp::kv("replay.has_full_snapshot", full.to_string()),
        otlp::kv("replay.records_count", records.to_string()),
        otlp::kv("replay.source", seg.get("source").and_then(Value::as_str).unwrap_or("browser")),
    ];
    if let Some(i) = num("index_in_view") {
        attributes.push(otlp::kv("replay.index_in_view", i.to_string()));
    }
    let t = (start.max(0) as u64) * 1_000_000;
    Some(otlp::LogRecord {
        time_unix_nano: t,
        observed_time_unix_nano: t,
        severity_number: 9,
        severity_text: "info".into(),
        body: Some(otlp::AnyValue { value: Some(otlp::any_value::Value::StringValue(seg.to_string())) }),
        attributes,
    })
}

pub fn to_otlp(records: Vec<otlp::LogRecord>) -> otlp::ExportLogsServiceRequest {
    otlp::ExportLogsServiceRequest {
        resource_logs: vec![otlp::ResourceLogs {
            resource: Some(otlp::Resource { attributes: vec![otlp::kv("oc.signal", "dd.replay")] }),
            scope_logs: vec![otlp::ScopeLogs { scope: otlp::scope("datadog.replay"), log_records: records }],
        }],
    }
}

/// `POST /api/v2/replay`: one or more recording segments.
pub async fn post(State(st): State<Shared>, RawQuery(query): RawQuery, headers: HeaderMap, mut form: Multipart) -> Response {
    let q = params(query.as_deref().unwrap_or(""));
    let length = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
    let app = match authenticate(&st, &headers, &q, length).await {
        Ok(app) => app,
        Err(r) => return r.into_response(),
    };
    let mut records = Vec::new();
    loop {
        let field = match form.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return (StatusCode::BAD_REQUEST, format!("invalid multipart body: {e}")).into_response(),
        };
        if field.name() != Some("segment") {
            continue;
        }
        let bytes = match field.bytes().await {
            Ok(b) => b,
            Err(e) => return (StatusCode::BAD_REQUEST, format!("invalid segment: {e}")).into_response(),
        };
        match decode(&bytes).map(|seg| to_record(&seg)) {
            Ok(Some(r)) => records.push(r),
            Ok(None) => return (StatusCode::BAD_REQUEST, "the segment names no application, session or view").into_response(),
            Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
        }
    }
    if !app.keep || records.is_empty() {
        return accepted();
    }
    match send(&st, &app.org, crate::kafka::Sink::Logs, Some(&HeaderValue::from_static("application/x-protobuf")), None, otlp::encode(&to_otlp(records))).await {
        Ok(up) if up.status.is_success() => accepted(),
        Ok(up) => {
            tracing::warn!(status = %up.status, "collector refused replay segments");
            Rejection { status: StatusCode::SERVICE_UNAVAILABLE, message: "segments were not accepted" }.into_response()
        }
        Err(r) => r.into_response(),
    }
}

fn accepted() -> Response {
    (StatusCode::ACCEPTED, [(header::CONTENT_TYPE, "application/json")], "{}").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn segment() -> Value {
        json!({"application": {"id": "app-1"}, "session": {"id": "s-1"}, "view": {"id": "v-1"}, "start": 1_700_000_000_000i64, "end": 1_700_000_005_000i64,
               "index_in_view": 0, "source": "browser", "records": [{"type": 4, "timestamp": 1_700_000_000_000i64, "data": {"href": "https://shop.example.com/", "width": 1280, "height": 800}},
               {"type": 2, "timestamp": 1_700_000_000_001i64, "data": {"node": {"type": 0, "id": 0, "childNodes": []}, "initialOffset": {"top": 0, "left": 0}}}]})
    }

    #[test]
    fn segments_decode_compressed_or_plain() {
        let raw = segment().to_string();
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(raw.as_bytes()).unwrap();
        assert_eq!(decode(&z.finish().unwrap()).unwrap(), segment());
        let mut d = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        d.write_all(raw.as_bytes()).unwrap();
        assert_eq!(decode(&d.finish().unwrap()).unwrap(), segment());
        assert_eq!(decode(raw.as_bytes()).unwrap(), segment());
        assert!(decode(b"garbage").is_err());
    }

    #[test]
    fn a_segment_becomes_a_log_record_routed_to_replay() {
        let r = to_record(&segment()).unwrap();
        assert_eq!(r.time_unix_nano, 1_700_000_000_000_000_000);
        for kv in [otlp::kv("session.id", "s-1"), otlp::kv("view.id", "v-1"), otlp::kv("replay.has_full_snapshot", "true"), otlp::kv("replay.records_count", "2"), otlp::kv("replay.end", "1700000005000")] {
            assert!(r.attributes.contains(&kv), "{kv:?}");
        }
        assert!(to_otlp(vec![r]).resource_logs[0].resource.as_ref().unwrap().attributes.contains(&otlp::kv("oc.signal", "dd.replay")));
        assert!(to_record(&json!({"session": {"id": "s"}})).is_none());
    }
}
