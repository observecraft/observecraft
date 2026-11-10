//! `POST /intake/webhook/<name>?api_key=…`: the alert webhooks Datadog's integrations documentation has vendors
//! post to (SolarWinds Orion trigger and reset actions, Prometheus Alertmanager's webhook receiver, the Zabbix
//! media type), turned into events (`otel.otel_events`, source `<name>`) like `POST /api/v1/events`. Our own mapping
//! of each vendor's payload; any other `<name>` becomes an event titled from the body's usual fields, its JSON as
//! the text.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::datadog::inflate;
use crate::datadog_events::{deliver, Event};
use crate::{admit, Shared};

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(t)) => t.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// The first of `keys` set in `v`.
fn first(v: &Value, keys: &[&str]) -> String {
    keys.iter().map(|k| s(v, k)).find(|t| !t.is_empty()).unwrap_or_default()
}

/// Unix seconds of an RFC 3339 time.
fn unix(t: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(t).ok().map(|d| d.timestamp()).filter(|t| *t > 0)
}

fn base(source: &str, now: i64) -> Event {
    Event { timestamp: now, alert_type: "info".into(), priority: "normal".into(), source_type_name: source.into(), ..Event::default() }
}

/// A SolarWinds Orion trigger or reset action's body (the README's template).
fn solarwinds(v: &Value, now: i64) -> Vec<Event> {
    let reset = s(v, "reset") == "true";
    let name = first(v, &["alert_name", "alert_message"]);
    let object = first(v, &["object", "host"]);
    let mut e = base("solarwinds", now);
    e.title = format!("[{}] {}{}", if reset { "Reset" } else { "Triggered" }, if name.is_empty() { "SolarWinds alert" } else { &name }, if object.is_empty() { String::new() } else { format!(" on {object}") });
    e.text = first(v, &["alert_message", "alert_description"]);
    e.host = s(v, "host");
    e.alert_type = if reset {
        "success".into()
    } else {
        match s(v, "alert_severity").to_lowercase().as_str() {
            "critical" | "serious" => "error".into(),
            "warning" => "warning".into(),
            _ => "info".into(),
        }
    };
    e.aggregation_key = [s(v, "alert_id"), object].join(":");
    e.tags = [("object_type", s(v, "object_type")), ("severity", s(v, "alert_severity").to_lowercase()), ("location", s(v, "location")), ("device_type", s(v, "device_type"))]
        .into_iter()
        .filter(|(_, v)| !v.is_empty() && !v.starts_with("${"))
        .map(|(k, v)| format!("{k}:{v}"))
        .collect();
    vec![e]
}

/// Prometheus Alertmanager's webhook body: one event per alert.
fn prometheus(v: &Value, now: i64) -> Vec<Event> {
    let alerts = v.get("alerts").and_then(Value::as_array).cloned().unwrap_or_default();
    alerts
        .iter()
        .map(|a| {
            let labels = a.get("labels").cloned().unwrap_or(Value::Null);
            let notes = a.get("annotations").cloned().unwrap_or(Value::Null);
            let resolved = s(a, "status") == "resolved";
            let mut e = base("prometheus", now);
            e.title = format!("[{}] {}", if resolved { "Resolved" } else { "Firing" }, first(&labels, &["alertname"]).chars().take(200).collect::<String>());
            e.text = first(&notes, &["description", "summary", "message"]);
            e.timestamp = unix(&s(a, if resolved { "endsAt" } else { "startsAt" })).unwrap_or(now);
            e.alert_type = if resolved {
                "success".into()
            } else {
                match s(&labels, "severity").to_lowercase().as_str() {
                    "warning" | "warn" => "warning".into(),
                    "info" | "none" => "info".into(),
                    _ => "error".into(),
                }
            };
            e.aggregation_key = first(a, &["fingerprint"]);
            e.tags = labels.as_object().into_iter().flatten().filter_map(|(k, v)| v.as_str().map(|v| format!("{k}:{v}"))).collect();
            e
        })
        .collect()
}

/// The Zabbix media type's parameters (`event_name`, `alert_subject`, `event_nseverity`, `event_value`, …).
fn zabbix(v: &Value, now: i64) -> Vec<Event> {
    let mut e = base("zabbix", now);
    e.title = first(v, &["alert_subject", "event_name", "item_name"]);
    if e.title.is_empty() {
        e.title = "Zabbix event".into();
    }
    e.text = first(v, &["alert_message", "event_name"]);
    let severity: i64 = s(v, "event_nseverity").parse().unwrap_or(0);
    e.alert_type = if s(v, "event_value") == "0" {
        "success".into()
    } else if severity >= 4 {
        "error".into()
    } else if severity >= 2 {
        "warning".into()
    } else {
        "info".into()
    };
    let tags: Value = match v.get("event_tags") {
        Some(Value::String(t)) => serde_json::from_str(t).unwrap_or(Value::Null),
        Some(other) => other.clone(),
        None => Value::Null,
    };
    e.tags = tags.as_array().into_iter().flatten().map(|t| format!("{}:{}", s(t, "tag"), s(t, "value"))).filter(|t| t.len() > 1).collect();
    e.aggregation_key = first(v, &["event_name"]);
    vec![e]
}

