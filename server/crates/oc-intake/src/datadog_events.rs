//! Datadog events: the Agent's (inside `/intake/` payloads and on `/api/v2/events`) and the events API
//! (`POST /api/v1/events`). Each becomes an OTLP log record with resource `oc.signal = dd.events`,
//! which the collector routes to `otel.otel_events`:
//!   Body = text, SeverityText = alert type (error / warning / info / success),
//!   LogAttributes = title, alert_type, priority, source_type_name, aggregation_key, event_type, tags,
//!   resource = host.name plus the Kubernetes / container tags under their OTel names.

use std::collections::BTreeMap;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::datadog::{inflate, TAG_ATTRS};
use crate::otlp;
use crate::{admit, send, KeyOrg, Rejection, Shared};

/// One event, whichever API it came from.
#[derive(Debug, Default, PartialEq)]
pub struct Event {
    pub title: String,
    pub text: String,
    /// Unix seconds.
    pub timestamp: i64,
    pub host: String,
    pub tags: Vec<String>,
    pub alert_type: String,
    pub priority: String,
    pub source_type_name: String,
    pub aggregation_key: String,
    pub event_type: String,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn tags(v: &Value) -> Vec<String> {
    match v.get("tags") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(t)) => t.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect(),
        _ => vec![],
    }
}

/// An event object in either the Agent's (`msg_title`, `msg_text`, `timestamp`) or the API's
/// (`title`, `text`, `date_happened`) spelling.
fn event(v: &Value, now: i64) -> Option<Event> {
    let title = [s(v, "msg_title"), s(v, "title")].into_iter().find(|t| !t.is_empty())?;
    let ts = ["timestamp", "date_happened"].iter().find_map(|k| v.get(*k).and_then(|t| t.as_i64().or_else(|| t.as_f64().map(|f| f as i64))));
    Some(Event {
        title,
        text: [s(v, "msg_text"), s(v, "text")].into_iter().find(|t| !t.is_empty()).unwrap_or_default(),
        timestamp: ts.filter(|t| *t > 0).unwrap_or(now),
        host: s(v, "host"),
        tags: tags(v),
        alert_type: Some(s(v, "alert_type")).filter(|a| !a.is_empty()).unwrap_or_else(|| "info".into()),
        priority: Some(s(v, "priority")).filter(|p| !p.is_empty()).unwrap_or_else(|| "normal".into()),
        source_type_name: s(v, "source_type_name"),
        aggregation_key: s(v, "aggregation_key"),
        event_type: s(v, "event_type"),
    })
}

/// Events in an Agent payload: `{"events": {"<source>": [ … ], …}}` (as sent to `/intake/` and
/// `/api/v2/events`).
pub fn agent_events(v: &Value, now: i64) -> Vec<Event> {
    let Some(groups) = v.get("events").and_then(Value::as_object) else {
        return vec![];
    };
    groups
        .iter()
        .flat_map(|(source, list)| {
            list.as_array().into_iter().flatten().filter_map(move |e| {
                let mut ev = event(e, now)?;
                if ev.source_type_name.is_empty() {
                    ev.source_type_name = source.clone();
                }
                Some(ev)
            })
        })
        .collect()
}

fn severity(alert_type: &str) -> i32 {
    match alert_type {
        "error" => 17,
        "warning" => 13,
        _ => 9,
    }
}

/// OTLP logs for the events, one resource per (host, promoted tags).
pub fn to_otlp(events: &[Event]) -> otlp::ExportLogsServiceRequest {
    let mut groups: BTreeMap<Vec<(String, String)>, Vec<otlp::LogRecord>> = BTreeMap::new();
    for e in events {
        let mut res: BTreeMap<String, String> = BTreeMap::new();
        if !e.host.is_empty() {
            res.insert("host.name".into(), e.host.clone());
        }
        for t in &e.tags {
            if let Some((k, v)) = t.split_once(':') {
                if let Some((_, otel)) = TAG_ATTRS.iter().find(|(dd, _)| *dd == k) {
                    res.entry((*otel).to_string()).or_insert_with(|| v.to_string());
                } else if k == "service" {
                    res.entry("service.name".into()).or_insert_with(|| v.to_string());
                }
            }
        }
        let t = (e.timestamp.max(0) as u64) * 1_000_000_000;
        let mut attributes = vec![otlp::kv("title", e.title.as_str()), otlp::kv("alert_type", e.alert_type.as_str()), otlp::kv("priority", e.priority.as_str())];
        for (k, v) in [("source_type_name", &e.source_type_name), ("aggregation_key", &e.aggregation_key), ("event_type", &e.event_type)] {
            if !v.is_empty() {
                attributes.push(otlp::kv(k, v.as_str()));
            }
        }
        if !e.tags.is_empty() {
            attributes.push(otlp::kv("tags", e.tags.join(",")));
        }
        let body = otlp::AnyValue { value: Some(otlp::any_value::Value::StringValue(e.text.clone())) };
        groups.entry(res.into_iter().collect()).or_default().push(otlp::LogRecord {
            time_unix_nano: t,
            observed_time_unix_nano: t,
            severity_number: severity(&e.alert_type),
            severity_text: e.alert_type.clone(),
            body: Some(body),
            attributes,
        });
    }
    otlp::ExportLogsServiceRequest {
        resource_logs: groups
            .into_iter()
            .map(|(res, log_records)| {
                let mut attributes: Vec<otlp::KeyValue> = res.into_iter().map(|(k, v)| otlp::kv(k, v)).collect();
                attributes.push(otlp::kv("oc.signal", "dd.events"));
                otlp::ResourceLogs { resource: Some(otlp::Resource { attributes }), scope_logs: vec![otlp::ScopeLogs { scope: otlp::scope("datadog.events"), log_records }] }
            })
            .collect(),
    }
}

