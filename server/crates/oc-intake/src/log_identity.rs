//! JSON log identities are assigned before Kafka, once per accepted event, not on consumption.
//! Equal log lines remain distinct; a Kafka replay retains the serialized identity and fallback time.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

fn stamp(event: &mut Map<String, Value>, received_at: DateTime<Utc>) {
    // UUIDv7: the acceptance time followed by random bits, with the version and variant set.
    let mut bits = ((received_at.timestamp_millis() as u128 & 0xffff_ffff_ffff) << 80) | (rand::random::<u128>() & 0xffff_ffff_ffff_ffff_ffff);
    bits = (bits & !(0xf << 76)) | (7 << 76);
    bits = (bits & !(3 << 62)) | (2 << 62);
    let hex = format!("{bits:032x}");
    let id = format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]);
    // This field is reserved: callers cannot choose another accepted event's identity.
    event.insert("_oc_ingest".into(), serde_json::json!({"id": id, "received_at": received_at.to_rfc3339()}));
}

pub fn prepare(body: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut value: Value = serde_json::from_slice(body).map_err(|_| "invalid JSON log body")?;
    let received_at = Utc::now();
    match &mut value {
        Value::Object(event) => stamp(event, received_at),
        Value::Array(events) => {
            for event in events {
                stamp(event.as_object_mut().ok_or("JSON logs must be an object or an array of objects")?, received_at);
            }
        }
        _ => return Err("JSON logs must be an object or an array of objects"),
    }
    serde_json::to_vec(&value).map_err(|_| "cannot encode JSON logs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{send, test_state, Config, KeyOrg, Rejection, Sink};
    use axum::{
        body::Bytes,
        http::{header, HeaderMap, HeaderValue, StatusCode},
        routing::post,
        Router,
    };

    #[tokio::test]
    async fn json_logs_are_stamped_after_decompression_on_the_http_path() {
        use std::io::Write;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = Config::from_env();
        cfg.logs_url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/",
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
        let st = test_state(cfg);
        let org = KeyOrg {
            org_id: "example-org".into(),
            span_retention_days: 15,
            metrics_retention_days: 30,
            metrics_5m_days: 90,
            metrics_1h_days: 395,
            flag_environment_id: None,
        };
        let input = br#"[{"message":"same"},{"message":"same","_oc_ingest":{"id":"forged"}}]"#;
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(input).unwrap();
        let bodies = [
            (None, input.to_vec()),
            (Some(HeaderValue::from_static("gzip")), gzip.finish().unwrap()),
            (Some(HeaderValue::from_static("zstd")), zstd::stream::encode_all(&input[..], 1).unwrap()),
        ];
        for (encoding, body) in bodies {
            let result = send(&st, &org, Sink::JsonLogs, Some(&HeaderValue::from_static("application/json")), encoding.as_ref(), body.into())
                .await
                .unwrap_or_else(|r| panic!("{}", r.message));
            assert_eq!(result.status, StatusCode::OK);
            let events: serde_json::Value = serde_json::from_slice(&result.body).unwrap();
            assert_eq!(events[0]["message"], "same");
            assert_ne!(events[0]["_oc_ingest"]["id"], events[1]["_oc_ingest"]["id"]);
            assert_ne!(events[1]["_oc_ingest"]["id"], "forged");
        }
        let invalid = send(&st, &org, Sink::JsonLogs, None, None, Bytes::from_static(b"[{},null]")).await;
        assert!(matches!(invalid, Err(Rejection { status: StatusCode::BAD_REQUEST, .. })));
        task.abort();
    }

    #[test]
    fn equal_events_and_separate_requests_have_distinct_ids() {
        let a: Value = serde_json::from_slice(&prepare(br#"[{"message":"same"},{"message":"same"}]"#).unwrap()).unwrap();
        let b: Value = serde_json::from_slice(&prepare(br#"{"message":"same"}"#).unwrap()).unwrap();
        assert_ne!(a[0]["_oc_ingest"]["id"], a[1]["_oc_ingest"]["id"]);
        assert_ne!(a[0]["_oc_ingest"]["id"], b["_oc_ingest"]["id"]);
        assert_eq!(a[0]["_oc_ingest"]["received_at"], a[1]["_oc_ingest"]["received_at"]);
        let id = a[0]["_oc_ingest"]["id"].as_str().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "7");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert!(DateTime::parse_from_rfc3339(a[0]["_oc_ingest"]["received_at"].as_str().unwrap()).is_ok());
    }

    #[test]
    fn reserved_metadata_is_replaced_and_user_fields_survive() {
        let a: Value = serde_json::from_slice(&prepare(br#"{"_oc_ingest":{"id":"forged"},"message":"ok","timestamp":"2026-10-01T00:00:00Z","attributes":{"id":"user-id"}}"#).unwrap()).unwrap();
        assert_ne!(a["_oc_ingest"]["id"], "forged");
        assert_eq!(a["message"], "ok");
        assert_eq!(a["timestamp"], "2026-10-01T00:00:00Z");
        assert_eq!(a["attributes"]["id"], "user-id");
    }

    #[test]
    fn reject_poison_shapes_before_kafka_accepts_them() {
        for body in ["null", "1", "\"text\"", "[{} , false]", "{broken"] {
            assert!(prepare(body.as_bytes()).is_err(), "{body}");
        }
        assert_eq!(prepare(b"[]").unwrap(), b"[]");
    }
}