/// Any other webhook: titled from the body's usual fields, its JSON as the text.
fn generic(name: &str, v: &Value, now: i64) -> Vec<Event> {
    let mut e = base(name, now);
    e.title = first(v, &["title", "alert_name", "alertname", "subject", "summary", "name"]);
    if e.title.is_empty() {
        e.title = format!("{name} webhook");
    }
    e.text = serde_json::to_string(v).unwrap_or_default();
    e.host = first(v, &["host", "hostname"]);
    vec![e]
}

/// The events of a webhook body posted as `name`.
pub fn events(name: &str, v: &Value, now: i64) -> Vec<Event> {
    match name {
        "solarwinds" => solarwinds(v, now),
        "prometheus" => prometheus(v, now),
        "zabbix" => zabbix(v, now),
        _ => generic(name, v, now),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `POST /intake/webhook/{name}`.
pub async fn post(State(st): State<Shared>, Path(name): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let name: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')).take(64).collect::<String>().to_lowercase();
    let v: Value = match inflate(&headers, &body).and_then(|raw| serde_json::from_slice(&raw).map_err(|e| format!("invalid JSON payload: {e}"))) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "errors": [e] }))).into_response(),
    };
    let events = events(&name, &v, now());
    match deliver(&st, &org, &events).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({ "status": "ok" }))).into_response(),
        Err(r) => r.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solarwinds_trigger_and_reset() {
        let trigger = json!({"alert_name": "Node is down", "alert_severity": "Critical", "alert_message": "core-sw-01 stopped responding", "alert_id": "17", "host": "core-sw-01", "object": "core-sw-01", "object_type": "Node", "location": "${N=SwisEntity;M=Router.Nodes.CustomProperties.City}"});
        let e = &events("solarwinds", &trigger, 5)[0];
        assert_eq!((e.title.as_str(), e.text.as_str(), e.alert_type.as_str(), e.host.as_str()), ("[Triggered] Node is down on core-sw-01", "core-sw-01 stopped responding", "error", "core-sw-01"));
        assert_eq!((e.source_type_name.as_str(), e.aggregation_key.as_str(), e.timestamp), ("solarwinds", "17:core-sw-01", 5));
        assert_eq!(e.tags, ["object_type:Node", "severity:critical"], "unresolved template variables are left out");
        let mut reset = trigger.clone();
        reset["reset"] = json!("true");
        let r = &events("solarwinds", &reset, 5)[0];
        assert_eq!((r.title.as_str(), r.alert_type.as_str(), r.aggregation_key.as_str()), ("[Reset] Node is down on core-sw-01", "success", "17:core-sw-01"));
    }

    #[test]
    fn alertmanager_alerts_are_one_event_each() {
        let body = json!({"version": "4", "status": "firing", "alerts": [
            {"status": "firing", "labels": {"alertname": "HighErrorRate", "severity": "warning", "service": "checkout"}, "annotations": {"summary": "5xx above 5%"}, "startsAt": "2026-11-10T10:00:00Z", "fingerprint": "abc"},
            {"status": "resolved", "labels": {"alertname": "InstanceDown", "instance": "10.0.0.5:9100"}, "annotations": {"description": "back"}, "startsAt": "2026-11-10T09:00:00Z", "endsAt": "2026-11-10T09:30:00Z"}
        ]});
        let ev = events("prometheus", &body, 1);
        assert_eq!(ev.len(), 2);
        assert_eq!((ev[0].title.as_str(), ev[0].text.as_str(), ev[0].alert_type.as_str(), ev[0].timestamp), ("[Firing] HighErrorRate", "5xx above 5%", "warning", 1_794_304_800));
        assert!(ev[0].tags.contains(&"service:checkout".to_string()));
        assert_eq!((ev[1].title.as_str(), ev[1].alert_type.as_str(), ev[1].timestamp), ("[Resolved] InstanceDown", "success", 1_794_303_000));
    }

    #[test]
    fn zabbix_problems_and_recoveries() {
        let body = json!({"alert_subject": "Problem: High CPU on db-01", "alert_message": "CPU 97%", "event_nseverity": "4", "event_value": "1", "event_name": "High CPU", "event_tags": "[{\"tag\":\"scope\",\"value\":\"performance\"}]"});
        let e = &events("zabbix", &body, 1)[0];
        assert_eq!((e.title.as_str(), e.text.as_str(), e.alert_type.as_str()), ("Problem: High CPU on db-01", "CPU 97%", "error"));
        assert_eq!(e.tags, ["scope:performance"]);
        let mut ok = body.clone();
        ok["event_value"] = json!("0");
        assert_eq!(events("zabbix", &ok, 1)[0].alert_type, "success");
    }

    #[test]
    fn other_webhooks_keep_their_body() {
        let e = &events("acme-monitor", &json!({"subject": "Disk full", "host": "web-01", "pct": 99}), 1)[0];
        assert_eq!((e.title.as_str(), e.host.as_str(), e.source_type_name.as_str()), ("Disk full", "web-01", "acme-monitor"));
        assert_eq!(serde_json::from_str::<Value>(&e.text).unwrap()["pct"], 99);
        assert_eq!(events("acme-monitor", &json!({}), 1)[0].title, "acme-monitor webhook");
    }
}