/// Sends events to the collector.
pub async fn deliver(st: &Shared, org: &KeyOrg, events: &[Event]) -> Result<(), Rejection> {
    if events.is_empty() {
        return Ok(());
    }
    let target = crate::kafka::Sink::Logs;
    let up = send(st, org, target, Some(&HeaderValue::from_static("application/x-protobuf")), None, otlp::encode(&to_otlp(events))).await?;
    if up.status.is_success() {
        return Ok(());
    }
    tracing::warn!(status = %up.status, "collector refused converted Datadog events");
    Err(Rejection { status: StatusCode::SERVICE_UNAVAILABLE, message: "events were not accepted" })
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn parse(headers: &HeaderMap, body: &[u8]) -> Result<Value, String> {
    inflate(headers, body).and_then(|raw| serde_json::from_slice(&raw).map_err(|e| format!("invalid JSON payload: {e}")))
}

/// `POST /api/v1/events`: Datadog's events API (one event).
pub async fn post_v1(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let v = match parse(&headers, &body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "errors": [e] }))).into_response(),
    };
    let Some(ev) = event(&v, now()) else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "errors": ["Event title is required"] }))).into_response();
    };
    let echo = json!({ "title": ev.title, "text": ev.text, "date_happened": ev.timestamp, "priority": ev.priority, "alert_type": ev.alert_type, "tags": ev.tags, "host": ev.host });
    match deliver(&st, &org, &[ev]).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({ "status": "ok", "event": echo }))).into_response(),
        Err(r) => r.into_response(),
    }
}

/// `POST /api/v2/events` from the Agent: the same payload shape as the events inside `/intake/`.
pub async fn post_agent(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let v = match parse(&headers, &body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "errors": [e] }))).into_response(),
    };
    match deliver(&st, &org, &agent_events(&v, now())).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({}))).into_response(),
        Err(r) => r.into_response(),
    }
}

/// Events of an `/intake/` payload, if it has any.
pub fn intake_events(v: &Value) -> Vec<Event> {
    agent_events(v, now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_and_api_events() {
        let agent: Value =
            serde_json::from_str(r#"{"events":{"System":[{"msg_title":"Agent Startup","msg_text":"Version 7.84.6","timestamp":1700000000,"host":"web-1","event_type":"Agent Startup"}]}}"#).unwrap();
        let ev = agent_events(&agent, 1);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].title, "Agent Startup");
        assert_eq!(ev[0].source_type_name, "System");
        assert_eq!(ev[0].alert_type, "info");
        let api: Value = serde_json::from_str(r#"{"title":"Deploy order-service","text":"v1.4.2","alert_type":"success","tags":["env:prod","kube_namespace:shop"],"host":"web-2"}"#).unwrap();
        let e = event(&api, 99).unwrap();
        assert_eq!(e.timestamp, 99);
        assert!(event(&serde_json::json!({"text": "no title"}), 1).is_none());

        let req = to_otlp(&[e]);
        let rl = &req.resource_logs[0];
        let res = &rl.resource.as_ref().unwrap().attributes;
        assert!(res.contains(&otlp::kv("k8s.namespace.name", "shop")));
        assert!(res.contains(&otlp::kv("deployment.environment", "prod")));
        assert!(res.contains(&otlp::kv("oc.signal", "dd.events")));
        let r = &rl.scope_logs[0].log_records[0];
        assert_eq!(r.severity_text, "success");
        assert!(r.attributes.contains(&otlp::kv("title", "Deploy order-service")));
        assert_eq!(r.time_unix_nano, 99_000_000_000);
    }
}
